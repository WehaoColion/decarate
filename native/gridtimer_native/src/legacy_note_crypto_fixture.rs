// Frozen pre-declaration crypto implementation; test-only compatibility fixture.
use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD};
use base64::Engine;
use rand::rngs::OsRng;
use rand::RngCore;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use zeroize::{Zeroize, Zeroizing};

use crate::rich_text_security::sanitize_note_document_value;

const FORMAT_VERSION: u32 = 1;
const PAYLOAD_FORMAT_VERSION: u32 = 1;
const CIPHER_SUITE: &str = "AES-256-GCM";
const KDF_NAME: &str = "Argon2id";
const KDF_MEMORY_KIB: u32 = 32 * 1024;
const KDF_ITERATIONS: u32 = 3;
const KDF_PARALLELISM: u32 = 1;
const DATA_KEY_BYTES: usize = 32;
const SALT_BYTES: usize = 16;
const NONCE_BYTES: usize = 12;
const KEY_ID_BYTES: usize = 16;
const SESSION_TOKEN_BYTES: usize = 32;
const MAX_PASSWORD_BYTES: usize = 1024;
const MIN_NEW_PASSWORD_CHARS: usize = 8;
const MAX_NEW_PASSWORD_CHARS: usize = 128;
const MAX_NOTE_JSON_BYTES: usize = 64 * 1024 * 1024;
const MAX_SESSIONS: usize = 64;

type CryptoResult<T> = Result<T, CryptoError>;

#[derive(Debug)]
enum CryptoError {
    InvalidInput,
    UnsupportedEnvelope,
    AuthenticationFailed,
    RandomFailed,
    SessionUnavailable,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NoteEncryptionEnvelope {
    #[serde(default)]
    format_version: u32,
    key_id: String,
    protection_revision: u64,
    #[serde(default)]
    cipher_suite: String,
    #[serde(default)]
    kdf: String,
    #[serde(rename = "memoryKiB")]
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
    salt_base64: String,
    key_nonce_base64: String,
    wrapped_key_base64: String,
    content_nonce_base64: String,
    ciphertext_base64: String,
}

impl NoteEncryptionEnvelope {
    fn repair_legacy_omitted_metadata(&mut self) {
        // The first encrypted-note candidate encoded Kotlin default values with
        // encodeDefaults=false. The app-data bridge then materialized the three
        // omitted fixed identifiers as 0/empty strings. Recover only that exact
        // triplet; partial or explicitly unsupported metadata remains invalid.
        if self.format_version == 0 && self.cipher_suite.is_empty() && self.kdf.is_empty() {
            self.format_version = FORMAT_VERSION;
            self.cipher_suite = CIPHER_SUITE.to_string();
            self.kdf = KDF_NAME.to_string();
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProtectedNotePayload {
    format_version: u32,
    note_id: String,
    title: Value,
    content: Value,
    document: Value,
    attachments: Value,
    revisions: Value,
    #[serde(default = "empty_array_value")]
    versions: Value,
    #[serde(default = "empty_string_value")]
    latest_version_id: Value,
}

struct NoteSession {
    note_id: String,
    data_key: [u8; DATA_KEY_BYTES],
    envelope: NoteEncryptionEnvelope,
    cached_payload_digest: [u8; 32],
    pending_password_change: Option<PendingPasswordChange>,
    sequence: u64,
}

struct PendingPasswordChange {
    source_envelope: NoteEncryptionEnvelope,
    candidate_envelope: NoteEncryptionEnvelope,
}

impl Drop for NoteSession {
    fn drop(&mut self) {
        self.data_key.zeroize();
        self.cached_payload_digest.zeroize();
    }
}

static NOTE_SESSIONS: OnceLock<Mutex<HashMap<String, NoteSession>>> = OnceLock::new();
static SESSION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn sessions() -> &'static Mutex<HashMap<String, NoteSession>> {
    NOTE_SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn encrypt_note(note_json: &str, password: &str) -> Option<(String, String)> {
    encrypt_note_impl(note_json, password).ok()
}

pub fn unlock_note(note_json: &str, password: &str) -> Option<(String, String)> {
    unlock_note_impl(note_json, password).ok()
}

pub fn seal_note(note_json: &str, session_token: &str) -> Option<String> {
    seal_note_impl(note_json, session_token).ok()
}

pub fn create_and_seal_note_version(
    app_data_json: &str,
    note_json: &str,
    session_token: &str,
    source_version_id: &str,
    expected_latest_version_id: &str,
    request_id: &str,
    now: i64,
) -> Option<String> {
    let next_note = crate::app_data::create_unlocked_note_version_json(
        app_data_json,
        note_json,
        source_version_id,
        expected_latest_version_id,
        request_id,
        now,
    )?;
    seal_note_impl(&next_note, session_token).ok()
}

pub fn change_password(note_json: &str, session_token: &str, new_password: &str) -> Option<String> {
    change_password_impl(note_json, session_token, new_password).ok()
}

/// Commits a password-change candidate after the caller has durably persisted it.
///
/// A successful commit revokes every in-memory session for the same note. The
/// note deliberately remains locked; callers must unlock it again with the new
/// password when editing resumes.
pub fn commit_password_change(note_json: &str, session_token: &str) -> bool {
    commit_password_change_impl(note_json, session_token).is_ok()
}

/// Discards one exact password-change candidate after persistence fails.
/// The current session and every other old-generation session remain usable.
pub fn abort_password_change(note_json: &str, session_token: &str) -> bool {
    abort_password_change_impl(note_json, session_token).is_ok()
}

pub fn close_session(session_token: &str) -> bool {
    if session_token.is_empty() {
        return false;
    }
    lock_sessions().remove(session_token).is_some()
}

fn encrypt_note_impl(note_json: &str, password: &str) -> CryptoResult<(String, String)> {
    validate_new_password(password)?;
    let mut note = parse_note(note_json)?;
    if note.get("encryption").is_some_and(|value| !value.is_null()) {
        return Err(CryptoError::InvalidInput);
    }
    let mut payload = protected_payload_from_note(&note)?;
    sanitize_payload_rich_text(&mut payload);
    let payload_bytes = serialize_payload(&payload)?;
    let payload_digest = digest(&payload_bytes);
    let note_id = payload.note_id.clone();

    let data_key = Zeroizing::new(random_array::<DATA_KEY_BYTES>()?);
    let key_id = random_id::<KEY_ID_BYTES>()?;
    let protection_revision = plaintext_protection_state_revision(&note)?
        .checked_add(1)
        .ok_or(CryptoError::InvalidInput)?;
    let envelope = create_envelope(
        &note_id,
        &key_id,
        protection_revision,
        &data_key,
        password,
        &payload_bytes,
    )?;
    let sealed_note = seal_note_value(&mut note, &envelope)?;
    let token = insert_session(NoteSession {
        note_id,
        data_key: *data_key,
        envelope,
        cached_payload_digest: payload_digest,
        pending_password_change: None,
        sequence: next_session_sequence(),
    })?;
    Ok((sealed_note, token))
}

fn unlock_note_impl(note_json: &str, password: &str) -> CryptoResult<(String, String)> {
    validate_password(password)?;
    let mut note = parse_note(note_json)?;
    let note_id = note_id(&note)?.to_string();
    let envelope = envelope_from_note(&note)?;
    validate_envelope(&envelope)?;
    validate_note_protection_state(&note, envelope.protection_revision)?;

    let data_key = Zeroizing::new(unwrap_data_key(&note_id, &envelope, password)?);
    let payload_bytes = decrypt_payload(&note_id, &envelope, &data_key)?;
    let mut payload = parse_payload(&payload_bytes, &note_id)?;
    let payload_digest = digest(&payload_bytes);
    sanitize_payload_rich_text(&mut payload);
    restore_protected_payload(&mut note, &payload)?;
    note.as_object_mut()
        .ok_or(CryptoError::InvalidInput)?
        .insert(
            "encryption".to_string(),
            serde_json::to_value(&envelope).map_err(|_| CryptoError::InvalidInput)?,
        );
    set_protection_state_revision(&mut note, envelope.protection_revision)?;
    let decrypted_note = serde_json::to_string(&note).map_err(|_| CryptoError::InvalidInput)?;
    let token = insert_session(NoteSession {
        note_id,
        data_key: *data_key,
        envelope,
        cached_payload_digest: payload_digest,
        pending_password_change: None,
        sequence: next_session_sequence(),
    })?;
    Ok((decrypted_note, token))
}

fn seal_note_impl(note_json: &str, session_token: &str) -> CryptoResult<String> {
    let mut note = parse_note(note_json)?;
    let mut payload = protected_payload_from_note(&note)?;
    sanitize_payload_rich_text(&mut payload);
    let payload_bytes = serialize_payload(&payload)?;
    let payload_digest = digest(&payload_bytes);

    let mut store = lock_sessions();
    let session = store
        .get_mut(session_token)
        .ok_or(CryptoError::SessionUnavailable)?;
    if session.pending_password_change.is_some() {
        return Err(CryptoError::SessionUnavailable);
    }
    if payload.note_id != session.note_id {
        return Err(CryptoError::AuthenticationFailed);
    }
    validate_note_envelope_for_session(&note, session)?;

    let envelope = if payload_digest == session.cached_payload_digest {
        session.envelope.clone()
    } else {
        let mut next = session.envelope.clone();
        let content_nonce = random_array::<NONCE_BYTES>()?;
        let content_aad = content_aad(&session.note_id, &next.key_id, next.protection_revision);
        let ciphertext = encrypt_aead(
            &session.data_key,
            content_nonce,
            &content_aad,
            &payload_bytes,
        )?;
        next.content_nonce_base64 = encode(&content_nonce);
        next.ciphertext_base64 = encode(&ciphertext);
        session.envelope = next.clone();
        session.cached_payload_digest = payload_digest;
        next
    };
    seal_note_value(&mut note, &envelope)
}

fn change_password_impl(
    note_json: &str,
    session_token: &str,
    new_password: &str,
) -> CryptoResult<String> {
    validate_new_password(new_password)?;
    let mut note = parse_note(note_json)?;
    let mut payload = protected_payload_from_note(&note)?;
    sanitize_payload_rich_text(&mut payload);
    let payload_bytes = serialize_payload(&payload)?;

    let (expected_envelope, expected_cached_payload_digest) = {
        let store = lock_sessions();
        let session = store
            .get(session_token)
            .ok_or(CryptoError::SessionUnavailable)?;
        if session.pending_password_change.is_some() {
            return Err(CryptoError::SessionUnavailable);
        }
        if payload.note_id != session.note_id {
            return Err(CryptoError::AuthenticationFailed);
        }
        validate_note_envelope_for_session(&note, session)?;
        (session.envelope.clone(), session.cached_payload_digest)
    };

    let next_revision = expected_envelope
        .protection_revision
        .checked_add(1)
        .ok_or(CryptoError::InvalidInput)?;
    let next_data_key = Zeroizing::new(random_array::<DATA_KEY_BYTES>()?);
    let next_key_id = random_id::<KEY_ID_BYTES>()?;
    let next_envelope = create_envelope(
        &payload.note_id,
        &next_key_id,
        next_revision,
        &next_data_key,
        new_password,
        &payload_bytes,
    )?;

    set_protection_state_revision(&mut note, next_revision)?;
    let candidate = seal_note_value(&mut note, &next_envelope)?;

    {
        let mut store = lock_sessions();
        let session = store
            .get_mut(session_token)
            .ok_or(CryptoError::SessionUnavailable)?;
        if session.note_id != payload.note_id
            || session.envelope != expected_envelope
            || session.cached_payload_digest != expected_cached_payload_digest
            || session.pending_password_change.is_some()
        {
            return Err(CryptoError::SessionUnavailable);
        }
        session.pending_password_change = Some(PendingPasswordChange {
            source_envelope: expected_envelope,
            candidate_envelope: next_envelope,
        });
    }
    Ok(candidate)
}

fn commit_password_change_impl(note_json: &str, session_token: &str) -> CryptoResult<()> {
    if session_token.is_empty() {
        return Err(CryptoError::SessionUnavailable);
    }
    let note = parse_note(note_json)?;
    let note_id = note_id(&note)?.to_string();
    let candidate_envelope = envelope_from_note(&note)?;
    validate_envelope(&candidate_envelope)?;
    validate_note_protection_state(&note, candidate_envelope.protection_revision)?;

    let mut store = lock_sessions();
    {
        let session = store
            .get(session_token)
            .ok_or(CryptoError::SessionUnavailable)?;
        let pending = session
            .pending_password_change
            .as_ref()
            .ok_or(CryptoError::SessionUnavailable)?;
        if session.note_id != note_id
            || session.envelope != pending.source_envelope
            || pending.candidate_envelope != candidate_envelope
        {
            return Err(CryptoError::AuthenticationFailed);
        }
    }

    let candidate_key_id = candidate_envelope.key_id.as_str();
    let candidate_revision = candidate_envelope.protection_revision;
    if store.values().any(|session| {
        session.note_id == note_id
            && (session.envelope.protection_revision > candidate_revision
                || (session.envelope.protection_revision == candidate_revision
                    && session.envelope.key_id != candidate_key_id))
    }) {
        return Err(CryptoError::SessionUnavailable);
    }

    // HashMap::retain drops removed NoteSession values before returning. Its
    // Drop implementation zeroizes both the data key and cached payload digest.
    store.retain(|_, session| session.note_id != note_id);
    Ok(())
}

fn abort_password_change_impl(note_json: &str, session_token: &str) -> CryptoResult<()> {
    if session_token.is_empty() {
        return Err(CryptoError::SessionUnavailable);
    }
    let note = parse_note(note_json)?;
    let note_id = note_id(&note)?;
    let candidate_envelope = envelope_from_note(&note)?;
    validate_envelope(&candidate_envelope)?;
    validate_note_protection_state(&note, candidate_envelope.protection_revision)?;

    let mut store = lock_sessions();
    let session = store
        .get_mut(session_token)
        .ok_or(CryptoError::SessionUnavailable)?;
    let pending = session
        .pending_password_change
        .as_ref()
        .ok_or(CryptoError::SessionUnavailable)?;
    if session.note_id != note_id || pending.candidate_envelope != candidate_envelope {
        return Err(CryptoError::AuthenticationFailed);
    }
    session.pending_password_change = None;
    Ok(())
}

fn create_envelope(
    note_id: &str,
    key_id: &str,
    protection_revision: u64,
    data_key: &[u8; DATA_KEY_BYTES],
    password: &str,
    payload_bytes: &[u8],
) -> CryptoResult<NoteEncryptionEnvelope> {
    let salt = random_array::<SALT_BYTES>()?;
    let key_nonce = random_array::<NONCE_BYTES>()?;
    let content_nonce = random_array::<NONCE_BYTES>()?;
    let wrapping_key = Zeroizing::new(derive_wrapping_key(password, &salt)?);
    let wrapped_key = encrypt_aead(
        &wrapping_key,
        key_nonce,
        &key_aad(note_id, key_id, protection_revision),
        data_key,
    )?;
    let ciphertext = encrypt_aead(
        data_key,
        content_nonce,
        &content_aad(note_id, key_id, protection_revision),
        payload_bytes,
    )?;
    Ok(NoteEncryptionEnvelope {
        format_version: FORMAT_VERSION,
        key_id: key_id.to_string(),
        protection_revision,
        cipher_suite: CIPHER_SUITE.to_string(),
        kdf: KDF_NAME.to_string(),
        memory_kib: KDF_MEMORY_KIB,
        iterations: KDF_ITERATIONS,
        parallelism: KDF_PARALLELISM,
        salt_base64: encode(&salt),
        key_nonce_base64: encode(&key_nonce),
        wrapped_key_base64: encode(&wrapped_key),
        content_nonce_base64: encode(&content_nonce),
        ciphertext_base64: encode(&ciphertext),
    })
}

fn unwrap_data_key(
    note_id: &str,
    envelope: &NoteEncryptionEnvelope,
    password: &str,
) -> CryptoResult<[u8; DATA_KEY_BYTES]> {
    let salt = decode_array::<SALT_BYTES>(&envelope.salt_base64)?;
    let key_nonce = decode_array::<NONCE_BYTES>(&envelope.key_nonce_base64)?;
    let wrapped_key = decode(&envelope.wrapped_key_base64)?;
    let wrapping_key = Zeroizing::new(derive_wrapping_key(password, &salt)?);
    let mut raw_key = decrypt_aead(
        &wrapping_key,
        key_nonce,
        &key_aad(note_id, &envelope.key_id, envelope.protection_revision),
        &wrapped_key,
    )?;
    if raw_key.len() != DATA_KEY_BYTES {
        raw_key.zeroize();
        return Err(CryptoError::AuthenticationFailed);
    }
    let mut data_key = [0_u8; DATA_KEY_BYTES];
    data_key.copy_from_slice(&raw_key);
    raw_key.zeroize();
    Ok(data_key)
}

fn decrypt_payload(
    note_id: &str,
    envelope: &NoteEncryptionEnvelope,
    data_key: &[u8; DATA_KEY_BYTES],
) -> CryptoResult<Vec<u8>> {
    let nonce = decode_array::<NONCE_BYTES>(&envelope.content_nonce_base64)?;
    let ciphertext = decode(&envelope.ciphertext_base64)?;
    decrypt_aead(
        data_key,
        nonce,
        &content_aad(note_id, &envelope.key_id, envelope.protection_revision),
        &ciphertext,
    )
}

fn encrypt_aead(
    key_bytes: &[u8; DATA_KEY_BYTES],
    nonce_bytes: [u8; NONCE_BYTES],
    aad: &[u8],
    plaintext: &[u8],
) -> CryptoResult<Vec<u8>> {
    let unbound =
        UnboundKey::new(&AES_256_GCM, key_bytes).map_err(|_| CryptoError::InvalidInput)?;
    let key = LessSafeKey::new(unbound);
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);
    let mut output = plaintext.to_vec();
    key.seal_in_place_append_tag(nonce, Aad::from(aad), &mut output)
        .map_err(|_| CryptoError::AuthenticationFailed)?;
    Ok(output)
}

fn decrypt_aead(
    key_bytes: &[u8; DATA_KEY_BYTES],
    nonce_bytes: [u8; NONCE_BYTES],
    aad: &[u8],
    ciphertext: &[u8],
) -> CryptoResult<Vec<u8>> {
    let unbound =
        UnboundKey::new(&AES_256_GCM, key_bytes).map_err(|_| CryptoError::InvalidInput)?;
    let key = LessSafeKey::new(unbound);
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);
    let mut output = ciphertext.to_vec();
    let plaintext_length = key
        .open_in_place(nonce, Aad::from(aad), &mut output)
        .map_err(|_| CryptoError::AuthenticationFailed)?
        .len();
    output.truncate(plaintext_length);
    Ok(output)
}

fn derive_wrapping_key(password: &str, salt: &[u8; SALT_BYTES]) -> CryptoResult<[u8; 32]> {
    let params = Params::new(
        KDF_MEMORY_KIB,
        KDF_ITERATIONS,
        KDF_PARALLELISM,
        Some(DATA_KEY_BYTES),
    )
    .map_err(|_| CryptoError::InvalidInput)?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut output = [0_u8; DATA_KEY_BYTES];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut output)
        .map_err(|_| CryptoError::AuthenticationFailed)?;
    Ok(output)
}

fn validate_password(password: &str) -> CryptoResult<()> {
    if password.is_empty() || password.len() > MAX_PASSWORD_BYTES {
        return Err(CryptoError::InvalidInput);
    }
    Ok(())
}

fn validate_new_password(password: &str) -> CryptoResult<()> {
    validate_password(password)?;
    if !(MIN_NEW_PASSWORD_CHARS..=MAX_NEW_PASSWORD_CHARS).contains(&password.chars().count()) {
        return Err(CryptoError::InvalidInput);
    }
    Ok(())
}

fn parse_note(note_json: &str) -> CryptoResult<Value> {
    if note_json.is_empty() || note_json.len() > MAX_NOTE_JSON_BYTES {
        return Err(CryptoError::InvalidInput);
    }
    let note: Value = serde_json::from_str(note_json).map_err(|_| CryptoError::InvalidInput)?;
    if !note.is_object() {
        return Err(CryptoError::InvalidInput);
    }
    note_id(&note)?;
    Ok(note)
}

fn note_id(note: &Value) -> CryptoResult<&str> {
    let id = note
        .get("id")
        .and_then(Value::as_str)
        .ok_or(CryptoError::InvalidInput)?;
    if id.is_empty() || id.len() > 1024 {
        return Err(CryptoError::InvalidInput);
    }
    Ok(id)
}

fn explicit_protection_state_revision(note: &Value) -> CryptoResult<Option<u64>> {
    let Some(value) = note.get("protectionStateRevision") else {
        return Ok(None);
    };
    let revision = value.as_u64().ok_or(CryptoError::InvalidInput)?;
    if revision > i64::MAX as u64 {
        return Err(CryptoError::InvalidInput);
    }
    Ok(Some(revision))
}

fn plaintext_protection_state_revision(note: &Value) -> CryptoResult<u64> {
    Ok(explicit_protection_state_revision(note)?.unwrap_or(0))
}

fn validate_note_protection_state(note: &Value, expected_revision: u64) -> CryptoResult<()> {
    match explicit_protection_state_revision(note)? {
        // Missing/zero is the compatibility representation of legacy encrypted
        // notes. Every successful crypto output materializes the canonical value.
        None | Some(0) => Ok(()),
        Some(revision) if revision == expected_revision => Ok(()),
        Some(_) => Err(CryptoError::AuthenticationFailed),
    }
}

fn set_protection_state_revision(note: &mut Value, revision: u64) -> CryptoResult<()> {
    if revision == 0 || revision > i64::MAX as u64 {
        return Err(CryptoError::InvalidInput);
    }
    note.as_object_mut()
        .ok_or(CryptoError::InvalidInput)?
        .insert("protectionStateRevision".to_string(), json!(revision));
    Ok(())
}

fn protected_payload_from_note(note: &Value) -> CryptoResult<ProtectedNotePayload> {
    let mut normalized_note = note.clone();
    bootstrap_legacy_version_stack(&mut normalized_note)?;
    synchronize_latest_version_from_note(&mut normalized_note)?;
    let note = &normalized_note;
    let note_id = note_id(note)?.to_string();
    let title = field_or_default(note, "title", Value::String(String::new()));
    let content = field_or_default(note, "content", Value::String(String::new()));
    let document = field_or_default(note, "document", empty_document());
    let attachments = field_or_default(note, "attachments", Value::Array(Vec::new()));
    let revisions = field_or_default(note, "revisions", Value::Array(Vec::new()));
    let versions = field_or_default(note, "versions", Value::Array(Vec::new()));
    let latest_version_id = field_or_default(note, "latestVersionId", Value::String(String::new()));
    if !title.is_string()
        || !content.is_string()
        || !document.is_object()
        || !attachments.is_array()
        || !revisions.is_array()
        || !versions.is_array()
        || !latest_version_id.is_string()
    {
        return Err(CryptoError::InvalidInput);
    }
    Ok(ProtectedNotePayload {
        format_version: PAYLOAD_FORMAT_VERSION,
        note_id,
        title,
        content,
        document,
        attachments,
        revisions,
        versions,
        latest_version_id,
    })
}

fn field_or_default(note: &Value, name: &str, default: Value) -> Value {
    note.get(name).cloned().unwrap_or(default)
}

fn empty_document() -> Value {
    json!({
        "markdownEnabled": false,
        "richTextEnabled": false,
        "richTextPlainText": "",
        "blocks": []
    })
}

fn empty_array_value() -> Value {
    Value::Array(Vec::new())
}

fn empty_string_value() -> Value {
    Value::String(String::new())
}

fn serialize_payload(payload: &ProtectedNotePayload) -> CryptoResult<Vec<u8>> {
    serde_json::to_vec(payload).map_err(|_| CryptoError::InvalidInput)
}

fn sanitize_payload_rich_text(payload: &mut ProtectedNotePayload) {
    sanitize_note_document_value(&mut payload.document);
    sanitize_history_document_values(&mut payload.revisions);
    sanitize_history_document_values(&mut payload.versions);
}

fn sanitize_history_document_values(history: &mut Value) {
    let Some(items) = history.as_array_mut() else {
        return;
    };
    for item in items {
        if let Some(document) = item.get_mut("document") {
            sanitize_note_document_value(document);
        }
    }
}

fn parse_payload(
    payload_bytes: &[u8],
    expected_note_id: &str,
) -> CryptoResult<ProtectedNotePayload> {
    let payload: ProtectedNotePayload =
        serde_json::from_slice(payload_bytes).map_err(|_| CryptoError::AuthenticationFailed)?;
    if payload.format_version != PAYLOAD_FORMAT_VERSION || payload.note_id != expected_note_id {
        return Err(CryptoError::AuthenticationFailed);
    }
    if !payload.title.is_string()
        || !payload.content.is_string()
        || !payload.document.is_object()
        || !payload.attachments.is_array()
        || !payload.revisions.is_array()
        || !payload.versions.is_array()
        || !payload.latest_version_id.is_string()
    {
        return Err(CryptoError::AuthenticationFailed);
    }
    Ok(payload)
}

fn seal_note_value(note: &mut Value, envelope: &NoteEncryptionEnvelope) -> CryptoResult<String> {
    let object = note.as_object_mut().ok_or(CryptoError::InvalidInput)?;
    object.insert("title".to_string(), Value::String(String::new()));
    object.insert("content".to_string(), Value::String(String::new()));
    object.insert("document".to_string(), empty_document());
    object.insert("attachments".to_string(), Value::Array(Vec::new()));
    object.insert("revisions".to_string(), Value::Array(Vec::new()));
    object.insert("versions".to_string(), Value::Array(Vec::new()));
    object.insert("latestVersionId".to_string(), Value::String(String::new()));
    object.insert(
        "encryption".to_string(),
        serde_json::to_value(envelope).map_err(|_| CryptoError::InvalidInput)?,
    );
    object.insert(
        "protectionStateRevision".to_string(),
        json!(envelope.protection_revision),
    );
    serde_json::to_string(note).map_err(|_| CryptoError::InvalidInput)
}

fn restore_protected_payload(note: &mut Value, payload: &ProtectedNotePayload) -> CryptoResult<()> {
    if note_id(note)? != payload.note_id {
        return Err(CryptoError::AuthenticationFailed);
    }
    let object = note.as_object_mut().ok_or(CryptoError::InvalidInput)?;
    object.insert("title".to_string(), payload.title.clone());
    object.insert("content".to_string(), payload.content.clone());
    object.insert("document".to_string(), payload.document.clone());
    object.insert("attachments".to_string(), payload.attachments.clone());
    object.insert("revisions".to_string(), payload.revisions.clone());
    object.insert("versions".to_string(), payload.versions.clone());
    object.insert(
        "latestVersionId".to_string(),
        payload.latest_version_id.clone(),
    );
    bootstrap_legacy_version_stack(note)?;
    Ok(())
}

fn bootstrap_legacy_version_stack(note: &mut Value) -> CryptoResult<()> {
    let note_id = note_id(note)?.to_string();
    let versions = match note.get("versions") {
        Some(value) => value.as_array().ok_or(CryptoError::InvalidInput)?.clone(),
        None => Vec::new(),
    };
    if !versions.is_empty() {
        let latest_is_present = note
            .get("latestVersionId")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.is_empty());
        if !latest_is_present {
            let latest_id = versions
                .iter()
                .filter(|version| {
                    version
                        .get("deletedAtEpochMillis")
                        .is_none_or(Value::is_null)
                })
                .max_by_key(|version| {
                    (
                        version.get("sequence").and_then(Value::as_i64).unwrap_or(0),
                        version
                            .get("createdAtEpochMillis")
                            .and_then(Value::as_i64)
                            .unwrap_or(0),
                    )
                })
                .and_then(|version| version.get("id"))
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .ok_or(CryptoError::AuthenticationFailed)?
                .to_string();
            note.as_object_mut()
                .ok_or(CryptoError::InvalidInput)?
                .insert("latestVersionId".to_string(), Value::String(latest_id));
        }
        return Ok(());
    }

    let attachment_ids = note
        .get("attachments")
        .map(|value| value.as_array().ok_or(CryptoError::InvalidInput))
        .transpose()?
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|attachment| attachment.get("id").and_then(Value::as_str))
        .map(|id| Value::String(id.to_string()))
        .collect::<Vec<_>>();
    let version_id = format!("{note_id}:version:1");
    let version = json!({
        "id": version_id.clone(),
        "noteId": note_id,
        "sequence": 1,
        "title": field_or_default(note, "title", Value::String(String::new())),
        "content": field_or_default(note, "content", Value::String(String::new())),
        "kind": field_or_default(note, "kind", Value::String("STICKY".to_string())),
        "document": field_or_default(note, "document", empty_document()),
        "accentSeed": field_or_default(note, "accentSeed", Value::String("amber".to_string())),
        "pinned": field_or_default(note, "pinned", Value::Bool(false)),
        "folderId": field_or_default(note, "folderId", Value::Null),
        "attachmentIds": attachment_ids,
        "attachments": field_or_default(note, "attachments", Value::Array(Vec::new())),
        "createdAtEpochMillis": field_or_default(note, "createdAtEpochMillis", json!(0)),
        "updatedAtEpochMillis": field_or_default(note, "updatedAtEpochMillis", json!(0)),
        "isLatest": true,
        "deletedAtEpochMillis": Value::Null,
    });
    let object = note.as_object_mut().ok_or(CryptoError::InvalidInput)?;
    object.insert("versions".to_string(), Value::Array(vec![version]));
    object.insert("latestVersionId".to_string(), Value::String(version_id));
    Ok(())
}

fn synchronize_latest_version_from_note(note: &mut Value) -> CryptoResult<()> {
    let latest_version_id = note
        .get("latestVersionId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or(CryptoError::InvalidInput)?
        .to_string();
    let title = field_or_default(note, "title", Value::String(String::new()));
    let content = field_or_default(note, "content", Value::String(String::new()));
    let kind = field_or_default(note, "kind", Value::String("STICKY".to_string()));
    let document = field_or_default(note, "document", empty_document());
    let accent_seed = field_or_default(note, "accentSeed", Value::String("amber".to_string()));
    let pinned = field_or_default(note, "pinned", Value::Bool(false));
    let folder_id = field_or_default(note, "folderId", Value::Null);
    let attachments = field_or_default(note, "attachments", Value::Array(Vec::new()));
    let attachment_ids = attachments
        .as_array()
        .ok_or(CryptoError::InvalidInput)?
        .iter()
        .filter_map(|attachment| attachment.get("id").and_then(Value::as_str))
        .map(|id| Value::String(id.to_string()))
        .collect::<Vec<_>>();
    let updated_at = field_or_default(note, "updatedAtEpochMillis", json!(0));

    let versions = note
        .get_mut("versions")
        .and_then(Value::as_array_mut)
        .ok_or(CryptoError::InvalidInput)?;
    let latest_index = versions
        .iter()
        .position(|version| version.get("id").and_then(Value::as_str) == Some(&latest_version_id))
        .ok_or(CryptoError::InvalidInput)?;
    for (index, version) in versions.iter_mut().enumerate() {
        let object = version.as_object_mut().ok_or(CryptoError::InvalidInput)?;
        object.insert("isLatest".to_string(), Value::Bool(index == latest_index));
    }
    let latest = versions[latest_index]
        .as_object_mut()
        .ok_or(CryptoError::InvalidInput)?;
    let content_changed = latest.get("title") != Some(&title)
        || latest.get("content") != Some(&content)
        || latest.get("kind") != Some(&kind)
        || latest.get("document") != Some(&document)
        || latest.get("accentSeed") != Some(&accent_seed)
        || latest.get("attachmentIds") != Some(&Value::Array(attachment_ids.clone()))
        || latest.get("attachments") != Some(&attachments);
    latest.insert("title".to_string(), title);
    latest.insert("content".to_string(), content);
    latest.insert("kind".to_string(), kind);
    latest.insert("document".to_string(), document);
    latest.insert("accentSeed".to_string(), accent_seed);
    latest.insert("pinned".to_string(), pinned);
    latest.insert("folderId".to_string(), folder_id);
    latest.insert("attachmentIds".to_string(), Value::Array(attachment_ids));
    latest.insert("attachments".to_string(), attachments);
    if content_changed {
        latest.insert("updatedAtEpochMillis".to_string(), updated_at);
    }
    Ok(())
}

fn envelope_from_note(note: &Value) -> CryptoResult<NoteEncryptionEnvelope> {
    let mut envelope: NoteEncryptionEnvelope = serde_json::from_value(
        note.get("encryption")
            .cloned()
            .ok_or(CryptoError::InvalidInput)?,
    )
    .map_err(|_| CryptoError::InvalidInput)?;
    envelope.repair_legacy_omitted_metadata();
    Ok(envelope)
}

fn validate_envelope(envelope: &NoteEncryptionEnvelope) -> CryptoResult<()> {
    if envelope.format_version != FORMAT_VERSION
        || envelope.key_id.is_empty()
        || envelope.key_id.len() > 256
        || envelope.protection_revision == 0
        || envelope.cipher_suite != CIPHER_SUITE
        || envelope.kdf != KDF_NAME
        || envelope.memory_kib != KDF_MEMORY_KIB
        || envelope.iterations != KDF_ITERATIONS
        || envelope.parallelism != KDF_PARALLELISM
    {
        return Err(CryptoError::UnsupportedEnvelope);
    }
    decode_array::<SALT_BYTES>(&envelope.salt_base64)?;
    decode_array::<NONCE_BYTES>(&envelope.key_nonce_base64)?;
    decode_array::<NONCE_BYTES>(&envelope.content_nonce_base64)?;
    let wrapped_key = decode(&envelope.wrapped_key_base64)?;
    let ciphertext = decode(&envelope.ciphertext_base64)?;
    if wrapped_key.len() != DATA_KEY_BYTES + AES_256_GCM.tag_len()
        || ciphertext.len() < AES_256_GCM.tag_len()
        || ciphertext.len() > MAX_NOTE_JSON_BYTES + AES_256_GCM.tag_len()
    {
        return Err(CryptoError::InvalidInput);
    }
    Ok(())
}

fn validate_note_envelope_for_session(note: &Value, session: &NoteSession) -> CryptoResult<()> {
    if let Some(value) = note.get("encryption").filter(|value| !value.is_null()) {
        let mut envelope: NoteEncryptionEnvelope =
            serde_json::from_value(value.clone()).map_err(|_| CryptoError::InvalidInput)?;
        envelope.repair_legacy_omitted_metadata();
        validate_envelope(&envelope)?;
        if envelope.key_id != session.envelope.key_id
            || envelope.protection_revision != session.envelope.protection_revision
        {
            return Err(CryptoError::AuthenticationFailed);
        }
    }
    validate_note_protection_state(note, session.envelope.protection_revision)
}

fn key_aad(note_id: &str, key_id: &str, revision: u64) -> Vec<u8> {
    aad("key", note_id, key_id, revision)
}

fn content_aad(note_id: &str, key_id: &str, revision: u64) -> Vec<u8> {
    aad("content", note_id, key_id, revision)
}

fn aad(purpose: &str, note_id: &str, key_id: &str, revision: u64) -> Vec<u8> {
    format!(
        "gridtimer.note.crypto/v1|{}|noteId={}:{}|keyId={}:{}|revision={}",
        purpose,
        note_id.len(),
        note_id,
        key_id.len(),
        key_id,
        revision
    )
    .into_bytes()
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn encode(bytes: &[u8]) -> String {
    STANDARD_NO_PAD.encode(bytes)
}

fn decode(value: &str) -> CryptoResult<Vec<u8>> {
    STANDARD_NO_PAD
        .decode(value)
        .map_err(|_| CryptoError::InvalidInput)
}

fn decode_array<const N: usize>(value: &str) -> CryptoResult<[u8; N]> {
    let decoded = decode(value)?;
    decoded.try_into().map_err(|_| CryptoError::InvalidInput)
}

fn random_array<const N: usize>() -> CryptoResult<[u8; N]> {
    let mut output = [0_u8; N];
    OsRng
        .try_fill_bytes(&mut output)
        .map_err(|_| CryptoError::RandomFailed)?;
    Ok(output)
}

fn random_id<const N: usize>() -> CryptoResult<String> {
    Ok(URL_SAFE_NO_PAD.encode(random_array::<N>()?))
}

fn next_session_sequence() -> u64 {
    SESSION_SEQUENCE.fetch_add(1, Ordering::Relaxed)
}

fn insert_session(session: NoteSession) -> CryptoResult<String> {
    let mut store = lock_sessions();
    if store.len() >= MAX_SESSIONS {
        if let Some(oldest_token) = store
            .iter()
            .min_by_key(|(_, value)| value.sequence)
            .map(|(token, _)| token.clone())
        {
            store.remove(&oldest_token);
        }
    }
    for _ in 0..8 {
        let token = random_id::<SESSION_TOKEN_BYTES>()?;
        if !store.contains_key(&token) {
            store.insert(token.clone(), session);
            return Ok(token);
        }
    }
    Err(CryptoError::RandomFailed)
}

fn lock_sessions() -> std::sync::MutexGuard<'static, HashMap<String, NoteSession>> {
    sessions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
