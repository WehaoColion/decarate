// v2.22.21 - Harden Android automatic route discovery without rewriting the monolithic template.

pub const PATH: &str = "com/ofairyo/gridtimer/data/SyncNetworkRoute.kt";

#[path = "sync_network_route_source.rs"]
mod secure_sync_network_route_source;

pub fn render(base: &str) -> Result<String, String> {
    if !base.contains("DiscoveryIdentity") || !base.contains("DISCOVERY_PROOF_REGEX") {
        let recovered = secure_sync_network_route_source::CONTENTS;
        if !recovered.contains("SyncRemoteRendezvous.candidates(context, log)")
            || !recovered.contains("SyncRemoteRendezvous.acceptCandidate(context, candidate, log)")
            || !recovered.contains("verifiedPublicServerFromDiscoveryProof(")
        {
            return Err("recovered SyncNetworkRoute source is missing signed discovery safeguards".to_string());
        }
        return Ok(recovered.to_string());
    }

    let mut rendered = base.to_string();

    replace_exactly_once(
        &mut rendered,
        r####"            for (candidate in SyncRemoteRendezvous.candidates(log)) {
                val target = verifiedSignedPublicServerUrlOrNull(
                    candidate = candidate,
                    identity = discoveryIdentity,
                    log = log
                ) ?: continue
                return Resolution(serverUrl = target, rememberedServerUrl = target)
            }

            publicServerUrlOrNull(BOOTSTRAP_PUBLIC_SYNC_SERVER_URL)?.let { packaged ->
                verifiedBootstrapPublicServerUrlOrNull(packaged, log)?.let { target ->
                    return Resolution(serverUrl = target, rememberedServerUrl = target)
                }
            }
"####,
        r####"            for (candidate in SyncRemoteRendezvous.candidates(context, log)) {
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
"####,
        "automatic signed and bootstrap route selection",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"        if (resolvedIdentity == null) {
            val packagedPublicUrl = publicServerUrlOrNull(BOOTSTRAP_PUBLIC_SYNC_SERVER_URL)
            if (publicCandidate == null || publicCandidate != packagedPublicUrl) {
                log("Rejected unsigned discovery while no authenticated account proof was available.")
                return null
            }
            return verifiedBootstrapPublicServerUrlOrNull(publicCandidate, log)
        }
"####,
        r####"        if (resolvedIdentity == null) {
            log("Rejected unsigned discovery while no authenticated account proof was available.")
            return null
        }
"####,
        "unsigned discovery rejection",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"    private fun verifiedBootstrapPublicServerUrlOrNull(
        publicServerUrl: String,
        log: (String) -> Unit
    ): String? {
        val reachable = publicReachableServerUrlOrNull(publicServerUrl) ?: run {
            log("The desktop HTTPS endpoint did not pass the account bootstrap health check.")
            return null
        }
        if (reachable.syncProtocolVersion != SYNC_PROTOCOL_VERSION) {
            log("The desktop HTTPS endpoint uses an incompatible sync protocol.")
            return null
        }
        log("Verified a protocol-compatible desktop HTTPS endpoint without sending a bearer token.")
        return reachable.serverUrl
    }
"####,
        r####"    private fun verifiedBootstrapPublicServerUrlOrNull(
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
"####,
        "token-bound bootstrap verification",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"    private fun verifiedSignedPublicServerUrlOrNull(
        candidate: SignedSyncRendezvousCandidate,
        identity: DiscoveryIdentity?,
        log: (String) -> Unit
    ): String? {
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
            return reachable.serverUrl
        }
        return verifiedPublicServerFromDiscoveryProof(
            proofServerUrl = reachable.serverUrl,
            network = null,
            identity = resolvedIdentity,
            log = log
        )
    }
"####,
        r####"    private fun verifiedSignedPublicServerUrlOrNull(
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
"####,
        "signed route live proof",
    )?;

    for forbidden in [
        "SyncRemoteRendezvous.candidates(log)",
        "verifiedBootstrapPublicServerUrlOrNull(packaged, log)",
        "return verifiedBootstrapPublicServerUrlOrNull(publicCandidate, log)",
        "Verified a protocol-compatible desktop HTTPS endpoint without sending a bearer token.",
    ] {
        if rendered.contains(forbidden) {
            return Err(format!(
                "unsafe SyncNetworkRoute fragment remained after override: {forbidden}"
            ));
        }
    }

    Ok(rendered)
}

fn replace_exactly_once(
    target: &mut String,
    old: &str,
    new: &str,
    label: &str,
) -> Result<(), String> {
    let matches = target.match_indices(old).count();
    if matches != 1 {
        return Err(format!(
            "expected exactly one {label} fragment in SyncNetworkRoute, found {matches}"
        ));
    }
    *target = target.replacen(old, new, 1);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered_route() -> String {
        let base = crate::kotlin_sources::SOURCES
            .iter()
            .find(|source| source.path == PATH)
            .expect("base SyncNetworkRoute source")
            .contents;
        render(base).expect("secure SyncNetworkRoute override")
    }

    #[test]
    fn override_requires_live_proof_before_durable_generation_acceptance() {
        let route = rendered_route();
        let resolver_start = route.find("fun resolveServerUrl(").expect("resolver");
        let resolver_end = route[resolver_start..]
            .find("fun storedServerUrlForAutomaticMode(")
            .map(|offset| resolver_start + offset)
            .expect("bounded resolver");
        let resolver = &route[resolver_start..resolver_end];
        let candidate = resolver
            .find("SyncRemoteRendezvous.candidates(context, log)")
            .expect("context-bound candidates");
        let live_proof = resolver
            .find("verifiedSignedPublicServerUrlOrNull(")
            .expect("signed endpoint live proof");
        let accept = resolver
            .find("SyncRemoteRendezvous.acceptCandidate(context, candidate, log)")
            .expect("durable high-water acceptance");
        let returned = resolver[accept..]
            .find("return Resolution(serverUrl = target")
            .map(|offset| accept + offset)
            .expect("accepted route return");
        assert!(candidate < live_proof && live_proof < accept && accept < returned);
    }

    #[test]
    fn override_fails_closed_for_unsigned_and_health_only_bootstrap_routes() {
        let route = rendered_route();
        assert!(route.contains(
            "Rejected unsigned discovery while no authenticated account proof was available."
        ));
        assert!(route.contains(
            "Rejected the bootstrap endpoint until an authenticated account proof is available."
        ));
        assert!(route.contains("identity = discoveryIdentity"));
        assert!(route.contains("val proofBound = verifiedPublicServerFromDiscoveryProof("));
        assert!(route.contains("if (proofBound != reachable.serverUrl)"));
        assert!(route.contains("if (proofBound != candidate.serverUrl)"));
        assert!(route.contains(
            "Verified a live endpoint announced by the pinned desktop installation identity."
        ));
        assert!(!route.contains("SyncRemoteRendezvous.candidates(log)"));
        assert!(!route.contains("verifiedBootstrapPublicServerUrlOrNull(packaged, log)"));
        assert!(
            !route.contains("return verifiedBootstrapPublicServerUrlOrNull(publicCandidate, log)")
        );
        assert!(!route.contains(
            "Verified a protocol-compatible desktop HTTPS endpoint without sending a bearer token."
        ));
    }
}
