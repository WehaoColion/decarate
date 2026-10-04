// v2.23.1.1 - Preserve report identities and deletion receipts for account synchronization.
// v2.23 - Keep Android legal reports and finance review receipts private to each workspace.
pub const PATH: &str = "com/ofairyo/gridtimer/data/LegalLocalStore.kt";

pub const CONTENTS: &str = r####"package com.ofairyo.gridtimer.data

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import com.ofairyo.gridtimer.core.NativeOptimizerBridge
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.security.KeyStore
import java.security.MessageDigest
import java.util.UUID
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** Report metadata and a final report, never the scan's source documents or upload batches. */
data class StoredLegalReport(
    val id: String,
    val createdAtEpochMillis: Long,
    val reportJson: String
)

data class StoredLegalTombstone(val id: String, val deletedAtEpochMillis: Long, val synced: Boolean)

/**
 * Local, workspace-bound encrypted files. Android Auto Backup deliberately does not include
 * this directory, since an Android Keystore key cannot be restored on another device.
 */
private object PrivateAnalysisStore {
    private const val ROOT_NAME = "private_analysis_v1"
    private const val KEY_ALIAS = "grid_timer_private_analysis_v1"
    private const val TRANSFORMATION = "AES/GCM/NoPadding"
    private const val FORMAT_VERSION = 1
    private val safeWorkspaceKey = Regex("^[A-Za-z0-9_-]{1,128}$")

    fun read(context: Context, workspaceKey: String, purpose: String): JSONObject? {
        val target = targetFile(context, workspaceKey, purpose)
        if (!target.exists()) return null
        check(target.isFile) { "Private analysis storage is not a regular file." }
        val envelope = JSONObject(target.readText(Charsets.UTF_8))
        check(envelope.getInt("formatVersion") == FORMAT_VERSION) {
            "Unsupported private analysis storage version."
        }
        val iv = Base64.decode(envelope.getString("ivBase64"), Base64.NO_WRAP)
        check(iv.size == 12) { "Invalid private analysis nonce." }
        val ciphertext = Base64.decode(envelope.getString("ciphertextBase64"), Base64.NO_WRAP)
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.DECRYPT_MODE, secretKey(), GCMParameterSpec(128, iv))
        cipher.updateAAD(aad(workspaceKey, purpose))
        val payload = JSONObject(cipher.doFinal(ciphertext).toString(Charsets.UTF_8))
        check(payload.getInt("formatVersion") == FORMAT_VERSION &&
            payload.getString("workspaceKey") == workspaceKey) {
            "Private analysis storage belongs to another workspace."
        }
        return payload
    }

    fun write(context: Context, workspaceKey: String, purpose: String, payload: JSONObject) {
        check(payload.getInt("formatVersion") == FORMAT_VERSION &&
            payload.getString("workspaceKey") == workspaceKey) {
            "Private analysis storage identity mismatch."
        }
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, secretKey())
        cipher.updateAAD(aad(workspaceKey, purpose))
        val ciphertext = cipher.doFinal(payload.toString().toByteArray(Charsets.UTF_8))
        val envelope = JSONObject()
            .put("formatVersion", FORMAT_VERSION)
            .put("ivBase64", Base64.encodeToString(cipher.iv, Base64.NO_WRAP))
            .put("ciphertextBase64", Base64.encodeToString(ciphertext, Base64.NO_WRAP))
        val target = targetFile(context, workspaceKey, purpose)
        AtomicFileStore.writeText(target, envelope.toString())
        // Surface disk or key failures immediately; never silently replace an unreadable file.
        check(read(context, workspaceKey, purpose) != null) {
            "Private analysis storage verification failed."
        }
    }

    fun empty(workspaceKey: String, arrayName: String): JSONObject = JSONObject()
        .put("formatVersion", FORMAT_VERSION)
        .put("workspaceKey", workspaceKey)
        .put(arrayName, JSONArray())

    private fun targetFile(context: Context, workspaceKey: String, purpose: String): File {
        require(safeWorkspaceKey.matches(workspaceKey)) { "Invalid workspace key." }
        require(purpose == "legal_reports" || purpose == "finance_review") {
            "Invalid private analysis storage purpose."
        }
        val digest = MessageDigest.getInstance("SHA-256")
            .digest(workspaceKey.toByteArray(Charsets.UTF_8))
            .joinToString("") { "%02x".format(it.toInt() and 0xff) }
        return File(File(File(context.filesDir, ROOT_NAME), digest), "$purpose.secure")
    }

    private fun aad(workspaceKey: String, purpose: String): ByteArray =
        "com.ofairyo.gridtimer.private_analysis.v1\u0000$purpose\u0000$workspaceKey"
            .toByteArray(Charsets.UTF_8)

    @Synchronized
    private fun secretKey(): SecretKey {
        val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (keyStore.getKey(KEY_ALIAS, null) as? SecretKey)?.let { return it }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").run {
            init(
                KeyGenParameterSpec.Builder(
                    KEY_ALIAS,
                    KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT
                )
                    .setKeySize(256)
                    .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                    .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                    .setRandomizedEncryptionRequired(true)
                    .build()
            )
            generateKey()
        }
    }
}

/** Encrypted local reports and durable delete markers. Only final reports may be synced. */
object LegalLocalStore {
    private const val PURPOSE = "legal_reports"
    private const val MAX_REPORTS = 5
    private const val MAX_REPORT_BYTES = 32 * 1024 * 1024

    private val reportIdPattern = Regex("^[A-Za-z0-9_-]{1,128}$")

    private fun validFinalReport(reportJson: String, sourceWorkspaceId: String): JSONObject {
        val size = reportJson.toByteArray(Charsets.UTF_8).size
        require(size in 2..MAX_REPORT_BYTES) { "Legal report size is invalid." }
        val finalReport = JSONObject(reportJson)
        val manifest = finalReport.optJSONObject("manifest")
        require(sourceWorkspaceId.isNotBlank() &&
            finalReport.optString("workspaceId") == sourceWorkspaceId &&
            manifest?.optString("workspaceId") == sourceWorkspaceId &&
            finalReport.has("capturedAtEpochMillis") &&
            manifest?.optLong("capturedAtEpochMillis") == finalReport.optLong("capturedAtEpochMillis") &&
            finalReport.has("completed") &&
            finalReport.optJSONArray("findings") != null &&
            finalReport.optJSONArray("errors") != null &&
            !finalReport.has("records") &&
            !finalReport.has("uploadBatches") &&
            !finalReport.has("sourceDocuments")) {
            "Only a final legal report with matching source identity can be saved."
        }
        return finalReport
    }

    private fun reports(root: JSONObject): JSONArray = root.optJSONArray("reports") ?: JSONArray()
    private fun tombstones(root: JSONObject): JSONArray = root.optJSONArray("tombstones") ?: JSONArray()
    private fun storedReportJson(record: JSONObject): String =
        if (record.has("reportJson")) record.getString("reportJson")
        else record.getJSONObject("report").toString()

    private fun appendTombstone(root: JSONObject, reportId: String, deletedAt: Long, synced: Boolean) {
        val existing = tombstones(root)
        val next = JSONArray()
        var matched = false
        for (index in 0 until existing.length()) {
            val item = existing.getJSONObject(index)
            if (item.optString("id") == reportId) {
                val previousAt = item.optLong("deletedAtEpochMillis")
                next.put(JSONObject().put("id", reportId)
                    .put("deletedAtEpochMillis", maxOf(previousAt, deletedAt))
                    .put("synced", item.optBoolean("synced") || synced))
                matched = true
            } else next.put(item)
        }
        if (!matched) next.put(JSONObject().put("id", reportId)
            .put("deletedAtEpochMillis", deletedAt).put("synced", synced))
        root.put("tombstones", next)
    }

    private fun retainFive(root: JSONObject, records: List<JSONObject>) {
        val ordered = records.sortedWith(compareByDescending<JSONObject> { it.optLong("createdAtEpochMillis") }
            .thenByDescending { it.optString("id") })
        val kept = JSONArray()
        ordered.take(MAX_REPORTS).forEach { kept.put(it) }
        ordered.drop(MAX_REPORTS).forEach { item ->
            appendTombstone(root, item.getString("id"), System.currentTimeMillis(), false)
        }
        root.put("reports", kept)
    }

    @Synchronized
    fun saveReport(context: Context, workspaceKey: String, reportJson: String): String {
        val finalReport = validFinalReport(reportJson, workspaceKey)
        val id = UUID.randomUUID().toString()
        val root = PrivateAnalysisStore.read(context, workspaceKey, PURPOSE)
            ?: PrivateAnalysisStore.empty(workspaceKey, "reports")
        val prior = reports(root)
        val records = mutableListOf(JSONObject().put("id", id)
            .put("createdAtEpochMillis", maxOf(System.currentTimeMillis(),
                finalReport.optLong("capturedAtEpochMillis"))).put("reportJson", reportJson))
        for (index in 0 until prior.length()) records += prior.getJSONObject(index)
        retainFive(root, records)
        PrivateAnalysisStore.write(context, workspaceKey, PURPOSE, root)
        return id
    }

    /** Preserve the server's ID and creation time; source workspace need not match this device's key. */
    @Synchronized
    fun importReport(context: Context, workspaceKey: String, reportId: String,
        createdAtEpochMillis: Long, sourceWorkspaceId: String, reportJson: String, sha256: String): Boolean {
        require(reportIdPattern.matches(reportId) && createdAtEpochMillis > 0L)
        val finalReport = validFinalReport(reportJson, sourceWorkspaceId)
        require(NativeOptimizerBridge.validateLegalReport(reportJson)) {
            "Legal report contains invalid evidence or legal references."
        }
        require(createdAtEpochMillis >= finalReport.getLong("capturedAtEpochMillis")) {
            "Report creation time precedes its scan."
        }
        val actualSha = MessageDigest.getInstance("SHA-256")
            .digest(reportJson.toByteArray(Charsets.UTF_8))
            .joinToString("") { "%02x".format(it.toInt() and 0xff) }
        require(actualSha.equals(sha256, ignoreCase = true)) { "Report hash mismatch." }
        val root = PrivateAnalysisStore.read(context, workspaceKey, PURPOSE)
            ?: PrivateAnalysisStore.empty(workspaceKey, "reports")
        if ((0 until tombstones(root).length()).any { tombstones(root).getJSONObject(it).optString("id") == reportId }) {
            return false
        }
        val prior = reports(root)
        val records = (0 until prior.length()).map { prior.getJSONObject(it) }.toMutableList()
        val existing = records.firstOrNull { it.optString("id") == reportId }
        if (existing != null) {
            require(storedReportJson(existing) == reportJson) {
                "Report ID has different content."
            }
            existing.remove("report")
            existing.put("reportJson", reportJson)
            existing.put("syncedSha256", actualSha)
        } else {
            records += JSONObject().put("id", reportId)
                .put("createdAtEpochMillis", createdAtEpochMillis)
                .put("reportJson", reportJson).put("syncedSha256", actualSha)
        }
        retainFive(root, records)
        PrivateAnalysisStore.write(context, workspaceKey, PURPOSE, root)
        return true
    }

    @Synchronized
    fun markReportSynced(context: Context, workspaceKey: String, reportId: String, sha256: String) {
        val root = PrivateAnalysisStore.read(context, workspaceKey, PURPOSE) ?: return
        val records = reports(root)
        for (index in 0 until records.length()) {
            val item = records.getJSONObject(index)
            if (item.optString("id") == reportId) {
                val raw = storedReportJson(item)
                val bytes = raw.toByteArray(Charsets.UTF_8)
                val actualSha = MessageDigest.getInstance("SHA-256").digest(bytes)
                    .joinToString("") { "%02x".format(it.toInt() and 0xff) }
                require(actualSha.equals(sha256, ignoreCase = true)) { "Report changed during sync." }
                item.remove("report")
                item.put("reportJson", raw)
                item.put("syncedSha256", actualSha)
                PrivateAnalysisStore.write(context, workspaceKey, PURPOSE, root)
                return
            }
        }
    }

    @Synchronized
    fun pendingReports(context: Context, workspaceKey: String): List<StoredLegalReport> {
        val root = PrivateAnalysisStore.read(context, workspaceKey, PURPOSE) ?: return emptyList()
        val records = reports(root)
        return (0 until records.length()).mapNotNull { index ->
            val item = records.getJSONObject(index)
            val raw = storedReportJson(item)
            val hash = MessageDigest.getInstance("SHA-256").digest(raw.toByteArray(Charsets.UTF_8))
                .joinToString("") { "%02x".format(it.toInt() and 0xff) }
            if (item.optString("syncedSha256").equals(hash, ignoreCase = true)) null
            else StoredLegalReport(item.getString("id"), item.getLong("createdAtEpochMillis"), raw)
        }
    }

    /** Re-read the encrypted record before each transfer step; a deletion invalidates cached bytes. */
    @Synchronized
    fun reportStillCurrent(context: Context, workspaceKey: String, report: StoredLegalReport): Boolean {
        val root = PrivateAnalysisStore.read(context, workspaceKey, PURPOSE) ?: return false
        if ((0 until tombstones(root).length()).any {
            tombstones(root).getJSONObject(it).optString("id") == report.id
        }) return false
        val records = reports(root)
        return (0 until records.length()).any { index ->
            val current = records.getJSONObject(index)
            current.optString("id") == report.id &&
                current.optLong("createdAtEpochMillis") == report.createdAtEpochMillis &&
                storedReportJson(current) == report.reportJson
        }
    }

    /** Serialize the final commit with local deletion so a deleted report cannot be committed later. */
    @Synchronized
    fun commitIfCurrent(context: Context, workspaceKey: String, report: StoredLegalReport,
        commit: () -> Unit): Boolean {
        if (!reportStillCurrent(context, workspaceKey, report)) return false
        commit()
        return true
    }

    @Synchronized
    fun listTombstones(context: Context, workspaceKey: String): List<StoredLegalTombstone> {
        val root = PrivateAnalysisStore.read(context, workspaceKey, PURPOSE) ?: return emptyList()
        val items = tombstones(root)
        return (0 until items.length()).map { index ->
            val item = items.getJSONObject(index)
            StoredLegalTombstone(item.getString("id"), item.getLong("deletedAtEpochMillis"),
                item.optBoolean("synced"))
        }
    }

    @Synchronized
    fun applyRemoteTombstone(context: Context, workspaceKey: String, reportId: String, deletedAt: Long) {
        require(reportIdPattern.matches(reportId) && deletedAt > 0L)
        val root = PrivateAnalysisStore.read(context, workspaceKey, PURPOSE)
            ?: PrivateAnalysisStore.empty(workspaceKey, "reports")
        appendTombstone(root, reportId, deletedAt, true)
        val prior = reports(root)
        val remaining = JSONArray()
        for (index in 0 until prior.length()) {
            val item = prior.getJSONObject(index)
            if (item.optString("id") != reportId) remaining.put(item)
        }
        root.put("reports", remaining)
        PrivateAnalysisStore.write(context, workspaceKey, PURPOSE, root)
    }

    @Synchronized
    fun markTombstoneSynced(context: Context, workspaceKey: String, reportId: String) {
        val root = PrivateAnalysisStore.read(context, workspaceKey, PURPOSE) ?: return
        val items = tombstones(root)
        for (index in 0 until items.length()) {
            val item = items.getJSONObject(index)
            if (item.optString("id") == reportId) {
                item.put("synced", true)
                PrivateAnalysisStore.write(context, workspaceKey, PURPOSE, root)
                return
            }
        }
    }

    @Synchronized
    fun pendingSyncCount(context: Context, workspaceKey: String): Int =
        pendingReports(context, workspaceKey).size + listTombstones(context, workspaceKey).count { !it.synced }

    @Synchronized
    fun listReports(context: Context, workspaceKey: String): List<StoredLegalReport> {
        val root = PrivateAnalysisStore.read(context, workspaceKey, PURPOSE) ?: return emptyList()
        val reports = reports(root)
        return (0 until reports.length()).map { index ->
            val record = reports.getJSONObject(index)
            StoredLegalReport(
                id = record.getString("id"),
                createdAtEpochMillis = record.getLong("createdAtEpochMillis"),
                reportJson = storedReportJson(record)
            )
        }
    }

    @Synchronized
    fun deleteReport(context: Context, workspaceKey: String, reportId: String): Boolean {
        val root = PrivateAnalysisStore.read(context, workspaceKey, PURPOSE) ?: return false
        val reports = reports(root)
        val retained = JSONArray()
        var removed = false
        for (index in 0 until reports.length()) {
            val record = reports.getJSONObject(index)
            if (record.getString("id") == reportId) removed = true else retained.put(record)
        }
        if (removed) {
            root.put("reports", retained)
            appendTombstone(root, reportId, System.currentTimeMillis(), false)
            PrivateAnalysisStore.write(context, workspaceKey, PURPOSE, root)
        }
        return removed
    }
}

/** A receipt is valid only while its fingerprint matches Rust's current source-data digest. */
object FinanceReviewStore {
    private const val PURPOSE = "finance_review"
    private val monthKeyPattern = Regex("^[0-9]{4}-(0[1-9]|1[0-2])$")
    private val categories = setOf("incomeExpense", "cash", "assets", "liabilities")

    @Synchronized
    fun loadReceipts(context: Context, workspaceKey: String): String =
        (PrivateAnalysisStore.read(context, workspaceKey, PURPOSE)
            ?: PrivateAnalysisStore.empty(workspaceKey, "receipts"))
            .getJSONArray("receipts").toString()

    /** Returns the complete receipt array for the Rust V2 risk calculation. */
    @Synchronized
    fun confirm(
        context: Context,
        workspaceKey: String,
        monthKey: String,
        category: String,
        dataFingerprint: String,
        zeroConfirmed: Boolean = false
    ): String {
        require(monthKeyPattern.matches(monthKey)) { "Invalid finance month." }
        require(category in categories) { "Invalid finance review category." }
        require(dataFingerprint.isNotBlank() && dataFingerprint.length <= 128) {
            "Invalid finance source fingerprint."
        }
        val root = PrivateAnalysisStore.read(context, workspaceKey, PURPOSE)
            ?: PrivateAnalysisStore.empty(workspaceKey, "receipts")
        val receipts = root.getJSONArray("receipts")
        val updated = JSONArray()
        var found = false
        for (index in 0 until receipts.length()) {
            val receipt = receipts.getJSONObject(index)
            if (receipt.optString("monthKey") == monthKey) {
                updateCategory(receipt, category, dataFingerprint, zeroConfirmed)
                found = true
            }
            updated.put(receipt)
        }
        if (!found) {
            val receipt = newReceipt(monthKey)
            updateCategory(receipt, category, dataFingerprint, zeroConfirmed)
            updated.put(receipt)
        }
        root.put("receipts", updated)
        PrivateAnalysisStore.write(context, workspaceKey, PURPOSE, root)
        return updated.toString()
    }

    @Synchronized
    fun setConfirmedRecurringKeys(
        context: Context,
        workspaceKey: String,
        monthKey: String,
        keys: List<String>
    ): String {
        require(monthKeyPattern.matches(monthKey)) { "Invalid finance month." }
        require(keys.size <= 512 && keys.all { it.isNotBlank() && it.length <= 256 }) {
            "Invalid recurring item keys."
        }
        val root = PrivateAnalysisStore.read(context, workspaceKey, PURPOSE)
            ?: PrivateAnalysisStore.empty(workspaceKey, "receipts")
        val receipts = root.getJSONArray("receipts")
        val updated = JSONArray()
        var found = false
        for (index in 0 until receipts.length()) {
            val receipt = receipts.getJSONObject(index)
            if (receipt.optString("monthKey") == monthKey) {
                receipt.put("confirmedRecurringKeys", JSONArray(keys.distinct()))
                found = true
            }
            updated.put(receipt)
        }
        if (!found) {
            updated.put(newReceipt(monthKey).put("confirmedRecurringKeys", JSONArray(keys.distinct())))
        }
        root.put("receipts", updated)
        PrivateAnalysisStore.write(context, workspaceKey, PURPOSE, root)
        return updated.toString()
    }

    @Synchronized
    fun clearMonth(context: Context, workspaceKey: String, monthKey: String): String {
        require(monthKeyPattern.matches(monthKey)) { "Invalid finance month." }
        val root = PrivateAnalysisStore.read(context, workspaceKey, PURPOSE)
            ?: return JSONArray().toString()
        val receipts = root.getJSONArray("receipts")
        val retained = JSONArray()
        for (index in 0 until receipts.length()) {
            val receipt = receipts.getJSONObject(index)
            if (receipt.optString("monthKey") != monthKey) retained.put(receipt)
        }
        if (retained.length() != receipts.length()) {
            root.put("receipts", retained)
            PrivateAnalysisStore.write(context, workspaceKey, PURPOSE, root)
        }
        return retained.toString()
    }

    private fun updateCategory(
        receipt: JSONObject,
        category: String,
        fingerprint: String,
        zeroConfirmed: Boolean
    ) {
        if (category == "incomeExpense" &&
            receipt.optString("incomeExpenseFingerprint") != fingerprint) {
            receipt.put("confirmedRecurringKeys", JSONArray())
        }
        receipt.put("${category}Fingerprint", fingerprint)
        receipt.put("${category}Verified", true)
        when (category) {
            "cash" -> receipt.put("zeroCashConfirmed", zeroConfirmed)
            "assets" -> receipt.put("zeroAssetsConfirmed", zeroConfirmed)
            "liabilities" -> receipt.put("zeroLiabilitiesConfirmed", zeroConfirmed)
        }
    }

    private fun newReceipt(monthKey: String): JSONObject = JSONObject()
        .put("monthKey", monthKey)
        .put("incomeExpenseFingerprint", "")
        .put("incomeExpenseVerified", false)
        .put("cashFingerprint", "")
        .put("cashVerified", false)
        .put("assetsFingerprint", "")
        .put("assetsVerified", false)
        .put("liabilitiesFingerprint", "")
        .put("liabilitiesVerified", false)
        .put("zeroCashConfirmed", false)
        .put("zeroAssetsConfirmed", false)
        .put("zeroLiabilitiesConfirmed", false)
        .put("confirmedRecurringKeys", JSONArray())
}
"####;
