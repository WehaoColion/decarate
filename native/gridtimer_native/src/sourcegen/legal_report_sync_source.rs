// v2.23.1.1 - Generate Android's account-bound final-report transport from Rust source.
pub const PATH: &str = "com/ofairyo/gridtimer/data/LegalReportSync.kt";

pub const CONTENTS: &str = r####"package com.ofairyo.gridtimer.data

import android.content.Context
import android.util.Base64
import com.ofairyo.gridtimer.core.NativeOptimizerBridge
import org.json.JSONArray
import org.json.JSONObject
import java.security.MessageDigest

internal data class LegalReportSyncSummary(
    val pending: Int = 0,
    val failed: Boolean = false,
    val unsupported: Boolean = false,
    val hadReports: Boolean = false
) {
    val messageSuffix: String
        get() = when {
            unsupported && pending > 0 -> "；法律报告待同步，请更新电脑端服务"
            pending > 0 || failed -> "；法律报告待同步"
            hadReports -> "；法律报告已同步"
            else -> ""
        }
}

/** Account transport sends final reports only; local encrypted storage remains authoritative. */
internal object LegalReportSync {
    private const val CHUNK_BYTES = 2 * 1024 * 1024
    private const val MAX_REPORT_BYTES = 32 * 1024 * 1024

    fun sync(
        context: Context,
        workspaceKey: String,
        serverUrl: String,
        session: SyncAccountSession,
        isCurrent: () -> Boolean
    ): LegalReportSyncSummary {
        if (!session.loggedIn || session.workspaceId.isBlank() ||
            session.workspaceProof.isBlank() || session.acknowledgedGeneration < 0L ||
            !isCurrent()) return LegalReportSyncSummary(failed = true)
        return runCatching {
            val first = request(serverUrl, session, "manifest", JSONObject(), isCurrent)
            val hadReports = (first.optJSONArray("legalReports")?.length() ?: 0) > 0 ||
                LegalLocalStore.listReports(context, workspaceKey).isNotEmpty() ||
                LegalLocalStore.listTombstones(context, workspaceKey).isNotEmpty()
            reconcile(context, workspaceKey, serverUrl, session, first, isCurrent)
            val final = request(serverUrl, session, "manifest", JSONObject(), isCurrent)
            applyTombstones(context, workspaceKey, final.optJSONArray("legalTombstones") ?: JSONArray())
            LegalReportSyncSummary(pending = LegalLocalStore.pendingSyncCount(context, workspaceKey),
                hadReports = hadReports)
        }.getOrElse { error ->
            val pending = runCatching { LegalLocalStore.pendingSyncCount(context, workspaceKey) }
                .getOrDefault(0)
            LegalReportSyncSummary(
                pending = pending,
                failed = true,
                unsupported = error.message == "Endpoint not found."
            )
        }
    }

    private fun reconcile(
        context: Context,
        workspaceKey: String,
        serverUrl: String,
        session: SyncAccountSession,
        manifest: JSONObject,
        isCurrent: () -> Boolean
    ) {
        applyTombstones(context, workspaceKey, manifest.optJSONArray("legalTombstones") ?: JSONArray())
        val remote = manifest.optJSONArray("legalReports") ?: JSONArray()
        val remoteById = mutableMapOf<String, JSONObject>()
        for (index in 0 until remote.length()) {
            val item = remote.getJSONObject(index)
            val id = item.getString("reportId")
            require(remoteById.put(id, item) == null) { "Duplicate report ID in manifest." }
        }

        // Propagate local deletions before downloading or uploading. Server tombstones win.
        for (item in LegalLocalStore.listTombstones(context, workspaceKey).filter { !it.synced }) {
            val response = request(serverUrl, session, "delete",
                JSONObject().put("reportId", item.id), isCurrent)
            val confirmed = response.optJSONArray("legalTombstones") ?: JSONArray()
            require((0 until confirmed.length()).any { index ->
                confirmed.getJSONObject(index).optString("reportId") == item.id
            }) { "Report deletion was not acknowledged." }
            applyTombstones(context, workspaceKey, confirmed)
            LegalLocalStore.markTombstoneSynced(context, workspaceKey, item.id)
        }
        val deleted = LegalLocalStore.listTombstones(context, workspaceKey).map { it.id }.toSet()
        for ((id, descriptor) in remoteById) {
            if (id in deleted) continue
            require(isCurrent()) { "Account or workspace changed during report sync." }
            val expectedHash = descriptor.getString("sha256").lowercase()
            val expectedSize = descriptor.getLong("sizeBytes")
            val origin = descriptor.getString("sourceWorkspaceId")
            val createdAt = descriptor.getLong("createdAtEpochMillis")
            require(Regex("^[0-9a-f]{64}$").matches(expectedHash) &&
                expectedSize in 2..MAX_REPORT_BYTES.toLong()) { "Invalid report manifest." }
            val local = LegalLocalStore.listReports(context, workspaceKey).firstOrNull { it.id == id }
            if (local != null) {
                require(local.createdAtEpochMillis == createdAt &&
                    JSONObject(local.reportJson).optString("workspaceId") == origin) {
                    "Report metadata conflicts with the account copy."
                }
                require(sha256(local.reportJson.toByteArray(Charsets.UTF_8)) == expectedHash) {
                    "Report ID conflicts with the account copy."
                }
                LegalLocalStore.markReportSynced(context, workspaceKey, id, expectedHash)
                continue
            }
            val bytes = ByteArray(expectedSize.toInt())
            var offset = 0
            while (offset < bytes.size) {
                val response = request(serverUrl, session, "download", JSONObject()
                    .put("reportId", id)
                    .put("offsetBytes", offset)
                    .put("maxBytes", CHUNK_BYTES), isCurrent)
                require(response.getLong("legalReportTotalBytes") == expectedSize &&
                    response.getString("legalReportSha256").equals(expectedHash, ignoreCase = true) &&
                    response.getLong("legalReportCreatedAtEpochMillis") == createdAt &&
                    response.getString("legalReportSourceWorkspaceId") == origin) {
                    "Report changed during download."
                }
                val chunk = Base64.decode(response.getString("legalReportChunkBase64"), Base64.NO_WRAP)
                require(chunk.isNotEmpty() && chunk.size <= CHUNK_BYTES && offset + chunk.size <= bytes.size) {
                    "Invalid report chunk."
                }
                chunk.copyInto(bytes, offset)
                offset += chunk.size
            }
            require(sha256(bytes) == expectedHash) { "Downloaded report hash mismatch." }
            require(isCurrent()) { "Account or workspace changed during report sync." }
            LegalLocalStore.importReport(context, workspaceKey, id, createdAt, origin,
                bytes.toString(Charsets.UTF_8), expectedHash)
        }

        // Old Android reports have no acknowledgement marker and are migrated with their IDs.
        val pending = LegalLocalStore.pendingReports(context, workspaceKey)
            .sortedWith(compareBy<StoredLegalReport> { it.createdAtEpochMillis }.thenBy { it.id })
        for (report in pending) {
            require(isCurrent()) { "Account or workspace changed during report sync." }
            if (!LegalLocalStore.reportStillCurrent(context, workspaceKey, report)) continue
            val known = remoteById[report.id]
            if (known != null) {
                require(known.getString("sha256").equals(
                    sha256(report.reportJson.toByteArray(Charsets.UTF_8)), ignoreCase = true
                )) { "Report ID conflicts with the account copy." }
                LegalLocalStore.markReportSynced(context, workspaceKey, report.id, known.getString("sha256"))
                continue
            }
            val bytes = report.reportJson.toByteArray(Charsets.UTF_8)
            require(bytes.size in 2..MAX_REPORT_BYTES) { "Invalid report size." }
            val parsed = JSONObject(report.reportJson)
            val source = parsed.getString("workspaceId")
            require(parsed.getJSONObject("manifest").getString("workspaceId") == source &&
                parsed.getLong("capturedAtEpochMillis") ==
                    parsed.getJSONObject("manifest").getLong("capturedAtEpochMillis")) {
                "Report source identity mismatch."
            }
            val hash = sha256(bytes)
            var offset = 0
            while (offset < bytes.size) {
                if (!LegalLocalStore.reportStillCurrent(context, workspaceKey, report)) break
                val end = minOf(offset + CHUNK_BYTES, bytes.size)
                request(serverUrl, session, "upload", JSONObject()
                    .put("phase", "chunk")
                    .put("reportId", report.id)
                    .put("createdAtEpochMillis", report.createdAtEpochMillis)
                    .put("sha256", hash)
                    .put("totalBytes", bytes.size)
                    .put("sourceWorkspaceId", source)
                    .put("offsetBytes", offset)
                    .put("chunkBase64", Base64.encodeToString(bytes.copyOfRange(offset, end), Base64.NO_WRAP)),
                    isCurrent)
                offset = end
            }
            if (offset != bytes.size) continue
            if (!LegalLocalStore.commitIfCurrent(context, workspaceKey, report) {
                request(serverUrl, session, "upload", JSONObject()
                    .put("phase", "commit")
                    .put("reportId", report.id)
                    .put("createdAtEpochMillis", report.createdAtEpochMillis)
                    .put("sha256", hash)
                    .put("totalBytes", bytes.size)
                    .put("sourceWorkspaceId", source), isCurrent)
            }) continue
            LegalLocalStore.markReportSynced(context, workspaceKey, report.id, hash)
        }
    }

    private fun applyTombstones(context: Context, workspaceKey: String, items: JSONArray) {
        for (index in 0 until items.length()) {
            val item = items.getJSONObject(index)
            LegalLocalStore.applyRemoteTombstone(context, workspaceKey,
                item.getString("reportId"), item.getLong("deletedAtEpochMillis"))
        }
    }

    private fun request(
        serverUrl: String,
        session: SyncAccountSession,
        operation: String,
        details: JSONObject,
        isCurrent: () -> Boolean
    ): JSONObject {
        require(isCurrent()) { "Account or workspace changed during report sync." }
        val body = JSONObject()
            .put("acknowledgedGeneration", session.acknowledgedGeneration)
            .put("workspaceId", session.workspaceId)
            .put("workspaceProof", session.workspaceProof)
        val keys = details.keys()
        while (keys.hasNext()) {
            val key = keys.next()
            body.put(key, details.get(key))
        }
        val raw = NativeOptimizerBridge.legalReportsRequest(serverUrl, session.token, operation,
            body.toString()) ?: error("Report sync request unavailable.")
        require(isCurrent()) { "Account or workspace changed during report sync." }
        val result = JSONObject(raw)
        if (!result.optBoolean("ok")) error(result.optString("message").ifBlank { "Report sync failed." })
        val expectedMode = when (operation) {
            "manifest" -> "legal_manifest"
            "download" -> "legal_download"
            "delete" -> "legal_delete"
            "upload" -> if (details.optString("phase") == "chunk")
                "legal_upload_chunk" else "legal_upload_committed"
            else -> error("Unsupported report sync operation.")
        }
        require(result.optString("mode") == expectedMode) { "Report response operation mismatch." }
        require(result.getString("userId") == session.userId &&
            result.getString("tokenId") == session.tokenId &&
            result.getString("serverInstanceId") == session.serverInstanceId &&
            result.getString("accountNamespace") == session.accountNamespace &&
            result.optLong("currentGeneration", session.acknowledgedGeneration) >=
                session.acknowledgedGeneration) { "Report response identity or generation mismatch." }
        return result
    }

    private fun sha256(bytes: ByteArray): String = MessageDigest.getInstance("SHA-256")
        .digest(bytes).joinToString("") { "%02x".format(it.toInt() and 0xff) }
}
"####;

fn replace_once(source: String, before: &str, after: &str) -> Result<String, String> {
    if !source.contains(before) {
        return Err(format!("missing report sync source anchor: {before}"));
    }
    Ok(source.replacen(before, after, 1))
}

pub fn render(path: &str, source: &str) -> Result<String, String> {
    match path {
        "com/ofairyo/gridtimer/core/NativeOptimizerBridge.kt" => render_bridge(source),
        "com/ofairyo/gridtimer/data/TimerRepository.kt" => render_repository(source),
        _ => Ok(source.to_string()),
    }
}

fn render_bridge(source: &str) -> Result<String, String> {
    let source = replace_once(
        source.to_string(),
        "    fun syncAppData(\n",
        "    fun validateLegalReport(reportJson: String): Boolean =\n        nativeAvailable && runCatching { nativeValidateLegalReport(reportJson) }.getOrDefault(false)\n\n    fun legalReportsRequest(serverUrl: String, token: String, operation: String, requestJson: String): String? {\n        if (!nativeAvailable) return null\n        return runCatching { nativeLegalReportsRequest(serverUrl, token, operation, requestJson) }.getOrNull()\n    }\n\n    fun syncAppData(\n",
    )?;
    replace_once(
        source,
        "    private external fun nativeSyncAppData(\n",
        "    private external fun nativeValidateLegalReport(reportJson: String): Boolean\n\n    @JvmStatic\n    private external fun nativeLegalReportsRequest(\n        serverUrl: String,\n        token: String,\n        operation: String,\n        requestJson: String\n    ): String?\n\n    @JvmStatic\n    private external fun nativeSyncAppData(\n",
    )
}

fn render_repository(source: &str) -> Result<String, String> {
    let mut source = replace_once(
        source.to_string(),
        "    suspend fun syncNow() {\n",
        r####"    private suspend fun syncLegalReportsBestEffort(
        route: SyncNetworkRoute.Resolution,
        session: SyncAccountSession
    ) {
        val workspaceKey = activeWorkspaceKey
        if (workspaceKey != workspaceKeyForSession(session)) return
        val result = withContext(Dispatchers.IO) {
            runCatching {
                SyncNetworkRoute.withLanRouteIfNeeded(
                    context = appContext, serverUrl = route.serverUrl, network = route.network, log = ::logSyncNetworkRoute
                ) {
                    LegalReportSync.sync(appContext, workspaceKey, route.serverUrl, session) {
                        activeWorkspaceKey == workspaceKey && _syncSession.value.tokenId == session.tokenId &&
                            _syncSession.value.workspaceId == session.workspaceId &&
                            _syncSession.value.acknowledgedGeneration == session.acknowledgedGeneration
                    }
                }
            }.getOrElse { LegalReportSyncSummary(failed = true) }
        }
        if (activeWorkspaceKey == workspaceKey && _syncSession.value.tokenId == session.tokenId &&
            result.messageSuffix.isNotEmpty()) {
            val current = _syncSession.value
            updateSyncSession(current.copy(lastMessage = current.lastMessage + result.messageSuffix))
        }
    }

    suspend fun syncNow() {
"####,
    )?;
    // Each normal, forced-upload, and download path requires a durable account snapshot first.
    for (start, end) in [
        (
            "    private suspend fun syncNowLocked(\n",
            "    suspend fun uploadLocalDataToAccount() {\n",
        ),
        (
            "    private suspend fun uploadLocalDataToAccountLocked() {\n",
            "    suspend fun downloadAccountDataToPhone() {\n",
        ),
        (
            "    private suspend fun downloadAccountDataToPhoneLocked(\n",
            "    suspend fun flushNow(\n",
        ),
    ] {
        let left = source.find(start).ok_or("missing report sync path start")?;
        let right = source[left..]
            .find(end)
            .ok_or("missing report sync path end")?
            + left;
        let segment = &source[left..right];
        if !segment.contains("        var legalSnapshotDurable = false\n") {
            let mut changed = replace_once(
                segment.to_string(),
                "        var workspaceProofRejected = false\n",
                "        var workspaceProofRejected = false\n        var legalSnapshotDurable = false\n",
            )?;
            changed = replace_once(
                changed,
                "                accountRestoreApplied = result.restoreRequired && snapshotDurable\n",
                "                accountRestoreApplied = result.restoreRequired && snapshotDurable\n                legalSnapshotDurable = snapshotDurable && !result.restoreRequired\n",
            )?;
            changed = replace_once(
                changed,
                "        if (!updateTerminalSyncSession(finalSession)) {\n            return\n        }\n",
                "        if (!updateTerminalSyncSession(finalSession)) {\n            return\n        }\n        if (legalSnapshotDurable && finalSession.loggedIn &&\n            finalSession.workspaceProof.isNotBlank() && finalSession.restoreReceipt.isBlank()) {\n            syncLegalReportsBestEffort(route, finalSession)\n        }\n",
            )?;
            source.replace_range(left..right, &changed);
        }
    }
    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_is_after_terminal_snapshot_and_in_all_three_account_paths() {
        let repository = crate::kotlin_sources::SOURCES
            .iter()
            .find(|item| item.path.ends_with("TimerRepository.kt"))
            .unwrap();
        let rendered = render_repository(repository.contents).unwrap();
        assert_eq!(
            rendered
                .matches("syncLegalReportsBestEffort(route, finalSession)")
                .count(),
            3
        );
        assert_eq!(
            rendered
                .matches("legalSnapshotDurable = snapshotDurable && !result.restoreRequired")
                .count(),
            3
        );
        assert_eq!(
            rendered
                .matches("if (legalSnapshotDurable && finalSession.loggedIn &&")
                .count(),
            3
        );
    }

    #[test]
    fn imported_reports_cross_the_shared_source_validator() {
        let store = crate::legal_storage_source::CONTENTS;
        assert!(store.contains("fun importReport(context: Context"));
        assert!(store.contains("require(NativeOptimizerBridge.validateLegalReport(reportJson))"));
        assert!(CONTENTS.contains("LegalLocalStore.importReport(context, workspaceKey"));
        let bridge = crate::kotlin_sources::SOURCES
            .iter()
            .find(|item| item.path.ends_with("NativeOptimizerBridge.kt"))
            .unwrap();
        let bridge = render_bridge(bridge.contents).unwrap();
        assert!(bridge.contains("nativeValidateLegalReport(reportJson)"));
    }
}
