// v2.22.21 - Persist signed rendezvous rollback protection before credential routing.
pub const PATH: &str = "com/ofairyo/gridtimer/data/SyncRemoteRendezvous.kt";

pub const CONTENTS: &str = r####"package com.ofairyo.gridtimer.data

import android.content.Context
import android.util.AtomicFile
import com.ofairyo.gridtimer.core.NativeOptimizerBridge
import kotlinx.serialization.ExperimentalSerializationApi
import kotlinx.serialization.Serializable
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.FileNotFoundException
import java.io.FileOutputStream
import java.net.HttpURLConnection
import java.net.URI
import java.net.URL
import java.security.MessageDigest

internal data class SignedSyncRendezvousCandidate(
    val serverUrl: String,
    val serverBuildId: String,
    val generation: Long,
    val issuedAt: Long,
    val signedPayloadFingerprint: String
)

internal object SyncRemoteRendezvous {
    @Serializable
    private data class NtfyEvent(
        val event: String = "",
        val time: Long = 0L,
        val message: String = ""
    )

    @Serializable
    private data class SignedEnvelope(
        val schemaVersion: Int = 0,
        val payload: String = "",
        val publicKey: String = "",
        val signature: String = ""
    )

    @Serializable
    private data class SignedPayload(
        val schema: String = "",
        val syncProtocolVersion: Int = 0,
        val serverBuildId: String = "",
        val identityId: String = "",
        val serverUrl: String = "",
        val generation: Long = 0L,
        val issuedAt: Long = 0L,
        val expiresAt: Long = 0L
    )

    private data class HighWaterState(
        val present: Boolean,
        val valid: Boolean,
        val generation: Long = 0L,
        val signedPayloadFingerprint: String = ""
    )

    private const val SERVICE_BASE_URL = "https://ntfy.sh"
    private const val TOPIC = "__GRIDTIMER_RENDEZVOUS_TOPIC__"
    private const val PINNED_PUBLIC_KEY = "__GRIDTIMER_RENDEZVOUS_PUBLIC_KEY_BASE64__"
    private const val PAYLOAD_SCHEMA = "gridtimer.sync.rendezvous.v1"
    private const val ENVELOPE_SCHEMA_VERSION = 1
    private const val SYNC_PROTOCOL_VERSION = __GRIDTIMER_RENDEZVOUS_SYNC_PROTOCOL_VERSION__
    private const val MAX_RESPONSE_BYTES = 256 * 1024
    private const val MAX_ENVELOPE_CHARS = 16 * 1024
    private const val MAX_SIGNED_PAYLOAD_BYTES = 8 * 1024
    private const val MAX_HIGH_WATER_BYTES = 512
    private const val MAX_LIFETIME_SECONDS = 12 * 60 * 60L
    private const val CLOCK_SKEW_SECONDS = 5 * 60L
    private const val HIGH_WATER_SCHEMA = "gridtimer.sync.rendezvous.high-water.v1"
    private const val HIGH_WATER_FILE_PREFIX = "sync-rendezvous-high-water-v1-"
    private val topicRegex = Regex("^[0-9a-f]{64}$")
    private val publicKeyRegex = Regex("^[A-Za-z0-9_-]{43}$")
    private val signatureRegex = Regex("^[A-Za-z0-9_-]{86}$")
    private val fingerprintRegex = Regex("^[0-9a-f]{64}$")
    private val buildIdRegex = Regex("^[A-Za-z0-9._+\\-]{1,128}$")
    private val highWaterLock = Any()
    private val ntfyJson = Json {
        ignoreUnknownKeys = true
        coerceInputValues = false
    }
    @OptIn(ExperimentalSerializationApi::class)
    private val signedJson = Json {
        ignoreUnknownKeys = false
        coerceInputValues = false
        encodeDefaults = true
        explicitNulls = false
    }

    fun candidates(
        context: Context,
        log: (String) -> Unit = {}
    ): List<SignedSyncRendezvousCandidate> {
        if (!topicRegex.matches(TOPIC) || !publicKeyRegex.matches(PINNED_PUBLIC_KEY)) {
            log("The signed public sync identity embedded in this app is invalid.")
            return emptyList()
        }
        val highWater = synchronized(highWaterLock) {
            readHighWaterStateLocked(context)
        }
        if (!highWater.valid) {
            log("The durable signed sync generation record is unreadable; remote discovery was rejected.")
            return emptyList()
        }
        val response = fetchBoundedSubscription(log) ?: return emptyList()
        val now = System.currentTimeMillis() / 1_000L
        // The response is byte-bounded. Verify every raw line before limiting
        // candidates so junk events cannot evict a valid signed announcement.
        val verified = response.lineSequence()
            .filter(String::isNotBlank)
            .mapNotNull { line -> verifiedCandidateFromEvent(line, now) }
            .toList()
        val newestGeneration = verified.maxOfOrNull { it.generation }
        if (
            newestGeneration != null &&
            verified.asSequence()
                .filter { it.generation == newestGeneration }
                .map { it.signedPayloadFingerprint }
                .distinct()
                .take(2)
                .count() > 1
        ) {
            log("Conflicting signed sync announcements used the same newest generation; remote discovery was rejected.")
            return emptyList()
        }
        val candidates = verified.asSequence()
            .filter { candidate -> candidateAllowedByHighWater(candidate, highWater) }
            .sortedWith(
                compareByDescending<SignedSyncRendezvousCandidate> { it.generation }
                    .thenByDescending { it.issuedAt }
            )
            .distinctBy { it.signedPayloadFingerprint }
            .take(2)
            .toList()
        if (candidates.isEmpty()) {
            log("No fresh signed public sync announcement was available.")
        } else {
            log("Verified a fresh signed public sync announcement.")
        }
        return candidates
    }

    /** Advances rollback protection only after the exact endpoint passed live proof. */
    fun acceptCandidate(
        context: Context,
        candidate: SignedSyncRendezvousCandidate,
        log: (String) -> Unit = {}
    ): Boolean = synchronized(highWaterLock) {
        if (
            candidate.generation <= 0L ||
            !fingerprintRegex.matches(candidate.signedPayloadFingerprint)
        ) {
            log("Rejected a malformed signed sync generation candidate.")
            return@synchronized false
        }
        val previous = readHighWaterStateLocked(context)
        if (!previous.valid) {
            log("The durable signed sync generation record is unreadable; its rollback guard was not replaced.")
            return@synchronized false
        }
        if (previous.present) {
            if (candidate.generation < previous.generation) {
                log("Rejected a signed sync announcement older than the durable accepted generation.")
                return@synchronized false
            }
            if (candidate.generation == previous.generation) {
                val samePayload = candidate.signedPayloadFingerprint == previous.signedPayloadFingerprint
                if (!samePayload) {
                    log("Rejected a different signed sync payload that reused an accepted generation.")
                }
                return@synchronized samePayload
            }
        }
        if (!writeHighWaterStateLocked(context, candidate)) {
            log("Could not durably advance the signed sync generation; the endpoint was rejected.")
            return@synchronized false
        }
        val persisted = readHighWaterStateLocked(context)
        val committed = persisted.valid &&
            persisted.present &&
            persisted.generation == candidate.generation &&
            persisted.signedPayloadFingerprint == candidate.signedPayloadFingerprint
        if (!committed) {
            log("The signed sync generation did not pass durable read-back verification.")
        }
        committed
    }

    private fun candidateAllowedByHighWater(
        candidate: SignedSyncRendezvousCandidate,
        highWater: HighWaterState
    ): Boolean = when {
        !highWater.present -> true
        candidate.generation > highWater.generation -> true
        candidate.generation < highWater.generation -> false
        else -> candidate.signedPayloadFingerprint == highWater.signedPayloadFingerprint
    }

    private fun highWaterFile(context: Context): AtomicFile = AtomicFile(
        File(
            context.applicationContext.noBackupFilesDir,
            "$HIGH_WATER_FILE_PREFIX$TOPIC.state"
        )
    )

    private fun readHighWaterStateLocked(context: Context): HighWaterState {
        val bytes = try {
            highWaterFile(context).openRead().use { input ->
                val output = ByteArrayOutputStream()
                val buffer = ByteArray(256)
                while (true) {
                    val read = input.read(buffer)
                    if (read < 0) break
                    if (output.size() + read > MAX_HIGH_WATER_BYTES) {
                        return HighWaterState(present = true, valid = false)
                    }
                    output.write(buffer, 0, read)
                }
                output.toByteArray()
            }
        } catch (_: FileNotFoundException) {
            return HighWaterState(present = false, valid = true)
        } catch (_: Throwable) {
            return HighWaterState(present = true, valid = false)
        }
        val text = String(bytes, Charsets.UTF_8)
        if (!text.toByteArray(Charsets.UTF_8).contentEquals(bytes)) {
            return HighWaterState(present = true, valid = false)
        }
        val fields = text.split('\n')
        val generation = fields.getOrNull(2)?.toLongOrNull()
        if (
            fields.size != 4 ||
            fields[0] != HIGH_WATER_SCHEMA ||
            fields[1] != TOPIC ||
            generation == null ||
            generation <= 0L ||
            !fingerprintRegex.matches(fields[3])
        ) {
            return HighWaterState(present = true, valid = false)
        }
        return HighWaterState(
            present = true,
            valid = true,
            generation = generation,
            signedPayloadFingerprint = fields[3]
        )
    }

    private fun writeHighWaterStateLocked(
        context: Context,
        candidate: SignedSyncRendezvousCandidate
    ): Boolean {
        val bytes = listOf(
            HIGH_WATER_SCHEMA,
            TOPIC,
            candidate.generation.toString(),
            candidate.signedPayloadFingerprint
        ).joinToString("\n").toByteArray(Charsets.UTF_8)
        if (bytes.size > MAX_HIGH_WATER_BYTES) return false
        val atomicFile = highWaterFile(context)
        var stream: FileOutputStream? = null
        return try {
            val pending = atomicFile.startWrite()
            stream = pending
            pending.write(bytes)
            atomicFile.finishWrite(pending)
            stream = null
            true
        } catch (_: Throwable) {
            stream?.let { failed -> runCatching { atomicFile.failWrite(failed) } }
            false
        }
    }

    private fun fetchBoundedSubscription(log: (String) -> Unit): String? = runCatching {
        val connection = URL("$SERVICE_BASE_URL/$TOPIC/json?poll=1&since=12h")
            .openConnection() as HttpURLConnection
        try {
            connection.connectTimeout = 7_000
            connection.readTimeout = 10_000
            connection.requestMethod = "GET"
            connection.instanceFollowRedirects = false
            connection.useCaches = false
            connection.setRequestProperty("Accept", "application/x-ndjson")
            connection.setRequestProperty("Cache-Control", "no-cache")
            check(connection.responseCode in 200..299)
            val bytes = connection.inputStream.use { input ->
                val output = ByteArrayOutputStream()
                val buffer = ByteArray(4 * 1024)
                while (true) {
                    val read = input.read(buffer)
                    if (read < 0) break
                    check(output.size() + read <= MAX_RESPONSE_BYTES)
                    output.write(buffer, 0, read)
                }
                output.toByteArray()
            }
            String(bytes, Charsets.UTF_8)
        } finally {
            connection.disconnect()
        }
    }.onFailure {
        log("The signed public sync announcement service is temporarily unreachable.")
    }.getOrNull()

    private fun verifiedCandidateFromEvent(
        eventLine: String,
        now: Long
    ): SignedSyncRendezvousCandidate? = runCatching {
        if (eventLine.isBlank() || eventLine.length > MAX_ENVELOPE_CHARS * 2) return@runCatching null
        val event = ntfyJson.decodeFromString<NtfyEvent>(eventLine)
        if (event.event != "message" || event.message.isBlank() || event.message.length > MAX_ENVELOPE_CHARS) {
            return@runCatching null
        }
        val envelope = signedJson.decodeFromString<SignedEnvelope>(event.message)
        if (
            envelope.schemaVersion != ENVELOPE_SCHEMA_VERSION ||
            envelope.publicKey != PINNED_PUBLIC_KEY ||
            !publicKeyRegex.matches(envelope.publicKey) ||
            !signatureRegex.matches(envelope.signature) ||
            envelope.payload.isEmpty() ||
            envelope.payload.toByteArray(Charsets.UTF_8).size > MAX_SIGNED_PAYLOAD_BYTES ||
            !NativeOptimizerBridge.verifySyncRendezvousSignature(
                payload = envelope.payload,
                publicKeyBase64 = envelope.publicKey,
                signatureBase64 = envelope.signature
            )
        ) {
            return@runCatching null
        }
        val payload = signedJson.decodeFromString<SignedPayload>(envelope.payload)
        if (signedJson.encodeToString(payload) != envelope.payload) return@runCatching null
        if (
            payload.schema != PAYLOAD_SCHEMA ||
            payload.syncProtocolVersion != SYNC_PROTOCOL_VERSION ||
            payload.identityId != TOPIC ||
            !buildIdRegex.matches(payload.serverBuildId) ||
            payload.generation <= 0L ||
            payload.issuedAt <= 0L ||
            payload.expiresAt <= payload.issuedAt ||
            payload.expiresAt - payload.issuedAt > MAX_LIFETIME_SECONDS ||
            payload.issuedAt > now + CLOCK_SKEW_SECONDS ||
            now > payload.expiresAt + CLOCK_SKEW_SECONDS
        ) {
            return@runCatching null
        }
        val canonicalUrl = safePublicHttpsBaseUrlOrNull(payload.serverUrl) ?: return@runCatching null
        if (canonicalUrl != payload.serverUrl) return@runCatching null
        SignedSyncRendezvousCandidate(
            serverUrl = canonicalUrl,
            serverBuildId = payload.serverBuildId,
            generation = payload.generation,
            issuedAt = payload.issuedAt,
            signedPayloadFingerprint = MessageDigest.getInstance("SHA-256")
                .digest(envelope.payload.toByteArray(Charsets.UTF_8))
                .joinToString(separator = "") { byte -> "%02x".format(byte.toInt() and 0xff) }
        )
    }.getOrNull()

    private fun safePublicHttpsBaseUrlOrNull(raw: String): String? {
        if (
            raw.isEmpty() ||
            raw.length > 2_048 ||
            raw != raw.trim() ||
            raw.any { it.isWhitespace() || it.isISOControl() } ||
            '\\' in raw
        ) {
            return null
        }
        val uri = runCatching { URI(raw) }.getOrNull() ?: return null
        if (
            !uri.isAbsolute ||
            !uri.scheme.equals("https", ignoreCase = true) ||
            uri.rawUserInfo != null ||
            uri.rawQuery != null ||
            uri.rawFragment != null ||
            uri.host.isNullOrBlank() ||
            uri.rawPath.orEmpty().contains("//")
        ) {
            return null
        }
        val host = uri.host?.lowercase() ?: return null
        if (
            !host.contains('.') ||
            host == "localhost" ||
            host.endsWith(".localhost") ||
            host.endsWith(".local") ||
            host.endsWith(".lan") ||
            host.endsWith(".internal") ||
            host.endsWith(".home") ||
            host.endsWith(".home.arpa") ||
            host.endsWith(".test") ||
            host.endsWith(".invalid") ||
            host.endsWith(".example") ||
            host.endsWith(".onion") ||
            isNonPublicIpv4(host)
        ) {
            return null
        }
        return raw.trimEnd('/')
    }

    private fun isNonPublicIpv4(host: String): Boolean {
        val parts = host.split('.')
        if (parts.size != 4) return false
        val octets = parts.map { it.toIntOrNull()?.takeIf { value -> value in 0..255 } ?: return true }
        val first = octets[0]
        val second = octets[1]
        val third = octets[2]
        return first == 0 ||
            first == 10 ||
            first == 127 ||
            first >= 224 ||
            (first == 100 && second in 64..127) ||
            (first == 169 && second == 254) ||
            (first == 172 && second in 16..31) ||
            (first == 192 && second == 168) ||
            (first == 192 && second == 0 && third == 2) ||
            (first == 198 && second in 18..19) ||
            (first == 198 && second == 51 && third == 100) ||
            (first == 203 && second == 0 && third == 113)
    }
}
"####;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_rendezvous_source_pins_identity_and_fails_closed() {
        assert!(CONTENTS.contains("PINNED_PUBLIC_KEY"));
        assert!(CONTENTS.contains("NativeOptimizerBridge.verifySyncRendezvousSignature"));
        assert!(CONTENTS.contains("payload.identityId != TOPIC"));
        assert!(CONTENTS.contains("payload.expiresAt - payload.issuedAt > MAX_LIFETIME_SECONDS"));
        assert!(CONTENTS.contains("instanceFollowRedirects = false"));
        assert!(CONTENTS.contains("since=12h"));
        assert!(CONTENTS.contains("connection.connectTimeout = 7_000"));
        assert!(CONTENTS.contains("connection.readTimeout = 10_000"));
        assert!(CONTENTS.contains("Cache-Control"));
        assert!(!CONTENTS.contains("takeLast(MAX_EVENT_LINES)"));
        let verify = CONTENTS
            .find(".mapNotNull { line -> verifiedCandidateFromEvent(line, now) }")
            .expect("events must be verified");
        let limit = CONTENTS
            .find(".take(2)")
            .expect("verified candidates must remain bounded");
        assert!(verify < limit);
        assert!(CONTENTS.contains(".take(2)"));
        assert!(CONTENTS.contains("AtomicFile"));
        assert!(CONTENTS.contains("noBackupFilesDir"));
        assert!(CONTENTS.contains("fun acceptCandidate("));
        assert!(CONTENTS.contains("candidate.generation < previous.generation"));
        assert!(CONTENTS.contains("candidate.generation == previous.generation"));
        assert!(CONTENTS
            .contains("candidate.signedPayloadFingerprint == previous.signedPayloadFingerprint"));
        assert!(CONTENTS.contains("durable read-back verification"));
        assert!(CONTENTS.contains("@OptIn(ExperimentalSerializationApi::class)"));
        assert!(!CONTENTS.contains("Authorization"));
    }
}
