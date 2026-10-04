use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
#[cfg(any(not(target_os = "android"), test))]
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt::Write as FmtWrite;
use std::net::{Ipv4Addr, Ipv6Addr};
use url::{Host, Url};
use zeroize::Zeroizing;

#[cfg(not(target_os = "android"))]
use std::fs::{self, File, OpenOptions};
#[cfg(not(target_os = "android"))]
use std::io::{self, Read, Write};
#[cfg(not(target_os = "android"))]
use std::path::{Path, PathBuf};

pub const RENDEZVOUS_SERVICE_BASE_URL: &str = "https://ntfy.sh";
pub const RENDEZVOUS_PAYLOAD_SCHEMA: &str = "gridtimer.sync.rendezvous.v1";
pub const RENDEZVOUS_ENVELOPE_SCHEMA_VERSION: u32 = 1;
pub const RENDEZVOUS_SYNC_PROTOCOL_VERSION: i64 = crate::sync_core::SYNC_PROTOCOL_VERSION;
pub const RENDEZVOUS_MAX_LIFETIME_SECONDS: u64 = 12 * 60 * 60;
pub const RENDEZVOUS_DEFAULT_CLOCK_SKEW_SECONDS: u64 = 5 * 60;

#[cfg(not(target_os = "android"))]
pub const RENDEZVOUS_IDENTITY_FILE_NAME: &str = "rendezvous_ed25519_identity.pkcs8";

const ED25519_PUBLIC_KEY_LENGTH: usize = 32;
const ED25519_SIGNATURE_LENGTH: usize = 64;
pub const RENDEZVOUS_MAX_SIGNED_PAYLOAD_BYTES: usize = 8 * 1_024;
const MAX_SERVER_URL_LENGTH: usize = 2_048;
const MAX_BUILD_ID_LENGTH: usize = 128;
const MAX_ENVELOPE_JSON_LENGTH: usize = 16 * 1_024;
#[cfg(not(target_os = "android"))]
const MAX_PKCS8_LENGTH: u64 = 4 * 1_024;

/// An unsigned rendezvous announcement. `identity_id` is deliberately filled by
/// `LocalRendezvousIdentity::sign_payload`, so callers cannot accidentally sign
/// an announcement bound to a different installation identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RendezvousPayload {
    pub schema: String,
    #[serde(rename = "syncProtocolVersion")]
    pub sync_protocol_version: i64,
    #[serde(rename = "serverBuildId")]
    pub server_build_id: String,
    #[serde(rename = "identityId")]
    pub identity_id: String,
    #[serde(rename = "serverUrl")]
    pub server_url: String,
    pub generation: u64,
    #[serde(rename = "issuedAt")]
    pub issued_at: u64,
    #[serde(rename = "expiresAt")]
    pub expires_at: u64,
}

impl RendezvousPayload {
    pub fn new(
        server_url: &str,
        server_build_id: &str,
        issued_at_unix: u64,
        expires_at_unix: u64,
    ) -> Result<Self, String> {
        Self::new_with_generation(
            server_url,
            server_build_id,
            issued_at_unix,
            issued_at_unix,
            expires_at_unix,
        )
    }

    pub fn new_with_generation(
        server_url: &str,
        server_build_id: &str,
        generation: u64,
        issued_at_unix: u64,
        expires_at_unix: u64,
    ) -> Result<Self, String> {
        validate_build_id(server_build_id)?;
        validate_time_window(issued_at_unix, expires_at_unix)?;
        if generation == 0 {
            return Err("rendezvous generation must be positive".to_string());
        }
        Ok(Self {
            schema: RENDEZVOUS_PAYLOAD_SCHEMA.to_string(),
            sync_protocol_version: RENDEZVOUS_SYNC_PROTOCOL_VERSION,
            server_build_id: server_build_id.to_string(),
            identity_id: String::new(),
            server_url: validate_https_server_url(server_url)?,
            generation,
            issued_at: issued_at_unix,
            expires_at: expires_at_unix,
        })
    }

    /// Returns the exact UTF-8 JSON representation covered by the signature.
    pub fn canonical_json(&self) -> Result<String, String> {
        validate_payload_structure(self)?;
        canonical_payload_json_unchecked(self)
    }

    fn bound_to_identity(&self, identity_id: &str) -> Result<Self, String> {
        validate_identity_id(identity_id)?;
        let mut bound = self.clone();
        bound.identity_id = identity_id.to_string();
        validate_payload_structure(&bound)?;
        Ok(bound)
    }
}

/// Wire format posted directly as the body of an ntfy topic request. `payload`
/// remains a string so every verifier checks exactly the bytes the server signed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RendezvousEnvelope {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    pub payload: String,
    #[serde(rename = "publicKey")]
    pub public_key: String,
    pub signature: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedRendezvousAnnouncement {
    pub payload: RendezvousPayload,
    pub public_key_base64: String,
    pub topic: String,
}

/// Long-term desktop installation identity. The PKCS#8 material is private and
/// zeroized when this value is dropped.
pub struct LocalRendezvousIdentity {
    pub public_key_base64: String,
    pub identity_id: String,
    pub topic: String,
    private_key_pkcs8: Zeroizing<Vec<u8>>,
}

impl std::fmt::Debug for LocalRendezvousIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalRendezvousIdentity")
            .field("public_key_base64", &self.public_key_base64)
            .field("identity_id", &self.identity_id)
            .field("topic", &self.topic)
            .finish_non_exhaustive()
    }
}

impl LocalRendezvousIdentity {
    pub fn topic(&self) -> &str {
        &self.topic
    }

    pub fn publish_url(&self) -> String {
        format!("{RENDEZVOUS_SERVICE_BASE_URL}/{}", self.topic)
    }

    pub fn subscription_url(&self) -> String {
        format!(
            "{RENDEZVOUS_SERVICE_BASE_URL}/{}/json?poll=1&since=12h",
            self.topic
        )
    }

    pub fn signed_envelope(
        &self,
        payload: &RendezvousPayload,
    ) -> Result<RendezvousEnvelope, String> {
        let payload = payload.bound_to_identity(&self.identity_id)?;
        let canonical_payload = payload.canonical_json()?;
        let key_pair =
            Ed25519KeyPair::from_pkcs8(self.private_key_pkcs8.as_slice()).map_err(|_| {
                "stored rendezvous identity is not a valid Ed25519 PKCS#8 key".to_string()
            })?;
        if key_pair.public_key().as_ref() != decode_public_key_base64(&self.public_key_base64)? {
            return Err(
                "stored rendezvous identity public key does not match its metadata".to_string(),
            );
        }
        let signature = key_pair.sign(canonical_payload.as_bytes());
        Ok(RendezvousEnvelope {
            schema_version: RENDEZVOUS_ENVELOPE_SCHEMA_VERSION,
            payload: canonical_payload,
            public_key: self.public_key_base64.clone(),
            signature: encode_signature_base64(signature.as_ref())?,
        })
    }

    /// Returns a complete JSON envelope suitable for a direct POST body to
    /// `publish_url(identity.topic())`.
    pub fn sign_payload(&self, payload: &RendezvousPayload) -> Result<String, String> {
        serde_json::to_string(&self.signed_envelope(payload)?)
            .map_err(|error| format!("failed to serialize rendezvous envelope: {error}"))
    }

    #[cfg(any(not(target_os = "android"), test))]
    fn from_pkcs8(pkcs8: Vec<u8>) -> Result<Self, String> {
        let private_key_pkcs8 = Zeroizing::new(pkcs8);
        let key_pair = Ed25519KeyPair::from_pkcs8(private_key_pkcs8.as_slice()).map_err(|_| {
            "rendezvous identity file is not a valid Ed25519 PKCS#8 key".to_string()
        })?;
        let public_key_base64 = encode_public_key_base64(key_pair.public_key().as_ref())?;
        let identity_id = identity_id_from_public_key(key_pair.public_key().as_ref())?;
        Ok(Self {
            public_key_base64,
            topic: identity_id.clone(),
            identity_id,
            private_key_pkcs8,
        })
    }
}

/// Loads or atomically creates the long-term identity at
/// `%LOCALAPPDATA%/GridTimerSync/rendezvous_ed25519_identity.pkcs8`.
#[cfg(not(target_os = "android"))]
pub fn ensure_local_rendezvous_identity() -> Result<LocalRendezvousIdentity, String> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            "LOCALAPPDATA is unavailable; cannot persist rendezvous identity".to_string()
        })?;
    ensure_rendezvous_identity_in_directory(&PathBuf::from(local_app_data).join("GridTimerSync"))
}

#[cfg(target_os = "android")]
pub fn ensure_local_rendezvous_identity() -> Result<LocalRendezvousIdentity, String> {
    Err("desktop rendezvous identity persistence is unavailable on Android".to_string())
}

pub fn publish_url(topic: &str) -> Result<String, String> {
    validate_topic(topic)?;
    Ok(format!("{RENDEZVOUS_SERVICE_BASE_URL}/{topic}"))
}

pub fn subscription_url(topic: &str) -> Result<String, String> {
    validate_topic(topic)?;
    Ok(format!(
        "{RENDEZVOUS_SERVICE_BASE_URL}/{topic}/json?poll=1&since=12h"
    ))
}

pub fn encode_public_key_base64(public_key: &[u8]) -> Result<String, String> {
    if public_key.len() != ED25519_PUBLIC_KEY_LENGTH {
        return Err(format!(
            "Ed25519 public key must contain {ED25519_PUBLIC_KEY_LENGTH} bytes"
        ));
    }
    Ok(URL_SAFE_NO_PAD.encode(public_key))
}

pub fn decode_public_key_base64(encoded: &str) -> Result<[u8; ED25519_PUBLIC_KEY_LENGTH], String> {
    decode_canonical_base64::<ED25519_PUBLIC_KEY_LENGTH>(encoded, "Ed25519 public key")
}

pub fn encode_signature_base64(signature: &[u8]) -> Result<String, String> {
    if signature.len() != ED25519_SIGNATURE_LENGTH {
        return Err(format!(
            "Ed25519 signature must contain {ED25519_SIGNATURE_LENGTH} bytes"
        ));
    }
    Ok(URL_SAFE_NO_PAD.encode(signature))
}

pub fn decode_signature_base64(encoded: &str) -> Result<[u8; ED25519_SIGNATURE_LENGTH], String> {
    decode_canonical_base64::<ED25519_SIGNATURE_LENGTH>(encoded, "Ed25519 signature")
}

/// Verifies a bounded UTF-8 payload with canonical URL-safe, unpadded Ed25519
/// material. This narrow helper is shared by the Android JNI boundary.
pub fn verify_ed25519_signature(
    payload: &str,
    public_key_base64: &str,
    signature_base64: &str,
) -> bool {
    if payload.is_empty() || payload.len() > RENDEZVOUS_MAX_SIGNED_PAYLOAD_BYTES {
        return false;
    }
    let Ok(public_key) = decode_public_key_base64(public_key_base64) else {
        return false;
    };
    let Ok(signature) = decode_signature_base64(signature_base64) else {
        return false;
    };
    UnparsedPublicKey::new(&ED25519, public_key)
        .verify(payload.as_bytes(), &signature)
        .is_ok()
}

pub fn identity_id_from_public_key(public_key: &[u8]) -> Result<String, String> {
    if public_key.len() != ED25519_PUBLIC_KEY_LENGTH {
        return Err(format!(
            "Ed25519 public key must contain {ED25519_PUBLIC_KEY_LENGTH} bytes"
        ));
    }
    let digest = Sha256::digest(public_key);
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}")
            .map_err(|_| "failed to encode rendezvous identity hash".to_string())?;
    }
    Ok(encoded)
}

pub fn topic_from_public_key_base64(public_key_base64: &str) -> Result<String, String> {
    identity_id_from_public_key(&decode_public_key_base64(public_key_base64)?)
}

/// Parses the envelope and enforces canonical payload JSON, exact schemas,
/// bounded fields, a safe HTTPS endpoint, and public-key/identity binding.
/// Signature and freshness checks are intentionally separate layers.
pub fn parse_rendezvous_envelope(json: &str) -> Result<RendezvousEnvelope, String> {
    if json.is_empty() || json.len() > MAX_ENVELOPE_JSON_LENGTH {
        return Err("rendezvous envelope JSON has an invalid size".to_string());
    }
    if json.trim() != json {
        return Err("rendezvous envelope JSON must not have surrounding whitespace".to_string());
    }
    let envelope: RendezvousEnvelope = serde_json::from_str(json)
        .map_err(|error| format!("invalid rendezvous envelope JSON: {error}"))?;
    validate_envelope_structure(&envelope)?;
    Ok(envelope)
}

pub fn parse_rendezvous_payload(
    envelope: &RendezvousEnvelope,
) -> Result<RendezvousPayload, String> {
    let payload: RendezvousPayload = serde_json::from_str(&envelope.payload)
        .map_err(|error| format!("invalid rendezvous payload JSON: {error}"))?;
    validate_payload_structure(&payload)?;
    let canonical = canonical_payload_json_unchecked(&payload)?;
    if canonical != envelope.payload {
        return Err("rendezvous payload JSON is not canonical".to_string());
    }
    Ok(payload)
}

pub fn verify_rendezvous_envelope_signature(
    envelope: &RendezvousEnvelope,
) -> Result<RendezvousPayload, String> {
    validate_envelope_structure(envelope)?;
    let payload = parse_rendezvous_payload(envelope)?;
    let public_key = decode_public_key_base64(&envelope.public_key)?;
    let signature = decode_signature_base64(&envelope.signature)?;
    UnparsedPublicKey::new(&ED25519, public_key)
        .verify(envelope.payload.as_bytes(), &signature)
        .map_err(|_| "rendezvous envelope signature verification failed".to_string())?;
    Ok(payload)
}

pub fn validate_rendezvous_payload_at(
    payload: &RendezvousPayload,
    now_unix: u64,
) -> Result<(), String> {
    validate_rendezvous_payload_at_with_skew(
        payload,
        now_unix,
        RENDEZVOUS_DEFAULT_CLOCK_SKEW_SECONDS,
    )
}

pub fn validate_rendezvous_payload_at_with_skew(
    payload: &RendezvousPayload,
    now_unix: u64,
    allowed_clock_skew_seconds: u64,
) -> Result<(), String> {
    validate_payload_structure(payload)?;
    if payload.issued_at > now_unix.saturating_add(allowed_clock_skew_seconds) {
        return Err("rendezvous announcement was issued in the future".to_string());
    }
    if now_unix
        > payload
            .expires_at
            .saturating_add(allowed_clock_skew_seconds)
    {
        return Err("rendezvous announcement has expired".to_string());
    }
    Ok(())
}

pub fn verify_and_parse_rendezvous_envelope(
    json: &str,
    now_unix: u64,
) -> Result<VerifiedRendezvousAnnouncement, String> {
    let envelope = parse_rendezvous_envelope(json)?;
    let payload = verify_rendezvous_envelope_signature(&envelope)?;
    validate_rendezvous_payload_at(&payload, now_unix)?;
    let topic = topic_from_public_key_base64(&envelope.public_key)?;
    Ok(VerifiedRendezvousAnnouncement {
        payload,
        public_key_base64: envelope.public_key,
        topic,
    })
}

/// Full verification with identity pinning for the Android APK. A valid
/// self-signed envelope from any other key is rejected before its URL is used.
pub fn verify_and_parse_rendezvous_envelope_for_public_key(
    json: &str,
    expected_public_key_base64: &str,
    now_unix: u64,
) -> Result<VerifiedRendezvousAnnouncement, String> {
    let expected_key = decode_public_key_base64(expected_public_key_base64)?;
    let verified = verify_and_parse_rendezvous_envelope(json, now_unix)?;
    let actual_key = decode_public_key_base64(&verified.public_key_base64)?;
    if actual_key != expected_key {
        return Err("rendezvous envelope was signed by an unexpected identity".to_string());
    }
    Ok(verified)
}

pub fn validate_https_server_url(server_url: &str) -> Result<String, String> {
    if server_url.is_empty() || server_url.len() > MAX_SERVER_URL_LENGTH {
        return Err("rendezvous server URL has an invalid size".to_string());
    }
    if server_url.trim() != server_url
        || server_url
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
        || server_url.contains('\\')
    {
        return Err("rendezvous server URL contains unsafe characters".to_string());
    }
    let mut parsed = Url::parse(server_url)
        .map_err(|_| "rendezvous server URL is not a valid absolute URL".to_string())?;
    if parsed.scheme() != "https" {
        return Err("rendezvous server URL must use HTTPS".to_string());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("rendezvous server URL must not contain user information".to_string());
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("rendezvous server URL must not contain a query or fragment".to_string());
    }
    let host = parsed
        .host()
        .ok_or_else(|| "rendezvous server URL must contain a public host".to_string())?;
    match host {
        Host::Domain(domain) => validate_public_domain(domain)?,
        Host::Ipv4(address) if is_public_ipv4(address) => {}
        Host::Ipv6(address) if is_public_ipv6(address) => {}
        Host::Ipv4(_) | Host::Ipv6(_) => {
            return Err(
                "rendezvous server URL must not target a private or reserved IP".to_string(),
            )
        }
    }
    if parsed.path().contains("//") {
        return Err("rendezvous server URL path must not contain empty segments".to_string());
    }
    if parsed.path() == "/" {
        parsed.set_path("");
    } else {
        let trimmed_path = parsed.path().trim_end_matches('/').to_string();
        parsed.set_path(&trimmed_path);
    }
    Ok(parsed.to_string().trim_end_matches('/').to_string())
}

fn validate_envelope_structure(envelope: &RendezvousEnvelope) -> Result<(), String> {
    if envelope.schema_version != RENDEZVOUS_ENVELOPE_SCHEMA_VERSION {
        return Err("unsupported rendezvous envelope schema version".to_string());
    }
    let public_key = decode_public_key_base64(&envelope.public_key)?;
    decode_signature_base64(&envelope.signature)?;
    let payload = parse_rendezvous_payload_without_envelope(&envelope.payload)?;
    let expected_identity_id = identity_id_from_public_key(&public_key)?;
    if payload.identity_id != expected_identity_id {
        return Err("rendezvous payload identity is not bound to its public key".to_string());
    }
    Ok(())
}

fn parse_rendezvous_payload_without_envelope(
    payload_json: &str,
) -> Result<RendezvousPayload, String> {
    let payload: RendezvousPayload = serde_json::from_str(payload_json)
        .map_err(|error| format!("invalid rendezvous payload JSON: {error}"))?;
    validate_payload_structure(&payload)?;
    if canonical_payload_json_unchecked(&payload)? != payload_json {
        return Err("rendezvous payload JSON is not canonical".to_string());
    }
    Ok(payload)
}

fn validate_payload_structure(payload: &RendezvousPayload) -> Result<(), String> {
    if payload.schema != RENDEZVOUS_PAYLOAD_SCHEMA {
        return Err("unsupported rendezvous payload schema".to_string());
    }
    if payload.sync_protocol_version != RENDEZVOUS_SYNC_PROTOCOL_VERSION {
        return Err("unsupported rendezvous sync protocol version".to_string());
    }
    validate_build_id(&payload.server_build_id)?;
    validate_identity_id(&payload.identity_id)?;
    if payload.generation == 0 {
        return Err("rendezvous generation must be positive".to_string());
    }
    validate_time_window(payload.issued_at, payload.expires_at)?;
    let normalized_url = validate_https_server_url(&payload.server_url)?;
    if normalized_url != payload.server_url {
        return Err("rendezvous server URL is not canonical".to_string());
    }
    Ok(())
}

fn validate_time_window(issued_at: u64, expires_at: u64) -> Result<(), String> {
    if expires_at <= issued_at {
        return Err("rendezvous expiration must be after issuance".to_string());
    }
    if expires_at - issued_at > RENDEZVOUS_MAX_LIFETIME_SECONDS {
        return Err(format!(
            "rendezvous lifetime must not exceed {RENDEZVOUS_MAX_LIFETIME_SECONDS} seconds"
        ));
    }
    Ok(())
}

fn validate_build_id(server_build_id: &str) -> Result<(), String> {
    if server_build_id.is_empty()
        || server_build_id.len() > MAX_BUILD_ID_LENGTH
        || !server_build_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+'))
    {
        return Err("rendezvous server build ID must be a bounded ASCII identifier".to_string());
    }
    Ok(())
}

fn validate_identity_id(identity_id: &str) -> Result<(), String> {
    if identity_id.len() != 64
        || !identity_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(
            "rendezvous identity ID must be 64 lowercase hexadecimal characters".to_string(),
        );
    }
    Ok(())
}

fn validate_topic(topic: &str) -> Result<(), String> {
    validate_identity_id(topic)
}

fn validate_public_domain(domain: &str) -> Result<(), String> {
    if domain.len() > 253 || domain.ends_with('.') || !domain.contains('.') {
        return Err(
            "rendezvous server URL must contain a fully qualified public domain".to_string(),
        );
    }
    let lower = domain.to_ascii_lowercase();
    const BLOCKED_SUFFIXES: &[&str] = &[
        "localhost",
        ".localhost",
        ".local",
        ".lan",
        ".internal",
        ".home",
        ".home.arpa",
        ".test",
        ".invalid",
        ".example",
        "example.com",
        ".example.com",
        "example.net",
        ".example.net",
        "example.org",
        ".example.org",
        ".onion",
    ];
    if BLOCKED_SUFFIXES
        .iter()
        .any(|suffix| lower == *suffix || lower.ends_with(suffix))
    {
        return Err("rendezvous server URL uses a local or reserved domain".to_string());
    }
    let labels: Vec<&str> = lower.split('.').collect();
    if labels.iter().any(|label| {
        label.is_empty()
            || label.len() > 63
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || label.starts_with('-')
            || label.ends_with('-')
    }) {
        return Err("rendezvous server URL contains an invalid domain name".to_string());
    }
    let top_level = labels.last().copied().unwrap_or_default();
    if !(top_level.len() >= 2
        && (top_level.bytes().all(|byte| byte.is_ascii_alphabetic())
            || top_level.starts_with("xn--")))
    {
        return Err("rendezvous server URL uses an invalid top-level domain".to_string());
    }
    Ok(())
}

fn is_public_ipv4(address: Ipv4Addr) -> bool {
    let [a, b, c, _] = address.octets();
    !matches!(
        (a, b, c),
        (0, _, _)
            | (10, _, _)
            | (100, 64..=127, _)
            | (127, _, _)
            | (169, 254, _)
            | (172, 16..=31, _)
            | (192, 0, 0)
            | (192, 0, 2)
            | (192, 168, _)
            | (198, 18..=19, _)
            | (198, 51, 100)
            | (203, 0, 113)
            | (224..=255, _, _)
    )
}

fn is_public_ipv6(address: Ipv6Addr) -> bool {
    let segments = address.segments();
    if segments[..5] == [0, 0, 0, 0, 0] && segments[5] == 0xffff {
        return is_public_ipv4(Ipv4Addr::new(
            (segments[6] >> 8) as u8,
            segments[6] as u8,
            (segments[7] >> 8) as u8,
            segments[7] as u8,
        ));
    }
    // Strictly accept globally routed unicast only, excluding documentation space.
    (segments[0] & 0xe000) == 0x2000 && !(segments[0] == 0x2001 && segments[1] == 0x0db8)
}

fn canonical_payload_json_unchecked(payload: &RendezvousPayload) -> Result<String, String> {
    serde_json::to_string(payload)
        .map_err(|error| format!("failed to serialize rendezvous payload: {error}"))
}

fn decode_canonical_base64<const LENGTH: usize>(
    encoded: &str,
    label: &str,
) -> Result<[u8; LENGTH], String> {
    if encoded.is_empty() || encoded.contains('=') {
        return Err(format!("{label} must use unpadded URL-safe base64"));
    }
    let decoded = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| format!("{label} is not valid URL-safe base64"))?;
    let bytes: [u8; LENGTH] = decoded
        .try_into()
        .map_err(|_| format!("{label} has an invalid decoded length"))?;
    if URL_SAFE_NO_PAD.encode(bytes) != encoded {
        return Err(format!("{label} is not canonically encoded"));
    }
    Ok(bytes)
}

#[cfg(not(target_os = "android"))]
fn ensure_rendezvous_identity_in_directory(
    directory: &Path,
) -> Result<LocalRendezvousIdentity, String> {
    fs::create_dir_all(directory)
        .map_err(|error| format!("failed to create rendezvous identity directory: {error}"))?;
    let metadata = fs::symlink_metadata(directory)
        .map_err(|error| format!("failed to inspect rendezvous identity directory: {error}"))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err("rendezvous identity directory is not a regular directory".to_string());
    }

    let identity_path = directory.join(RENDEZVOUS_IDENTITY_FILE_NAME);
    if let Some(identity) = load_rendezvous_identity(&identity_path)? {
        return Ok(identity);
    }

    let generated = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
        .map_err(|_| "failed to generate Ed25519 rendezvous identity".to_string())?;
    let temporary_path = unique_identity_temp_path(directory)?;
    let mut temporary = create_secret_file(&temporary_path)
        .map_err(|error| format!("failed to create rendezvous identity temporary file: {error}"))?;
    let write_result = (|| -> io::Result<()> {
        temporary.write_all(generated.as_ref())?;
        temporary.sync_all()
    })();
    drop(temporary);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary_path);
        return Err(format!(
            "failed to persist rendezvous identity temporary file: {error}"
        ));
    }

    match fs::hard_link(&temporary_path, &identity_path) {
        Ok(()) => {
            let _ = fs::remove_file(&temporary_path);
            sync_identity_directory(directory)?;
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let _ = fs::remove_file(&temporary_path);
        }
        Err(error) => {
            let _ = fs::remove_file(&temporary_path);
            return Err(format!(
                "failed to atomically publish rendezvous identity: {error}"
            ));
        }
    }

    load_rendezvous_identity(&identity_path)?
        .ok_or_else(|| "rendezvous identity disappeared after atomic publication".to_string())
}

#[cfg(not(target_os = "android"))]
fn load_rendezvous_identity(path: &Path) -> Result<Option<LocalRendezvousIdentity>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "failed to inspect rendezvous identity file: {error}"
            ))
        }
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err("rendezvous identity path is not a regular file".to_string());
    }
    if metadata.len() == 0 || metadata.len() > MAX_PKCS8_LENGTH {
        return Err("rendezvous identity PKCS#8 file has an invalid size".to_string());
    }
    let file = File::open(path)
        .map_err(|error| format!("failed to open rendezvous identity file: {error}"))?;
    let mut pkcs8 = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_PKCS8_LENGTH + 1)
        .read_to_end(&mut pkcs8)
        .map_err(|error| format!("failed to read rendezvous identity file: {error}"))?;
    if pkcs8.len() as u64 > MAX_PKCS8_LENGTH {
        return Err("rendezvous identity PKCS#8 file is too large".to_string());
    }
    LocalRendezvousIdentity::from_pkcs8(pkcs8).map(Some)
}

#[cfg(not(target_os = "android"))]
fn unique_identity_temp_path(directory: &Path) -> Result<PathBuf, String> {
    let random = SystemRandom::new();
    for _ in 0..16 {
        let mut nonce = [0_u8; 16];
        random
            .fill(&mut nonce)
            .map_err(|_| "failed to generate rendezvous identity temporary name".to_string())?;
        let name = format!(
            ".{RENDEZVOUS_IDENTITY_FILE_NAME}.{}.{}.tmp",
            std::process::id(),
            hex_bytes(&nonce)
        );
        let path = directory.join(name);
        if !path.exists() {
            return Ok(path);
        }
    }
    Err("failed to allocate a unique rendezvous identity temporary path".to_string())
}

#[cfg(not(target_os = "android"))]
fn create_secret_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

#[cfg(all(not(target_os = "android"), unix))]
fn sync_identity_directory(directory: &Path) -> Result<(), String> {
    File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(|error| format!("failed to sync rendezvous identity directory: {error}"))
}

#[cfg(all(not(target_os = "android"), not(unix)))]
fn sync_identity_directory(_directory: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(not(target_os = "android"))]
fn hex_bytes(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000;

    fn generated_identity() -> LocalRendezvousIdentity {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        LocalRendezvousIdentity::from_pkcs8(pkcs8.as_ref().to_vec()).unwrap()
    }

    fn sample_payload() -> RendezvousPayload {
        RendezvousPayload::new_with_generation(
            "https://tunnel.trycloudflare.com/",
            "v2.22.5-rendezvous",
            NOW + 7,
            NOW,
            NOW + 6 * 60 * 60,
        )
        .unwrap()
    }

    #[test]
    fn canonical_payload_has_exact_schema_fields_and_order() {
        let identity = generated_identity();
        let envelope = identity.signed_envelope(&sample_payload()).unwrap();
        let expected = format!(
            "{{\"schema\":\"gridtimer.sync.rendezvous.v1\",\"syncProtocolVersion\":1,\"serverBuildId\":\"v2.22.5-rendezvous\",\"identityId\":\"{}\",\"serverUrl\":\"https://tunnel.trycloudflare.com\",\"generation\":{},\"issuedAt\":{},\"expiresAt\":{}}}",
            identity.identity_id,
            NOW + 7,
            NOW,
            NOW + 6 * 60 * 60
        );
        assert_eq!(expected, envelope.payload);
        assert!(!envelope.payload.contains("account"));
        assert!(!envelope.payload.contains("token"));
        assert!(!envelope.payload.contains("userId"));
    }

    #[test]
    fn signature_and_base64_round_trip_are_strict() {
        let identity = generated_identity();
        let json = identity.sign_payload(&sample_payload()).unwrap();
        let verified = verify_and_parse_rendezvous_envelope_for_public_key(
            &json,
            &identity.public_key_base64,
            NOW + 60,
        )
        .unwrap();

        assert_eq!(identity.identity_id, verified.payload.identity_id);
        assert_eq!(identity.topic, verified.topic);
        assert_eq!(43, identity.public_key_base64.len());
        assert!(!identity.public_key_base64.contains('='));
        let parsed = parse_rendezvous_envelope(&json).unwrap();
        assert_eq!(86, parsed.signature.len());
        assert!(!parsed.signature.contains('='));
        assert!(verify_ed25519_signature(
            &parsed.payload,
            &parsed.public_key,
            &parsed.signature,
        ));
        assert!(!verify_ed25519_signature(
            &(parsed.payload.clone() + " "),
            &parsed.public_key,
            &parsed.signature,
        ));
        assert!(!verify_ed25519_signature(
            &"x".repeat(RENDEZVOUS_MAX_SIGNED_PAYLOAD_BYTES + 1),
            &parsed.public_key,
            &parsed.signature,
        ));

        let mut tampered: RendezvousEnvelope = serde_json::from_str(&json).unwrap();
        tampered.payload = tampered.payload.replace("tunnel", "attacker");
        let tampered_json = serde_json::to_string(&tampered).unwrap();
        assert!(verify_and_parse_rendezvous_envelope(&tampered_json, NOW).is_err());
        assert!(decode_public_key_base64(&(identity.public_key_base64.clone() + "=")).is_err());
    }

    #[test]
    fn pinned_verifier_rejects_another_valid_identity() {
        let intended = generated_identity();
        let attacker = generated_identity();
        let attacker_json = attacker.sign_payload(&sample_payload()).unwrap();

        assert!(verify_and_parse_rendezvous_envelope(&attacker_json, NOW).is_ok());
        assert!(verify_and_parse_rendezvous_envelope_for_public_key(
            &attacker_json,
            &intended.public_key_base64,
            NOW,
        )
        .is_err());
    }

    #[test]
    fn time_validation_handles_fresh_future_expired_and_oversized_windows() {
        let identity = generated_identity();
        let json = identity.sign_payload(&sample_payload()).unwrap();
        assert!(verify_and_parse_rendezvous_envelope(&json, NOW + 60).is_ok());
        assert!(verify_and_parse_rendezvous_envelope(
            &json,
            NOW + 6 * 60 * 60 + RENDEZVOUS_DEFAULT_CLOCK_SKEW_SECONDS + 1,
        )
        .is_err());
        assert!(verify_and_parse_rendezvous_envelope(
            &json,
            NOW - RENDEZVOUS_DEFAULT_CLOCK_SKEW_SECONDS - 1,
        )
        .is_err());
        assert!(RendezvousPayload::new(
            "https://sync.example.dev",
            "build-1",
            NOW,
            NOW + RENDEZVOUS_MAX_LIFETIME_SECONDS + 1,
        )
        .is_err());
    }

    #[test]
    fn https_validator_rejects_credential_local_and_ambiguous_urls() {
        assert_eq!(
            "https://sync.example.dev/api",
            validate_https_server_url("https://SYNC.EXAMPLE.dev/api/").unwrap()
        );
        assert_eq!(
            "https://8.8.8.8:8443",
            validate_https_server_url("https://8.8.8.8:8443/").unwrap()
        );

        for unsafe_url in [
            "http://sync.example.dev",
            "https://user:secret@sync.example.dev",
            "https://127.0.0.1",
            "https://192.168.1.8",
            "https://100.64.0.1",
            "https://[::1]",
            "https://[2001:db8::1]",
            "https://server.local",
            "https://example.com",
            "https://sync.example.dev?redirect=https://evil.dev",
            "https://sync.example.dev/#fragment",
            " https://sync.example.dev",
            "https://sync.example.dev\\path",
        ] {
            assert!(
                validate_https_server_url(unsafe_url).is_err(),
                "unexpectedly accepted {unsafe_url}"
            );
        }
    }

    #[test]
    fn topics_are_full_lowercase_sha256_and_urls_are_fixed() {
        let identity = generated_identity();
        assert_eq!(64, identity.topic().len());
        assert!(identity
            .topic()
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(
            format!("https://ntfy.sh/{}", identity.topic()),
            publish_url(identity.topic()).unwrap()
        );
        assert_eq!(
            format!("https://ntfy.sh/{}/json?poll=1&since=12h", identity.topic()),
            subscription_url(identity.topic()).unwrap()
        );
        assert!(publish_url(&identity.topic.to_ascii_uppercase()).is_err());
        assert!(publish_url("short").is_err());
    }

    #[test]
    fn unknown_payload_fields_cannot_smuggle_account_data() {
        let identity = generated_identity();
        let envelope = identity.signed_envelope(&sample_payload()).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&envelope.payload).unwrap();
        value["accountToken"] = serde_json::Value::String("secret".to_string());
        assert!(serde_json::from_value::<RendezvousPayload>(value).is_err());
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn persisted_pkcs8_identity_is_stable_and_valid() {
        let directory = test_directory("stable");
        let first = ensure_rendezvous_identity_in_directory(&directory).unwrap();
        let second = ensure_rendezvous_identity_in_directory(&directory).unwrap();
        let key_bytes = fs::read(directory.join(RENDEZVOUS_IDENTITY_FILE_NAME)).unwrap();

        assert_eq!(first.public_key_base64, second.public_key_base64);
        assert_eq!(first.identity_id, second.identity_id);
        assert!(Ed25519KeyPair::from_pkcs8(&key_bytes).is_ok());
        assert!(fs::read_dir(&directory).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn concurrent_first_use_publishes_one_identity_atomically() {
        let directory = std::sync::Arc::new(test_directory("concurrent"));
        let mut workers = Vec::new();
        for _ in 0..8 {
            let directory = directory.clone();
            workers.push(std::thread::spawn(move || {
                ensure_rendezvous_identity_in_directory(&directory)
                    .unwrap()
                    .identity_id
            }));
        }
        let identities: Vec<String> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();

        assert!(identities.iter().all(|identity| identity == &identities[0]));
        fs::remove_dir_all(directory.as_ref()).unwrap();
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn corrupt_identity_is_rejected_instead_of_silently_rotated() {
        let directory = test_directory("corrupt");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join(RENDEZVOUS_IDENTITY_FILE_NAME), b"not-pkcs8").unwrap();

        assert!(ensure_rendezvous_identity_in_directory(&directory).is_err());
        assert_eq!(
            b"not-pkcs8",
            fs::read(directory.join(RENDEZVOUS_IDENTITY_FILE_NAME))
                .unwrap()
                .as_slice()
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(not(target_os = "android"))]
    fn test_directory(label: &str) -> PathBuf {
        let mut nonce = [0_u8; 16];
        SystemRandom::new().fill(&mut nonce).unwrap();
        std::env::temp_dir().join(format!(
            "gridtimer-rendezvous-test-{label}-{}-{}",
            std::process::id(),
            hex_bytes(&nonce)
        ))
    }
}
