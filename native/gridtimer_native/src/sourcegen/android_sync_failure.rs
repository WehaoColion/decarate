// v2.23.2.5 - Label persisted transport errors as the last incomplete sync attempt.
// v2.23.2.3 - Surface payload-free transport failures without weakening account binding.
// Android production and JVM test code remain generated from this Rust source.

#[path = "android_agent_upgrade.rs"]
mod android_agent_upgrade;

const REPOSITORY_PATH: &str = "com/ofairyo/gridtimer/data/TimerRepository.kt";
const CONTRACT_PATH: &str = "com/ofairyo/gridtimer/data/WorkspaceIdentityRebindContract.kt";
pub const TEST_PATH: &str = "com/ofairyo/gridtimer/data/UnboundSyncFailureTest.kt";

const RESPONSE_ANCHOR: &str = r#"            val result = decodeSyncNetworkResult(resultJson)
            val boundResponseSession = result?.let { response ->"#;

const RESPONSE_REPLACEMENT: &str = r#"            val result = decodeSyncNetworkResult(resultJson)
            val unboundFailure = unboundPureSyncFailureSession(working, result)
            if (unboundFailure != null) {
                updateTerminalSyncSession(unboundFailure)
                return
            }
            val boundResponseSession = result?.let { response ->"#;

pub fn render(path: &str, source: &str) -> Result<String, String> {
    if path == android_agent_upgrade::SCREEN {
        return android_agent_upgrade::render(path, source);
    }
    match path {
        REPOSITORY_PATH => {
            let count = source.matches(RESPONSE_ANCHOR).count();
            if count != 3 {
                return Err(format!(
                    "expected three Android sync response boundaries, found {count}"
                ));
            }
            let rendered = source.replace(RESPONSE_ANCHOR, RESPONSE_REPLACEMENT);
            // Normal sync keeps its existing bounded retry scheduling. Upload and
            // download finish the explicit operation without creating a new request.
            Ok(rendered.replacen(
                "                updateTerminalSyncSession(unboundFailure)\n                return",
                "                if (updateTerminalSyncSession(unboundFailure)) {\n                    scheduleNormalSyncRetryAfterCooldown(remainingContinuations)\n                }\n                return",
                1,
            ))
        }
        CONTRACT_PATH => {
            if source.contains("fun unboundPureSyncFailureSession(") {
                return Err("Android pure sync failure helper already exists".to_string());
            }
            Ok(format!("{source}\n{HELPER_CONTENTS}"))
        }
        _ => Ok(source.to_owned()),
    }
}

pub const HELPER_CONTENTS: &str = r####"
// Native network failures and unauthenticated login rejections have no account
// identity. Only the empty/default response or the server's payload-free busy
// envelope may use this path. Both comparisons reject identity and data fields.
internal fun unboundPureSyncFailureSession(
    working: SyncAccountSession,
    result: SyncNetworkResult?
): SyncAccountSession? {
    if (result == null) return null
    val pure = result.copy(message = "") == SyncNetworkResult()
    val busy = result.mode == "busy" && result.copy(
        message = "", mode = "", syncProtocolVersion = 0, serverBuildId = ""
    ) == SyncNetworkResult()
    if (!pure && !busy) return null
    return working.copy(
        syncing = false,
        lastMessage = if (busy) "同步服务正忙，请稍后重试；本机数据已保留"
            else unboundPureSyncFailureMessage(result.message)
    )
}

private fun unboundPureSyncFailureMessage(raw: String): String {
    val message = raw.trim()
    return when {
        message == "Login expired." || message == "Login is invalid." ||
            message == "Login was revoked." || message == "Missing sync token." ->
            "登录状态已失效，请重新登录；本机数据已保留"
        message.startsWith("Could not connect to sync server:") ->
            "上次同步连接未完成，请稍后重试；本机数据已保留"
        message.startsWith("Could not send sync request:") ->
            "同步请求未发送完成，请稍后重试；本机数据已保留"
        message.startsWith("Could not read sync response:") ->
            "未完整收到同步服务响应，请稍后重试；本机数据已保留"
        message == "Invalid sync response." ||
            message == "Sync server returned an unreadable response." ||
            message.startsWith("Sync server returned HTTP ") ->
            "同步服务暂未返回可用结果，请稍后重试；本机数据已保留"
        else -> "本次同步未完成，请稍后重试；本机数据已保留"
    }
}
"####;

pub const TEST_CONTENTS: &str = r####"package com.ofairyo.gridtimer.data

import java.security.MessageDigest
import org.junit.Assert.*
import org.junit.Test

class UnboundSyncFailureTest {
    private fun working(): SyncAccountSession {
        val token = "fixture-bearer-only"
        val tokenId = MessageDigest.getInstance("SHA-256")
            .digest(token.toByteArray(Charsets.UTF_8))
            .joinToString("") { "%02x".format(it.toInt() and 0xff) }.take(32)
        val server = "a".repeat(64)
        val user = "fixture-user"
        val namespace = accountNamespaceIdentifier(server, user)
        return SyncAccountSession(
            serverUrl = "https://sync.example.test",
            lastResolvedServerUrl = "https://route.example.test",
            email = "fixture@example.test", userId = user, token = token, tokenId = tokenId,
            serverInstanceId = server, accountNamespace = namespace,
            workspaceId = "b".repeat(64), workspaceProof = "c".repeat(64),
            workspaceStorageKey = accountWorkspaceStorageKey(server, namespace, user),
            acknowledgedGeneration = 9L, restoreReceipt = "d".repeat(64),
            lastSyncAtEpochMillis = 1234L, lastAutomaticSyncAttemptAtEpochMillis = 5678L,
            aiApiKey = "fixture-ai-key", aiBaseUrl = "https://api.deepseek.com", aiModel = "deepseek-flash",
            syncing = true, lastMessage = "fixture-pending"
        )
    }

    @Test fun pureFailuresOnlyEndBusyStateAndReplaceStatus() {
        val original = working()
        for (message in listOf("Could not connect to sync server: fixture transport failure",
            "Could not send sync request: fixture transport failure",
            "Could not read sync response: fixture timeout", "Login expired.",
            "Login is invalid.", "Login was revoked.", "Missing sync token.",
            "Sync server returned an unreadable response.")) {
            val failed = unboundPureSyncFailureSession(original, SyncNetworkResult(message = message))!!
            assertFalse(failed.syncing)
            assertTrue(failed.lastMessage.isNotBlank())
            assertEquals(original.copy(syncing = false, lastMessage = failed.lastMessage), failed)
        }
    }

    @Test fun missingResponseAndSuccessfulResponseCannotUseFailurePath() {
        assertNull(unboundPureSyncFailureSession(working(), null))
        assertNull(unboundPureSyncFailureSession(working(), SyncNetworkResult(ok = true)))
    }

    @Test fun busyEnvelopeOnlyChangesStatusAndCannotCarryIdentityOrData() {
        val original = working()
        val busy = SyncNetworkResult(message = "Sync server is busy. Try again shortly.",
            mode = "busy", syncProtocolVersion = 1, serverBuildId = "1.1.0.6")
        val failed = unboundPureSyncFailureSession(original, busy)!!
        assertFalse(failed.syncing)
        assertTrue(failed.lastMessage.isNotBlank())
        assertEquals(original.copy(syncing = false, lastMessage = failed.lastMessage), failed)
        val secret = "fixture-sensitive-upstream-detail"
        assertFalse(unboundPureSyncFailureSession(original, busy.copy(message = secret))!!
            .lastMessage.contains(secret))
        for (response in listOf(
            busy.copy(ok = true), busy.copy(userId = "other-user"), busy.copy(token = "other-token"),
            busy.copy(tokenId = "e".repeat(32)), busy.copy(serverInstanceId = "f".repeat(64)),
            busy.copy(accountNamespace = "0".repeat(64)), busy.copy(appDataJson = "{}"),
            busy.copy(currentGeneration = 1L), busy.copy(currentGeneration = -1L),
            busy.copy(workspaceId = "b".repeat(64)), busy.copy(workspaceProof = "c".repeat(64)),
            busy.copy(currentCommitted = true), busy.copy(restoreRequired = true),
            busy.copy(baselineMergeRequired = true), busy.copy(workspaceIdentityRebound = true),
            busy.copy(restoreReceipt = "d".repeat(64)), busy.copy(retryable = true),
            busy.copy(publicServerUrl = "https://other.example.test"),
            busy.copy(resolvedServerUrl = "https://other.example.test"),
            busy.copy(mode = "upgrade_required")
        )) assertNull(unboundPureSyncFailureSession(original, response))
    }

    @Test fun anyPartialAccountIdentityRemainsSubjectToStrictBinding() {
        for (response in listOf(
            SyncNetworkResult(userId = "other-user"), SyncNetworkResult(token = "other-token"),
            SyncNetworkResult(tokenId = "e".repeat(32)), SyncNetworkResult(serverInstanceId = "f".repeat(64)),
            SyncNetworkResult(accountNamespace = "0".repeat(64))
        )) assertNull(unboundPureSyncFailureSession(working(), response))
    }

    @Test fun payloadProofGenerationAndRepairFlagsCannotBypassBinding() {
        for (response in listOf(
            SyncNetworkResult(appDataJson = ""), SyncNetworkResult(appDataJson = "{}"),
            SyncNetworkResult(currentGeneration = 1L), SyncNetworkResult(currentGeneration = -1L),
            SyncNetworkResult(serverUpdatedAtEpochMillis = 1L), SyncNetworkResult(clientUpdatedAtEpochMillis = 1L),
            SyncNetworkResult(currentCommitted = true), SyncNetworkResult(restoreRequired = true),
            SyncNetworkResult(baselineMergeRequired = true), SyncNetworkResult(workspaceIdentityRebound = true),
            SyncNetworkResult(restoreReceipt = "d".repeat(64)), SyncNetworkResult(workspaceId = "b".repeat(64)),
            SyncNetworkResult(workspaceProof = "c".repeat(64)), SyncNetworkResult(mode = "workspace_proof_invalid"),
            SyncNetworkResult(mode = "baseline_required"), SyncNetworkResult(retryable = true)
        )) assertNull(unboundPureSyncFailureSession(working(), response))
    }

    @Test fun routingQuotaAndProtocolMetadataCannotEnterEmptyFailurePath() {
        for (response in listOf(
            SyncNetworkResult(publicServerUrl = "https://other.example.test"),
            SyncNetworkResult(resolvedServerUrl = "https://other.example.test"),
            SyncNetworkResult(publicAccessMessage = "other"), SyncNetworkResult(syncProtocolVersion = 1),
            SyncNetworkResult(serverBuildId = "1.1.0.6"), SyncNetworkResult(snapshotHistoryUsageBytes = 1),
            SyncNetworkResult(snapshotHistoryProjectedBytes = 1), SyncNetworkResult(snapshotHistoryLimitBytes = 1),
            SyncNetworkResult(snapshotHistoryWarning = "other"), SyncNetworkResult(syncPayloadUsageBytes = 1),
            SyncNetworkResult(syncPayloadLimitBytes = 1), SyncNetworkResult(syncPayloadWarning = "other"),
            SyncNetworkResult(appDataCurrentBytes = 1), SyncNetworkResult(appDataProjectedBytes = 1),
            SyncNetworkResult(appDataLimitBytes = 1)
        )) assertNull(unboundPureSyncFailureSession(working(), response))
    }

    @Test fun upstreamDetailNeverAppearsInStatusOrChangesSavedSecrets() {
        val original = working()
        for (prefix in listOf("", "Could not connect to sync server: ", "Could not read sync response: ",
            "Sync server returned HTTP 502. ")) {
            val secret = "fixture-secret-that-must-not-be-shown"
            val failed = unboundPureSyncFailureSession(original, SyncNetworkResult(message = prefix + secret))!!
            assertFalse(failed.lastMessage.contains(secret))
            assertEquals(original.token, failed.token)
            assertEquals(original.aiApiKey, failed.aiApiKey)
        }
    }

    @Test fun successfulSnapshotWithoutOwnerStillFailsTheOriginalContract() {
        val session = working()
        val response = SyncNetworkResult(ok = true, appDataJson = "{}", currentCommitted = true)
        assertNull(unboundPureSyncFailureSession(session, response))
        assertThrows(IllegalStateException::class.java) {
            WorkspaceIdentityRebindContract.bindAuthenticatedResponse(
                session, response, session.workspaceStorageKey,
                allowNamespaceBootstrap = false, allowWorkspaceIdentityRebind = false
            )
        }
    }

    @Test fun foreignOwnerAndBearerCannotBeAcceptedAsOrdinaryErrors() {
        val session = working()
        val validIdentity = SyncNetworkResult(userId = session.userId, tokenId = session.tokenId,
            serverInstanceId = session.serverInstanceId, accountNamespace = session.accountNamespace)
        for (response in listOf(validIdentity.copy(userId = "other-user"),
            validIdentity.copy(tokenId = "0".repeat(32)), validIdentity.copy(token = "other-token"))) {
            assertNull(unboundPureSyncFailureSession(session, response))
            assertThrows(IllegalStateException::class.java) {
                WorkspaceIdentityRebindContract.bindAuthenticatedResponse(
                    session, response, session.workspaceStorageKey,
                    allowNamespaceBootstrap = false, allowWorkspaceIdentityRebind = false
                )
            }
        }
    }
}
"####;
