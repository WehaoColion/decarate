// Rust-owned recovery baseline for the signed Android sync route.
pub const CONTENTS: &str = r########"
package com.ofairyo.gridtimer.data

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.Uri
import android.net.wifi.WifiManager
import kotlinx.serialization.Serializable
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import java.io.ByteArrayOutputStream
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.HttpURLConnection
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.URL
import java.security.MessageDigest
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

internal object SyncNetworkRoute {
    data class DiscoveryIdentity(
        val userId: String,
        val tokenId: String,
        val rawToken: String
    )

    @Serializable
    private data class DiscoveryProofRequest(
        val userId: String,
        val tokenId: String,
        val nonce: String,
        val clientProof: String = ""
    )

    @Serializable
    private data class DiscoveryProofResponse(
        val ok: Boolean = false,
        val message: String = "",
        val userId: String = "",
        val mode: String = "",
        val publicServerUrl: String = "",
        val nonce: String = "",
        val proof: String = ""
    )

    data class Resolution(
        val serverUrl: String,
        val rememberedServerUrl: String = "",
        val network: Network? = null,
        val message: String? = null
    )
    private data class HealthProbeResult(
        val serverUrl: String,
        val publicServerUrl: String = "",
        val serverBuildId: String = "",
        val syncProtocolVersion: Int = 0,
        val network: Network? = null
    )
    private data class NetworkProbeCandidate(
        val serverUrl: String,
        val network: Network
    )

    private const val SYNC_BEACON_PORT = 8918
    private const val SYNC_PROTOCOL_VERSION = __GRIDTIMER_RENDEZVOUS_SYNC_PROTOCOL_VERSION__
    private const val HEALTH_RESPONSE_MAX_BYTES = 16 * 1024
    private const val DISCOVERY_PROOF_RESPONSE_MAX_BYTES = 16 * 1024
    private const val DISCOVERY_PROOF_PATH = "/v1/discovery-proof"
    private const val DISCOVERY_PENDING_RECOVERY_DOMAIN = "gridtimer.discovery.pending-recovery.v1"
    private const val DISCOVERY_NONCE_MIN_BYTES = 16
    private const val DISCOVERY_NONCE_MAX_BYTES = 256
    private val DISCOVERY_PROOF_REGEX = Regex("^[a-f0-9]{64}$")
    private val discoveryJson = Json {
        ignoreUnknownKeys = true
        encodeDefaults = true
        coerceInputValues = true
    }
    private val PUBLIC_SERVER_URL_REGEX = Regex("\\\"publicServerUrl\\\"\\s*:\\s*\\\"([^\\\"]*)\\\"")
    private val SERVER_BUILD_ID_REGEX = Regex("\\\"serverBuildId\\\"\\s*:\\s*\\\"([^\\\"]*)\\\"")
    private val SYNC_PROTOCOL_VERSION_REGEX = Regex("\\\"syncProtocolVersion\\\"\\s*:\\s*(\\d+)")
    private val BEACON_SERVER_URL_REGEX = Regex("\\\"serverUrl\\\"\\s*:\\s*\\\"([^\\\"]*)\\\"")
    private val LOCAL_BEACON_SERVER_URL_REGEX = Regex("\\\"localServerUrl\\\"\\s*:\\s*\\\"([^\\\"]*)\\\"")

    fun <T> withLanRouteIfNeeded(
        context: Context,
        serverUrl: String,
        network: Network? = null,
        log: (String) -> Unit = {},
        block: () -> T
    ): T {
        val manager = context.applicationContext
            .getSystemService(ConnectivityManager::class.java)
        val trimmed = serverUrl.trim()
        if (network != null && manager != null) {
            val previousNetwork = manager.boundNetworkForProcess
            if (manager.bindProcessToNetwork(network)) {
                log("Sync request is bound to the network that answered local discovery.")
                return try {
                    block()
                } finally {
                    manager.bindProcessToNetwork(previousNetwork)
                }
            }
            log("Could not bind sync request to the discovered local network; using default route.")
        } else if (
            trimmed.startsWith("http://127.0.0.1", ignoreCase = true) ||
            trimmed.startsWith("http://localhost", ignoreCase = true)
        ) {
            log("Sync target is device-local; use the computer's reachable sync address instead.")
        } else {
            log("Sync request uses Android's current default network route.")
        }
        return block()
    }

    fun resolveServerUrl(
        context: Context,
        configuredServerUrl: String,
        lastResolvedServerUrl: String,
        discoveryIdentity: DiscoveryIdentity? = null,
        log: (String) -> Unit = {}
    ): Resolution {
        val configured = configuredServerUrl.trim().trimEnd('/').ifBlank { AUTO_SYNC_SERVER_URL }
        val lastResolved = lastResolvedServerUrl.trim().trimEnd('/')
        if (isAutoServerUrl(configured) || isDeviceLocalUrl(configured)) {
            for (candidate in SyncRemoteRendezvous.candidates(context, log)) {
                val target = verifiedSignedPublicServerUrlOrNull(
                    candidate = candidate,
                    identity = discoveryIdentity,
                    log = log
                ) ?: continue
                if (!SyncRemoteRendezvous.acceptCandidate(context, candidate, log)) {
                    continue
                }
                return Resolution(serverUrl = target, rememberedServerUrl = target)
            }

            if (discoveryIdentity != null) {
                publicServerUrlOrNull(BOOTSTRAP_PUBLIC_SYNC_SERVER_URL)?.let { packaged ->
                    verifiedBootstrapPublicServerUrlOrNull(
                        publicServerUrl = packaged,
                        identity = discoveryIdentity,
                        log = log
                    )?.let { target ->
                        return Resolution(serverUrl = target, rememberedServerUrl = target)
                    }
                }
            }

            if (discoveryIdentity != null) {
                verifiedRememberedPublicServerUrlOrNull(lastResolved, discoveryIdentity, log)?.let { target ->
                    return Resolution(serverUrl = target, rememberedServerUrl = target)
                }
            }

            if (discoveryIdentity != null) {
                beaconReachableServerUrlOrNull(context, log)?.let { probe ->
                    verifiedDiscoveredServerUrlOrNull(probe, discoveryIdentity, log)?.let { target ->
                        return Resolution(
                            serverUrl = target,
                            rememberedServerUrl = target,
                            network = probe.network
                        )
                    }
                }
                localReachableServerUrlOrNull(context, lastResolved)?.let { probe ->
                    verifiedDiscoveredServerUrlOrNull(probe, discoveryIdentity, log)?.let { target ->
                        return Resolution(
                            serverUrl = target,
                            rememberedServerUrl = target,
                            network = probe.network
                        )
                    }
                }
                discoverLocalServerUrl(context, log)?.let { probe ->
                    verifiedDiscoveredServerUrlOrNull(probe, discoveryIdentity, log)?.let { target ->
                        return Resolution(
                            serverUrl = target,
                            rememberedServerUrl = target,
                            network = probe.network
                        )
                    }
                }
            }
            return Resolution(
                serverUrl = configured,
                message = "未能通过电脑公网入口与账号校验。请确认电脑端同步服务正在运行且电脑可以访问互联网；如果刚完成登录，请退出账号后重新登录一次。手机无需与电脑连接同一局域网。"
            )
        }

        if (isPrivateServerUrl(configured) && isCellularOnly(context)) {
            return Resolution(
                serverUrl = configured,
                message = mobileDataPrivateAddressMessage(context, configured)
                    ?: "当前流量网络访问不到这个内网同步地址。"
            )
        }

        if (isPrivateServerUrl(configured)) {
            localReachableServerUrlOrNull(context, configured)?.let { probe ->
                verifiedDiscoveredServerUrlOrNull(probe, discoveryIdentity, log)?.let { target ->
                    return Resolution(serverUrl = target, rememberedServerUrl = target, network = probe.network)
                }
            }
            beaconReachableServerUrlOrNull(context, log)?.let { probe ->
                verifiedDiscoveredServerUrlOrNull(probe, discoveryIdentity, log)?.let { target ->
                    return Resolution(serverUrl = target, rememberedServerUrl = target, network = probe.network)
                }
            }
            discoverLocalServerUrl(context, log)?.let { probe ->
                verifiedDiscoveredServerUrlOrNull(probe, discoveryIdentity, log)?.let { target ->
                    return Resolution(serverUrl = target, rememberedServerUrl = target, network = probe.network)
                }
            }
            return Resolution(
                serverUrl = configured,
                message = "没有在当前网络里找到电脑端同步服务，请确认电脑端服务正在运行。"
            )
        }

        val safeConfigured = credentialSafeServerUrlOrNull(configured)
        return if (safeConfigured != null) {
            Resolution(safeConfigured, rememberedServerUrl = safeConfigured)
        } else {
            Resolution(
                serverUrl = configured,
                message = "The sync base URL is not credential-safe. Use HTTPS without user info, query, or fragment."
            )
        }
    }

    fun storedServerUrlForAutomaticMode(serverUrl: String): String {
        val trimmed = serverUrl.trim()
        return if (
            trimmed.isBlank() ||
            isAutoServerUrl(trimmed) ||
            isDeviceLocalUrl(trimmed) ||
            isPrivateServerUrl(trimmed)
        ) {
            AUTO_SYNC_SERVER_URL
        } else {
            trimmed
        }
    }

    fun isAutoServerUrl(serverUrl: String): Boolean =
        serverUrl.trim().equals(AUTO_SYNC_SERVER_URL, ignoreCase = true)

    fun mobileDataPrivateAddressMessage(context: Context, serverUrl: String): String? {
        val host = runCatching { Uri.parse(serverUrl.trim()).host }
            .getOrNull()
            ?.trim()
            .orEmpty()
        val ipv4 = parseIpv4Literal(host) ?: return null
        if (!isPrivateIpv4(ipv4) || !isCellularOnly(context)) {
            return null
        }
        return "$host 是内网地址，当前 5G/流量访问不到。请把同步地址改成公网域名、内网穿透地址，或路由器端口转发后的地址。"
    }

    private fun isCellularOnly(context: Context): Boolean {
        val manager = context.applicationContext
            .getSystemService(ConnectivityManager::class.java)
            ?: return false
        val network = manager.activeNetwork ?: return false
        val capabilities = manager.getNetworkCapabilities(network) ?: return false
        if (
            capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) ||
            capabilities.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) ||
            capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN)
        ) {
            return false
        }
        return capabilities.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)
    }

    private fun publicServerUrlOrNull(serverUrl: String): String? {
        val trimmed = serverUrl.trim().trimEnd('/')
        if (trimmed.isBlank() || isPrivateServerUrl(trimmed) || isDeviceLocalUrl(trimmed)) {
            return null
        }
        val uri = runCatching { Uri.parse(trimmed) }.getOrNull() ?: return null
        return trimmed.takeIf {
            uri.scheme.equals("https", ignoreCase = true) &&
                !uri.host.isNullOrBlank() &&
                uri.userInfo == null &&
                uri.query.isNullOrEmpty() &&
                uri.fragment == null
        }
    }

    private fun verifiedDiscoveredServerUrlOrNull(
        probe: HealthProbeResult,
        identity: DiscoveryIdentity?,
        log: (String) -> Unit
    ): String? {
        val localBase = probe.serverUrl.trim().trimEnd('/')
        val publicCandidate = publicServerUrlOrNull(probe.publicServerUrl)
            ?: publicServerUrlOrNull(localBase)
        val resolvedIdentity = identity?.takeIf {
            it.userId.isNotBlank() && it.rawToken.isNotBlank()
        }
        if (resolvedIdentity == null) {
            log("Rejected unsigned discovery while no authenticated account proof was available.")
            return null
        }
        if (publicCandidate != null) {
            verifiedPublicServerFromDiscoveryProof(
                proofServerUrl = publicCandidate,
                network = null,
                identity = resolvedIdentity,
                log = log
            )?.let { return it }
        }
        credentialSafeServerUrlOrNull(localBase)?.let { safe ->
            if (isDeviceLocalUrl(safe)) {
                return safe
            }
        }
        if (!isPrivateServerUrl(localBase)) {
            log("Rejected an unsigned LAN discovery result outside the private network.")
            return null
        }
        return verifiedPublicServerFromDiscoveryProof(
            proofServerUrl = localBase,
            network = probe.network,
            identity = resolvedIdentity,
            log = log
        )
    }

    private fun verifiedBootstrapPublicServerUrlOrNull(
        publicServerUrl: String,
        identity: DiscoveryIdentity,
        log: (String) -> Unit
    ): String? {
        val resolvedIdentity = identity.takeIf {
            it.userId.isNotBlank() && it.rawToken.isNotBlank()
        } ?: run {
            log("Rejected the bootstrap endpoint until an authenticated account proof is available.")
            return null
        }
        val reachable = publicReachableServerUrlOrNull(publicServerUrl) ?: run {
            log("The desktop HTTPS endpoint did not pass the account bootstrap health check.")
            return null
        }
        if (reachable.syncProtocolVersion != SYNC_PROTOCOL_VERSION) {
            log("The desktop HTTPS endpoint uses an incompatible sync protocol.")
            return null
        }
        val proofBound = verifiedPublicServerFromDiscoveryProof(
            proofServerUrl = reachable.serverUrl,
            network = null,
            identity = resolvedIdentity,
            log = log
        ) ?: return null
        if (proofBound != reachable.serverUrl) {
            log("The bootstrap endpoint proof returned a different public route.")
            return null
        }
        log("Verified the bootstrap endpoint with the authenticated account proof.")
        return proofBound
    }

    private fun verifiedRememberedPublicServerUrlOrNull(
        rememberedServerUrl: String,
        identity: DiscoveryIdentity?,
        log: (String) -> Unit
    ): String? {
        val resolvedIdentity = identity?.takeIf {
            it.userId.isNotBlank() && it.rawToken.isNotBlank()
        }
        if (resolvedIdentity == null) {
            log("Ignored a remembered temporary endpoint until an authenticated proof is available.")
            return null
        }
        val reachable = publicReachableServerUrlOrNull(rememberedServerUrl) ?: return null
        if (reachable.syncProtocolVersion != SYNC_PROTOCOL_VERSION) {
            log("The remembered desktop endpoint uses an incompatible sync protocol.")
            return null
        }
        return verifiedPublicServerFromDiscoveryProof(
            proofServerUrl = reachable.serverUrl,
            network = null,
            identity = resolvedIdentity,
            log = log
        )
    }

    private fun verifiedSignedPublicServerUrlOrNull(
        candidate: SignedSyncRendezvousCandidate,
        identity: DiscoveryIdentity?,
        log: (String) -> Unit
    ): String? {
        if (!DISCOVERY_PROOF_REGEX.matches(candidate.signedPayloadFingerprint)) {
            log("A signed desktop endpoint omitted its verified installation payload fingerprint.")
            return null
        }
        val reachable = publicReachableServerUrlOrNull(candidate.serverUrl) ?: run {
            log("A signed desktop endpoint did not pass the HTTPS health check.")
            return null
        }
        if (reachable.syncProtocolVersion != SYNC_PROTOCOL_VERSION) {
            log("A signed desktop endpoint uses an incompatible sync protocol.")
            return null
        }
        if (reachable.serverBuildId != candidate.serverBuildId) {
            log("A signed desktop endpoint did not match its announced server build.")
            return null
        }
        val resolvedIdentity = identity?.takeIf {
            it.userId.isNotBlank() && it.rawToken.isNotBlank()
        }
        if (resolvedIdentity == null) {
            log("Verified a live endpoint announced by the pinned desktop installation identity.")
            return reachable.serverUrl
        }
        val proofBound = verifiedPublicServerFromDiscoveryProof(
            proofServerUrl = reachable.serverUrl,
            network = null,
            identity = resolvedIdentity,
            log = log
        ) ?: return null
        if (proofBound != candidate.serverUrl) {
            log("The signed endpoint proof returned a different public route.")
            return null
        }
        return proofBound
    }

    private fun credentialSafeServerUrlOrNull(serverUrl: String): String? {
        val trimmed = serverUrl.trim().trimEnd('/')
        val uri = runCatching { Uri.parse(trimmed) }.getOrNull() ?: return null
        val scheme = uri.scheme?.lowercase().orEmpty()
        val host = uri.host?.lowercase().orEmpty()
        if (
            uri.userInfo != null ||
            !uri.query.isNullOrEmpty() ||
            uri.fragment != null
        ) {
            return null
        }
        return when {
            scheme == "https" && host.isNotBlank() -> trimmed
            scheme == "http" && (host == "127.0.0.1" || host == "localhost") -> trimmed
            else -> null
        }
    }

    private fun verifiedPublicServerFromDiscoveryProof(
        proofServerUrl: String,
        network: Network?,
        identity: DiscoveryIdentity,
        log: (String) -> Unit
    ): String? {
        val proofBase = proofServerUrl.trim().trimEnd('/')
        val proofUri = runCatching { Uri.parse(proofBase) }.getOrNull() ?: return null
        val proofScheme = proofUri.scheme?.lowercase().orEmpty()
        if (
            proofUri.host.isNullOrBlank() ||
            proofUri.userInfo != null ||
            !proofUri.query.isNullOrEmpty() ||
            proofUri.fragment != null ||
            (proofScheme != "http" && proofScheme != "https") ||
            (publicServerUrlOrNull(proofBase) == null && !isPrivateServerUrl(proofBase))
        ) {
            return null
        }
        val nonce = UUID.randomUUID().toString()
        val nonceBytes = nonce.toByteArray(Charsets.UTF_8)
        if (
            nonceBytes.size !in DISCOVERY_NONCE_MIN_BYTES..DISCOVERY_NONCE_MAX_BYTES ||
            nonce.any(Char::isISOControl)
        ) {
            return null
        }
        val tokenId = identity.tokenId.trim().ifBlank { stableTokenId(identity.rawToken) }
        if (tokenId.isBlank()) {
            return null
        }
        val userId = identity.userId.trim()
        if (userId.isBlank()) {
            return null
        }
        val tokenHash = MessageDigest.getInstance("SHA-256")
            .digest(identity.rawToken.toByteArray(Charsets.UTF_8))
        val isPublicProof = publicServerUrlOrNull(proofBase) == proofBase
        val clientProof = if (isPublicProof) {
            val recoveryMessage = listOf(
                DISCOVERY_PENDING_RECOVERY_DOMAIN,
                userId,
                tokenId,
                nonce,
                proofBase
            ).joinToString("\n")
            val recoveryMac = Mac.getInstance("HmacSHA256").apply {
                init(SecretKeySpec(tokenHash, "HmacSHA256"))
            }
            recoveryMac.doFinal(recoveryMessage.toByteArray(Charsets.UTF_8))
                .joinToString(separator = "") { byte -> "%02x".format(byte.toInt() and 0xff) }
        } else {
            ""
        }
        val requestBytes = discoveryJson.encodeToString(
            DiscoveryProofRequest(
                userId = userId,
                tokenId = tokenId,
                nonce = nonce,
                clientProof = clientProof
            )
        ).toByteArray(Charsets.UTF_8)
        val attemptCount = if (isPublicProof) 3 else 1
        for (attempt in 0 until attemptCount) {
            val verified = runCatching {
                val url = URL("$proofBase$DISCOVERY_PROOF_PATH")
                val connection = if (network != null) {
                    network.openConnection(url)
                } else {
                    url.openConnection()
                } as HttpURLConnection
                try {
                    connection.connectTimeout = 2_500
                    connection.readTimeout = 4_000
                    connection.requestMethod = "POST"
                    connection.instanceFollowRedirects = false
                    connection.doOutput = true
                    connection.useCaches = false
                    connection.setRequestProperty("Accept", "application/json")
                    connection.setRequestProperty("Content-Type", "application/json; charset=utf-8")
                    connection.setFixedLengthStreamingMode(requestBytes.size)
                    connection.outputStream.use { output ->
                        output.write(requestBytes)
                        output.flush()
                    }
                    check(connection.responseCode in 200..299) { "Discovery proof endpoint rejected the request." }
                    val response = discoveryJson.decodeFromString<DiscoveryProofResponse>(
                        readBoundedResponseBody(connection, DISCOVERY_PROOF_RESPONSE_MAX_BYTES)
                    )
                    val publicUrl = response.publicServerUrl
                    check(response.ok)
                    check(response.mode == "discovery_proof")
                    check(response.userId == userId)
                    check(response.nonce == nonce)
                    check(publicUrl == publicUrl.trim().trimEnd('/'))
                    check(publicServerUrlOrNull(publicUrl) == publicUrl)
                    check(DISCOVERY_PROOF_REGEX.matches(response.proof))
                    val mac = Mac.getInstance("HmacSHA256").apply {
                        init(SecretKeySpec(tokenHash, "HmacSHA256"))
                    }
                    val expectedProof = mac.doFinal(
                        "$nonce\n$publicUrl".toByteArray(Charsets.UTF_8)
                    ).joinToString(separator = "") { byte -> "%02x".format(byte.toInt() and 0xff) }
                    check(
                        MessageDigest.isEqual(
                            expectedProof.toByteArray(Charsets.US_ASCII),
                            response.proof.toByteArray(Charsets.US_ASCII)
                        )
                    ) { "Discovery proof did not match the authenticated account." }
                    publicUrl
                } finally {
                    connection.disconnect()
                }
            }.onFailure {
                log("Sync discovery proof attempt ${attempt + 1}/$attemptCount failed; no bearer token or account data were sent.")
            }.getOrNull()
            if (verified != null) {
                log("Sync discovery proof verified; credentials will use the authenticated HTTPS endpoint.")
                return verified
            }
            if (attempt + 1 < attemptCount) {
                Thread.sleep(150L * (attempt + 1))
            }
        }
        return null
    }

    private fun stableTokenId(rawToken: String): String {
        if (rawToken.isBlank()) {
            return ""
        }
        return MessageDigest.getInstance("SHA-256")
            .digest(rawToken.toByteArray(Charsets.UTF_8))
            .joinToString(separator = "") { byte -> "%02x".format(byte.toInt() and 0xff) }
            .take(32)
    }

    private fun rememberedLocalServerUrlOrNull(serverUrl: String): String? {
        val trimmed = serverUrl.trim()
        if (
            trimmed.isBlank() ||
            (!isPrivateServerUrl(trimmed) && !isDeviceLocalUrl(trimmed))
        ) {
            return null
        }
        return trimmed.takeIf { it.startsWith("http://", ignoreCase = true) || it.startsWith("https://", ignoreCase = true) }
    }

    private fun publicReachableServerUrlOrNull(serverUrl: String): HealthProbeResult? =
        publicServerUrlOrNull(serverUrl)?.let { healthProbe(it) }

    private fun localReachableServerUrlOrNull(
        context: Context,
        serverUrl: String
    ): HealthProbeResult? {
        val trimmed = serverUrl.trim()
        if (!isPrivateServerUrl(trimmed)) {
            return null
        }
        reachableLocalNetworks(context).forEach { network ->
            healthProbe(trimmed, network, fastLocalProbe = true)?.let { return it }
        }
        return healthProbe(trimmed, fastLocalProbe = true)
    }

    private fun discoverLocalServerUrl(context: Context, log: (String) -> Unit): HealthProbeResult? {
        val manager = context.applicationContext
            .getSystemService(ConnectivityManager::class.java)
            ?: return null
        val candidates = reachableLocalNetworks(context)
            .flatMap { network ->
                val linkProperties = manager.getLinkProperties(network) ?: return@flatMap emptyList()
                linkProperties.linkAddresses
                    .mapNotNull { linkAddress ->
                        val address = linkAddress.address as? java.net.Inet4Address ?: return@mapNotNull null
                        val ipv4 = parseIpv4Literal(address.hostAddress.orEmpty()) ?: return@mapNotNull null
                        ipv4Candidates(ipv4, linkAddress.prefixLength)
                            .map { NetworkProbeCandidate(it, network) }
                    }
                    .flatten()
            }
            .distinctBy { candidate -> candidate.network.toString() to candidate.serverUrl }
        if (candidates.isEmpty()) {
            return null
        }
        log("Scanning local network for sync service candidates=${candidates.size}.")
        return firstHealthyCandidate(candidates)
    }

    private fun beaconReachableServerUrlOrNull(
        context: Context,
        log: (String) -> Unit
    ): HealthProbeResult? {
        val wifiManager = context.applicationContext.getSystemService(WifiManager::class.java)
        val lock = wifiManager?.createMulticastLock("grid_timer_sync_beacon")?.apply {
            setReferenceCounted(false)
            runCatching { acquire() }
        }
        return try {
            val deadline = System.currentTimeMillis() + 1_800L
            val networks: List<Network?> = reachableLocalNetworks(context).map { it } + listOf(null)
            for (network in networks) {
                beaconProbeOnNetwork(network, deadline, log)?.let { return it }
            }
            null
        } finally {
            lock?.let { multicastLock ->
                if (multicastLock.isHeld) {
                    runCatching { multicastLock.release() }
                }
            }
        }
    }

    private fun beaconProbeOnNetwork(
        network: Network?,
        deadline: Long,
        log: (String) -> Unit
    ): HealthProbeResult? {
        if (System.currentTimeMillis() >= deadline) {
            return null
        }
        val socket = DatagramSocket(null)
        socket.use { datagram ->
            datagram.reuseAddress = true
            datagram.bind(InetSocketAddress(SYNC_BEACON_PORT))
            if (network != null) {
                runCatching { network.bindSocket(datagram) }
            }
            sendBeaconDiscoveryRequest(datagram)
            val remaining = (deadline - System.currentTimeMillis()).coerceAtLeast(120L)
            datagram.soTimeout = remaining.coerceAtMost(360L).toInt()
            val buffer = ByteArray(2048)
            while (System.currentTimeMillis() < deadline) {
                val packet = DatagramPacket(buffer, buffer.size)
                val text = runCatching {
                    datagram.receive(packet)
                    String(packet.data, packet.offset, packet.length, Charsets.UTF_8)
                }.getOrNull() ?: break
                val candidates = beaconServerUrlsFromBody(text)
                candidates.firstNotNullOfOrNull(::publicServerUrlOrNull)?.let { publicCandidate ->
                    // This HTTPS address is still untrusted. The caller must complete
                    // the token-bound discovery proof before it may send a Bearer token.
                    log("Desktop beacon advertised an HTTPS discovery-proof candidate.")
                    return HealthProbeResult(
                        serverUrl = publicCandidate,
                        publicServerUrl = publicCandidate
                    )
                }
                candidates
                    .filter { candidate -> isPrivateServerUrl(candidate) || isDeviceLocalUrl(candidate) }
                    .forEach { candidate ->
                    val probe = healthProbe(candidate, network, fastLocalProbe = true)
                    probe?.let {
                            log("Sync service discovered through desktop beacon.")
                            return it
                    }
                }
            }
        }
        return null
    }

    private fun sendBeaconDiscoveryRequest(datagram: DatagramSocket) {
        runCatching { datagram.broadcast = true }
        val payload = "{\"type\":\"grid_timer_sync_discovery\",\"version\":1}".toByteArray(Charsets.UTF_8)
        listOf("255.255.255.255", "239.255.89.17").forEach { host ->
            runCatching {
                val packet = DatagramPacket(
                    payload,
                    payload.size,
                    InetAddress.getByName(host),
                    SYNC_BEACON_PORT
                )
                datagram.send(packet)
            }
        }
    }

    private fun reachableLocalNetworks(context: Context): List<Network> {
        val manager = context.applicationContext
            .getSystemService(ConnectivityManager::class.java)
            ?: return emptyList()
        return manager.allNetworks
            .filter { network ->
                val capabilities = manager.getNetworkCapabilities(network) ?: return@filter false
                capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) ||
                    capabilities.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)
            }
            .sortedByDescending { network -> network == manager.activeNetwork }
    }

    private fun firstHealthyCandidate(candidates: List<NetworkProbeCandidate>): HealthProbeResult? {
        val found = AtomicReference<HealthProbeResult?>(null)
        val latch = CountDownLatch(candidates.size)
        val executor = Executors.newFixedThreadPool(minOf(64, candidates.size))
        for (candidate in candidates) {
            executor.execute {
                try {
                    if (found.get() == null) {
                        healthProbe(
                            candidate.serverUrl,
                            candidate.network,
                            fastLocalProbe = true
                        )?.let { probe ->
                            found.compareAndSet(null, probe)
                        }
                    }
                } finally {
                    latch.countDown()
                }
            }
        }
        latch.await(4_500L, TimeUnit.MILLISECONDS)
        executor.shutdownNow()
        return found.get()
    }

    private fun healthProbe(
        serverUrl: String,
        network: Network? = null,
        fastLocalProbe: Boolean = false
    ): HealthProbeResult? {
        val base = serverUrl.trim().trimEnd('/')
        if (!base.startsWith("http://", ignoreCase = true) && !base.startsWith("https://", ignoreCase = true)) {
            return null
        }
        val publicProbe = publicServerUrlOrNull(base) != null
        val attemptCount = if (publicProbe) 3 else 1
        for (attempt in 0 until attemptCount) {
            val healthy = runCatching {
                val url = URL("$base/health")
                val connection = if (network != null) {
                    network.openConnection(url)
                } else {
                    url.openConnection()
                } as HttpURLConnection
                try {
                    connection.connectTimeout = when {
                        publicProbe -> 3_500
                        fastLocalProbe -> 350
                        else -> 1_200
                    }
                    connection.readTimeout = when {
                        publicProbe -> 5_000
                        fastLocalProbe -> 850
                        else -> 1_800
                    }
                    connection.requestMethod = "GET"
                    connection.instanceFollowRedirects = false
                    connection.setRequestProperty("Accept", "application/json")
                    connection.useCaches = false
                    val ok = connection.responseCode in 200..299
                    val body = if (ok) {
                        readBoundedResponseBody(connection, HEALTH_RESPONSE_MAX_BYTES)
                    } else {
                        ""
                    }
                    if (ok && body.contains("\"mode\":\"health\"")) {
                        HealthProbeResult(
                            serverUrl = base,
                            publicServerUrl = if (publicProbe) {
                                base
                            } else {
                                publicServerUrlOrNull(publicServerUrlFromHealthBody(body)).orEmpty()
                            },
                            serverBuildId = serverBuildIdFromHealthBody(body),
                            syncProtocolVersion = syncProtocolVersionFromHealthBody(body),
                            network = network
                        )
                    } else {
                        null
                    }
                } finally {
                    connection.disconnect()
                }
            }.getOrNull()
            if (healthy != null) {
                return healthy
            }
            if (attempt + 1 < attemptCount) {
                Thread.sleep(150L * (attempt + 1))
            }
        }
        return null
    }

    private fun readBoundedResponseBody(
        connection: HttpURLConnection,
        maxBytes: Int
    ): String {
        val output = ByteArrayOutputStream()
        connection.inputStream.use { input ->
            val buffer = ByteArray(4 * 1024)
            while (true) {
                val read = input.read(buffer)
                if (read < 0) break
                check(output.size() + read <= maxBytes) { "Sync discovery response exceeded its size limit." }
                output.write(buffer, 0, read)
            }
        }
        return String(output.toByteArray(), Charsets.UTF_8)
    }

    private fun publicServerUrlFromHealthBody(body: String): String =
        PUBLIC_SERVER_URL_REGEX.find(body)
            ?.groupValues
            ?.getOrNull(1)
            ?.let(::decodeJsonString)
            .orEmpty()

    private fun serverBuildIdFromHealthBody(body: String): String =
        SERVER_BUILD_ID_REGEX.find(body)
            ?.groupValues
            ?.getOrNull(1)
            ?.let(::decodeJsonString)
            .orEmpty()

    private fun syncProtocolVersionFromHealthBody(body: String): Int =
        SYNC_PROTOCOL_VERSION_REGEX.find(body)
            ?.groupValues
            ?.getOrNull(1)
            ?.toIntOrNull()
            ?: 0

    private fun serverUrlFromBeaconBody(body: String): String =
        BEACON_SERVER_URL_REGEX.find(body)
            ?.groupValues
            ?.getOrNull(1)
            ?.let(::decodeJsonString)
            .orEmpty()

    private fun localServerUrlFromBeaconBody(body: String): String =
        LOCAL_BEACON_SERVER_URL_REGEX.find(body)
            ?.groupValues
            ?.getOrNull(1)
            ?.let(::decodeJsonString)
            .orEmpty()

    private fun beaconServerUrlsFromBody(body: String): List<String> =
        listOf(
            publicServerUrlFromHealthBody(body),
            serverUrlFromBeaconBody(body),
            localServerUrlFromBeaconBody(body)
        )
            .map { it.trim() }
            .filter { it.startsWith("http://", ignoreCase = true) || it.startsWith("https://", ignoreCase = true) }
            .distinct()

    private fun decodeJsonString(raw: String): String =
        raw.replace("\\/", "/")
            .replace("\\\"", "\"")
            .replace("\\\\", "\\")

    private fun ipv4Candidates(localIpv4: Int, prefixLength: Int): List<String> {
        val bits = prefixLength.takeIf { it in 24..30 } ?: 24
        val mask = -1 shl (32 - bits)
        val network = localIpv4 and mask
        val broadcast = network or mask.inv()
        return ((network + 1) until broadcast)
            .filter { it != localIpv4 }
            .sortedBy { candidate ->
                kotlin.math.abs(candidate.toLong() - localIpv4.toLong())
            }
            .take(512)
            .map { "http://${formatIpv4(it)}:8917" }
    }

    private fun isDeviceLocalUrl(serverUrl: String): Boolean {
        val trimmed = serverUrl.trim()
        return trimmed.startsWith("http://127.0.0.1", ignoreCase = true) ||
            trimmed.startsWith("http://localhost", ignoreCase = true)
    }

    private fun isPrivateServerUrl(serverUrl: String): Boolean {
        val host = runCatching { Uri.parse(serverUrl.trim()).host }
            .getOrNull()
            ?.trim()
            .orEmpty()
        return parseIpv4Literal(host)?.let(::isPrivateIpv4) == true
    }

    private fun parseIpv4Literal(host: String): Int? {
        val parts = host.split('.')
        if (parts.size != 4) {
            return null
        }
        var value = 0
        for (part in parts) {
            val octet = part.toIntOrNull()?.takeIf { it in 0..255 } ?: return null
            value = (value shl 8) or octet
        }
        return value
    }

    private fun isPrivateIpv4(value: Int): Boolean {
        val first = (value ushr 24) and 0xff
        val second = (value ushr 16) and 0xff
        val third = (value ushr 8) and 0xff
        return first == 10 ||
            first == 0 ||
            first == 127 ||
            first >= 224 ||
            (first == 100 && second in 64..127) ||
            (first == 172 && second in 16..31) ||
            (first == 192 && second == 168) ||
            (first == 169 && second == 254) ||
            (first == 198 && second in 18..19) ||
            (first == 192 && second == 0 && third == 2) ||
            (first == 198 && second == 51 && third == 100) ||
            (first == 203 && second == 0 && third == 113)
    }

    private fun formatIpv4(value: Int): String =
        listOf(
            (value ushr 24) and 0xff,
            (value ushr 16) and 0xff,
            (value ushr 8) and 0xff,
            value and 0xff
        ).joinToString(".")
}

"########;
