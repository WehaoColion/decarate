// v1.0.3.1 Windows - Isolate password-change sessions by verified workspace owner.
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
    session_scope: String,
    note_id: String,
    data_key: [u8; DATA_KEY_BYTES],
    envelope: NoteEncryptionEnvelope,
    cached_payload_digest: [u8; 32],
    #[cfg(not(target_os = "android"))]
    media_references: Option<crate::sealed_media_references::SealedMediaReferences>,
    pending_password_change: Option<PendingPasswordChange>,
    sequence: u64,
}

struct PendingPasswordChange {
    source_envelope: NoteEncryptionEnvelope,
    candidate_envelope: NoteEncryptionEnvelope,
    #[cfg(not(target_os = "android"))]
    media_references: Option<crate::sealed_media_references::SealedMediaReferences>,
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
    encrypt_note_impl(note_json, password, "").ok()
}

pub fn unlock_note(note_json: &str, password: &str) -> Option<(String, String)> {
    unlock_note_impl(note_json, password, "").ok()
}

/// Callers bind desktop sessions to a stable verified workspace owner. Scope is
/// process-local metadata; it does not change the note envelope or its keys.
#[cfg(not(target_os = "android"))]
pub fn encrypt_note_in_scope(
    note_json: &str,
    password: &str,
    scope: &str,
) -> Option<(String, String)> {
    if scope.is_empty() || scope.len() > 512 {
        return None;
    }
    encrypt_note_impl(note_json, password, scope).ok()
}

#[cfg(not(target_os = "android"))]
pub fn unlock_note_in_scope(
    note_json: &str,
    password: &str,
    scope: &str,
) -> Option<(String, String)> {
    if scope.is_empty() || scope.len() > 512 {
        return None;
    }
    unlock_note_impl(note_json, password, scope).ok()
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
/// A successful commit revokes every in-memory session for the same note in its
/// workspace scope. Legacy callers share the original default scope. The note
/// remains locked until it is unlocked again with the new password.
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

fn encrypt_note_impl(
    note_json: &str,
    password: &str,
    session_scope: &str,
) -> CryptoResult<(String, String)> {
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
    #[cfg(not(target_os = "android"))]
    let media_references = reference_declaration(&envelope, &payload)?;
    let sealed_note = seal_note_value(&mut note, &envelope)?;
    let token = insert_session(NoteSession {
        session_scope: session_scope.to_owned(),
        note_id,
        data_key: *data_key,
        envelope,
        cached_payload_digest: payload_digest,
        #[cfg(not(target_os = "android"))]
        media_references,
        pending_password_change: None,
        sequence: next_session_sequence(),
    })?;
    Ok((sealed_note, token))
}

fn unlock_note_impl(
    note_json: &str,
    password: &str,
    session_scope: &str,
) -> CryptoResult<(String, String)> {
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
    #[cfg(not(target_os = "android"))]
    let media_references = reference_declaration(&envelope, &payload)?;
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
        session_scope: session_scope.to_owned(),
        note_id,
        data_key: *data_key,
        envelope,
        cached_payload_digest: payload_digest,
        #[cfg(not(target_os = "android"))]
        media_references,
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
    #[cfg(not(target_os = "android"))]
    {
        session.media_references = reference_declaration(&envelope, &payload)?;
    }
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
    #[cfg(not(target_os = "android"))]
    let media_references = reference_declaration(&next_envelope, &payload)?;
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
            #[cfg(not(target_os = "android"))]
            media_references,
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
    let session_scope = {
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
        session.session_scope.clone()
    };

    let candidate_key_id = candidate_envelope.key_id.as_str();
    let candidate_revision = candidate_envelope.protection_revision;
    if store.values().any(|session| {
        session.session_scope == session_scope
            && session.note_id == note_id
            && (session.envelope.protection_revision > candidate_revision
                || (session.envelope.protection_revision == candidate_revision
                    && session.envelope.key_id != candidate_key_id))
    }) {
        return Err(CryptoError::SessionUnavailable);
    }

    // HashMap::retain drops removed NoteSession values before returning. Its
    // Drop implementation zeroizes both the data key and cached payload digest.
    store.retain(|_, session| session.session_scope != session_scope || session.note_id != note_id);
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

/// Return companion metadata only while the matching authenticated session is
/// alive. Shared note JSON and the encryption envelope stay byte-shape compatible.
/// The caller must persist/transmit this separately before closing the session.
#[cfg(not(target_os = "android"))]
pub(crate) fn session_media_references_json(note_json: &str, token: &str) -> Option<String> {
    session_media_references_with_scope(note_json, token).map(|(metadata, _scope)| metadata)
}

/// Capture the verified envelope declaration and its originating workspace
/// under one session lock, so callers cannot rebind it to another account.
#[cfg(not(target_os = "android"))]
pub(crate) fn session_media_references_with_scope(
    note_json: &str,
    token: &str,
) -> Option<(String, String)> {
    let note = parse_note(note_json).ok()?;
    let note_id = note_id(&note).ok()?;
    let envelope = envelope_from_note(&note).ok()?;
    let store = lock_sessions();
    let session = store.get(token)?;
    if session.note_id != note_id {
        return None;
    }
    let declaration = if envelope == session.envelope {
        session.media_references.as_ref()?
    } else {
        let candidate = session.pending_password_change.as_ref()?;
        if envelope != candidate.candidate_envelope {
            return None;
        }
        candidate.media_references.as_ref()?
    };
    let envelope = serde_json::to_value(&envelope).ok()?;
    if !declaration.valid_for(note_id, &envelope) {
        return None;
    }
    Some((
        serde_json::to_string(declaration).ok()?,
        session.session_scope.clone(),
    ))
}

#[cfg(not(target_os = "android"))]
fn reference_declaration(
    envelope: &NoteEncryptionEnvelope,
    payload: &ProtectedNotePayload,
) -> CryptoResult<Option<crate::sealed_media_references::SealedMediaReferences>> {
    let envelope = serde_json::to_value(envelope).map_err(|_| CryptoError::InvalidInput)?;
    Ok(
        crate::sealed_media_references::SealedMediaReferences::from_payload(
            &payload.note_id,
            &envelope,
            &payload.attachments,
            &payload.document,
            &payload.revisions,
            &payload.versions,
        ),
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWORD: &str = "correct horse battery staple";
    const NEW_PASSWORD: &str = "new password with enough entropy";

    #[test]
    fn new_passwords_enforce_the_native_minimum_length() {
        assert!(validate_new_password("short").is_err());
        assert!(validate_new_password("12345678").is_ok());
        assert!(validate_new_password("😀😀😀😀").is_err());
        assert!(validate_new_password("😀😀😀😀😀😀😀😀").is_ok());
        assert!(validate_new_password(&"x".repeat(MAX_NEW_PASSWORD_CHARS + 1)).is_err());
    }

    #[test]
    fn legacy_encrypted_rich_text_is_sanitized_on_unlock_and_first_reseal() {
        let mut note: Value = serde_json::from_str(&sample_note("note-rich-text-safety")).unwrap();
        let unsafe_document = json!({
            "markdownEnabled": false,
            "richTextEnabled": true,
            "richTextPlainText": "Safe",
            "blocks": [{
                "id": "b1",
                "type": "TEXT",
                "text": "<p onclick=\"steal()\">Safe</p><script>steal()</script>"
            }]
        });
        note["document"] = unsafe_document.clone();
        note["revisions"][0]["document"] = unsafe_document.clone();
        note["versions"][0]["document"] = unsafe_document;

        // Recreate a legacy envelope whose authenticated plaintext predates the
        // sanitizer.  Going through encrypt_note() here would sanitize first and
        // would not prove that unlock migrates existing unsafe ciphertext.
        let payload = protected_payload_from_note(&note).unwrap();
        let payload_bytes = serialize_payload(&payload).unwrap();
        assert!(String::from_utf8_lossy(&payload_bytes).contains("onclick"));
        let data_key = random_array::<DATA_KEY_BYTES>().unwrap();
        let key_id = random_id::<KEY_ID_BYTES>().unwrap();
        let envelope = create_envelope(
            "note-rich-text-safety",
            &key_id,
            1,
            &data_key,
            PASSWORD,
            &payload_bytes,
        )
        .unwrap();
        let legacy_ciphertext = envelope.ciphertext_base64.clone();
        let sealed = seal_note_value(&mut note, &envelope).unwrap();

        let (unlocked, unlock_token) = unlock_note(&sealed, PASSWORD).unwrap();
        let unlocked_value: Value = serde_json::from_str(&unlocked).unwrap();
        assert_eq!(
            unlocked_value["document"]["blocks"][0]["text"].as_str(),
            Some("<p>Safe</p>")
        );
        assert_eq!(
            unlocked_value["revisions"][0]["document"]["blocks"][0]["text"].as_str(),
            Some("<p>Safe</p>")
        );
        assert_eq!(
            unlocked_value["versions"][0]["document"]["blocks"][0]["text"].as_str(),
            Some("<p>Safe</p>")
        );

        let resealed = seal_note(&unlocked, &unlock_token).unwrap();
        let resealed_value: Value = serde_json::from_str(&resealed).unwrap();
        let resealed_envelope = envelope_from_note(&resealed_value).unwrap();
        assert_ne!(resealed_envelope.ciphertext_base64, legacy_ciphertext);
        close_session(&unlock_token);

        let (round_tripped, round_trip_token) = unlock_note(&resealed, PASSWORD).unwrap();
        assert!(!round_tripped.contains("onclick"));
        assert!(!round_tripped.contains("<script"));
        close_session(&round_trip_token);
    }

    fn sample_note(id: &str) -> String {
        json!({
            "id": id,
            "title": "Private title",
            "content": "Private body",
            "kind": "DOCUMENT",
            "document": {
                "markdownEnabled": true,
                "richTextEnabled": false,
                "richTextPlainText": "",
                "blocks": [{"id":"b1", "type":"TEXT", "text":"Secret block"}]
            },
            "accentSeed": "amber",
            "pinned": true,
            "folderId": "private-folder",
            "attachments": [{
                "id": "a1",
                "kind": "IMAGE",
                "fileName": "private.jpg",
                "displayName": "Private image"
            }],
            "revisions": [{"id":"r1", "title":"Older secret", "content":"Old body"}],
            "versions": [{
                "id": format!("{id}:version:1"),
                "noteId": id,
                "sequence": 1,
                "title": "Private title",
                "content": "Private body",
                "kind": "DOCUMENT",
                "document": {
                    "markdownEnabled": true,
                    "richTextEnabled": false,
                    "richTextPlainText": "",
                    "blocks": [{"id":"b1", "type":"TEXT", "text":"Secret block"}]
                },
                "accentSeed": "amber",
                "pinned": true,
                "folderId": "private-folder",
                "attachments": [{
                    "id": "a1",
                    "kind": "IMAGE",
                    "fileName": "private.jpg",
                    "displayName": "Private image"
                }],
                "attachmentIds": ["a1"],
                "createdAtEpochMillis": 10,
                "updatedAtEpochMillis": 20,
                "isLatest": true
            }],
            "latestVersionId": format!("{id}:version:1"),
            "createdAtEpochMillis": 10,
            "updatedAtEpochMillis": 20
        })
        .to_string()
    }

    fn envelope(note_json: &str) -> NoteEncryptionEnvelope {
        let note: Value = serde_json::from_str(note_json).unwrap();
        envelope_from_note(&note).unwrap()
    }

    #[test]
    fn note_crypto_correct_password_restores_all_protected_fields() {
        let original = sample_note("note-correct");
        let (sealed, token) = encrypt_note(&original, PASSWORD).expect("encrypt note");
        let sealed_value: Value = serde_json::from_str(&sealed).unwrap();
        assert_eq!("", sealed_value["title"]);
        assert_eq!("", sealed_value["content"]);
        assert_eq!(
            0,
            sealed_value["document"]["blocks"].as_array().unwrap().len()
        );
        assert_eq!(0, sealed_value["attachments"].as_array().unwrap().len());
        assert_eq!(0, sealed_value["revisions"].as_array().unwrap().len());
        assert_eq!(0, sealed_value["versions"].as_array().unwrap().len());
        assert_eq!("", sealed_value["latestVersionId"]);
        assert!(!sealed.contains("Private title"));
        assert!(!sealed.contains("Secret block"));
        assert!(!sealed.contains("Private body"));
        close_session(&token);

        let (unlocked, unlock_token) = unlock_note(&sealed, PASSWORD).expect("unlock note");
        let unlocked_value: Value = serde_json::from_str(&unlocked).unwrap();
        assert_eq!("Private title", unlocked_value["title"]);
        assert_eq!("Private body", unlocked_value["content"]);
        assert_eq!(
            "Secret block",
            unlocked_value["document"]["blocks"][0]["text"]
        );
        assert_eq!("private.jpg", unlocked_value["attachments"][0]["fileName"]);
        assert_eq!("Older secret", unlocked_value["revisions"][0]["title"]);
        assert_eq!("Private title", unlocked_value["versions"][0]["title"]);
        assert_eq!("note-correct:version:1", unlocked_value["latestVersionId"]);
        assert!(unlocked_value.get("encryption").is_some());
        assert!(close_session(&unlock_token));
    }

    #[test]
    fn legacy_protected_payload_bootstraps_one_explicit_version() {
        let legacy = json!({
            "formatVersion": PAYLOAD_FORMAT_VERSION,
            "noteId": "legacy-encrypted",
            "title": "旧标题",
            "content": "旧正文",
            "document": empty_document(),
            "attachments": [],
            "revisions": []
        });
        let payload = parse_payload(legacy.to_string().as_bytes(), "legacy-encrypted")
            .expect("legacy payload remains readable");
        let mut note = json!({
            "id": "legacy-encrypted",
            "kind": "DOCUMENT",
            "accentSeed": "amber",
            "pinned": false,
            "folderId": null,
            "createdAtEpochMillis": 10,
            "updatedAtEpochMillis": 20
        });
        restore_protected_payload(&mut note, &payload).expect("restore legacy payload");
        assert_eq!(1, note["versions"].as_array().unwrap().len());
        assert_eq!(1, note["versions"][0]["sequence"]);
        assert_eq!("旧正文", note["versions"][0]["content"]);
        assert_eq!("legacy-encrypted:version:1", note["latestVersionId"]);
    }

    #[test]
    fn encrypted_note_versions_are_explicit_idempotent_and_latest_only_editable() {
        let note_id = "encrypted-version-note";
        let original = sample_note(note_id);
        let (sealed_v1, initial_token) = encrypt_note(&original, PASSWORD).unwrap();
        assert!(close_session(&initial_token));
        let app_data = crate::app_data::upsert_note_app_data_json(
            &crate::app_data::default_app_data_json(1),
            &sealed_v1,
            30,
        )
        .unwrap();
        let (unlocked_v1, token_v1) = unlock_note(&sealed_v1, PASSWORD).unwrap();
        let expected_v1 = format!("{note_id}:version:1");
        let sealed_v2 = create_and_seal_note_version(
            &app_data,
            &unlocked_v1,
            &token_v1,
            "",
            &expected_v1,
            "encrypted-request-2",
            40,
        )
        .unwrap();
        assert!(close_session(&token_v1));

        let app_data_v2 =
            crate::app_data::upsert_note_app_data_json(&app_data, &sealed_v2, 40).unwrap();
        let (unlocked_v2, token_v2) = unlock_note(&sealed_v2, PASSWORD).unwrap();
        let v2: Value = serde_json::from_str(&unlocked_v2).unwrap();
        assert_eq!(2, v2["versions"].as_array().unwrap().len());
        assert_eq!("encrypted-request-2", v2["latestVersionId"]);
        assert_eq!(1, v2["versions"][0]["sequence"]);
        assert_eq!(2, v2["versions"][1]["sequence"]);

        let retried = create_and_seal_note_version(
            &app_data_v2,
            &unlocked_v2,
            &token_v2,
            "",
            &expected_v1,
            "encrypted-request-2",
            41,
        )
        .expect("same request is idempotent even after latest advances");
        assert!(create_and_seal_note_version(
            &app_data_v2,
            &unlocked_v2,
            &token_v2,
            "",
            &expected_v1,
            "different-stale-request",
            41,
        )
        .is_none());

        let mut edited: Value = serde_json::from_str(&unlocked_v2).unwrap();
        edited["content"] = Value::String("latest encrypted edit".to_string());
        edited["updatedAtEpochMillis"] = json!(50);
        let edited_sealed = seal_note(&edited.to_string(), &token_v2).unwrap();
        assert!(close_session(&token_v2));
        let (edited_unlocked, edited_token) = unlock_note(&edited_sealed, PASSWORD).unwrap();
        let edited: Value = serde_json::from_str(&edited_unlocked).unwrap();
        assert_eq!(2, edited["versions"].as_array().unwrap().len());
        assert_eq!("Secret block", edited["versions"][0]["content"]);
        assert_eq!("latest encrypted edit", edited["versions"][1]["content"]);
        assert!(close_session(&edited_token));

        let (retried_unlocked, retried_token) = unlock_note(&retried, PASSWORD).unwrap();
        let retried_value: Value = serde_json::from_str(&retried_unlocked).unwrap();
        assert_eq!(2, retried_value["versions"].as_array().unwrap().len());
        assert!(close_session(&retried_token));
    }

    #[test]
    fn note_crypto_repairs_only_the_legacy_omitted_metadata_triplet() {
        let original = sample_note("note-legacy-metadata");
        let (sealed, initial_token) = encrypt_note(&original, PASSWORD).expect("encrypt note");
        assert!(close_session(&initial_token));

        let mut compact: Value = serde_json::from_str(&sealed).unwrap();
        let compact_envelope = compact["encryption"].as_object_mut().unwrap();
        compact_envelope.remove("formatVersion");
        compact_envelope.remove("cipherSuite");
        compact_envelope.remove("kdf");
        let (unlocked, compact_token) =
            unlock_note(&compact.to_string(), PASSWORD).expect("unlock compact legacy envelope");
        let unlocked_value: Value = serde_json::from_str(&unlocked).unwrap();
        assert_eq!(
            FORMAT_VERSION,
            unlocked_value["encryption"]["formatVersion"]
        );
        assert_eq!(CIPHER_SUITE, unlocked_value["encryption"]["cipherSuite"]);
        assert_eq!(KDF_NAME, unlocked_value["encryption"]["kdf"]);
        assert_eq!("Private body", unlocked_value["content"]);
        let resealed = seal_note(&unlocked, &compact_token).expect("reseal canonical envelope");
        let resealed_value: Value = serde_json::from_str(&resealed).unwrap();
        assert_eq!(
            FORMAT_VERSION,
            resealed_value["encryption"]["formatVersion"]
        );
        assert_eq!(CIPHER_SUITE, resealed_value["encryption"]["cipherSuite"]);
        assert_eq!(KDF_NAME, resealed_value["encryption"]["kdf"]);
        assert!(close_session(&compact_token));

        let mut materialized_legacy: Value = serde_json::from_str(&sealed).unwrap();
        materialized_legacy["encryption"]["formatVersion"] = json!(0);
        materialized_legacy["encryption"]["cipherSuite"] = json!("");
        materialized_legacy["encryption"]["kdf"] = json!("");
        let (unlocked, legacy_token) = unlock_note(&materialized_legacy.to_string(), PASSWORD)
            .expect("unlock materialized legacy envelope");
        let unlocked_value: Value = serde_json::from_str(&unlocked).unwrap();
        assert_eq!(
            FORMAT_VERSION,
            unlocked_value["encryption"]["formatVersion"]
        );
        assert_eq!(CIPHER_SUITE, unlocked_value["encryption"]["cipherSuite"]);
        assert_eq!(KDF_NAME, unlocked_value["encryption"]["kdf"]);
        assert!(close_session(&legacy_token));
    }

    #[test]
    fn note_crypto_keeps_all_other_envelope_fields_strict() {
        let (sealed, token) = encrypt_note(&sample_note("note-strict-envelope"), PASSWORD).unwrap();
        assert!(close_session(&token));
        let required_fields = [
            "keyId",
            "protectionRevision",
            "memoryKiB",
            "iterations",
            "parallelism",
            "saltBase64",
            "keyNonceBase64",
            "wrappedKeyBase64",
            "contentNonceBase64",
            "ciphertextBase64",
        ];
        for field in required_fields {
            let mut value: Value = serde_json::from_str(&sealed).unwrap();
            value["encryption"].as_object_mut().unwrap().remove(field);
            assert!(
                unlock_note(&value.to_string(), PASSWORD).is_none(),
                "missing {field} must be rejected"
            );
        }

        for (field, unsupported) in [
            ("formatVersion", json!(2)),
            ("cipherSuite", json!("AES-128-GCM")),
            ("kdf", json!("PBKDF2")),
        ] {
            let mut value: Value = serde_json::from_str(&sealed).unwrap();
            value["encryption"][field] = unsupported;
            assert!(
                unlock_note(&value.to_string(), PASSWORD).is_none(),
                "unsupported {field} must be rejected"
            );
        }

        let mut partial: Value = serde_json::from_str(&sealed).unwrap();
        partial["encryption"]
            .as_object_mut()
            .unwrap()
            .remove("formatVersion");
        assert!(unlock_note(&partial.to_string(), PASSWORD).is_none());

        let mut unknown: Value = serde_json::from_str(&sealed).unwrap();
        unknown["encryption"]["unknownField"] = json!(true);
        assert!(unlock_note(&unknown.to_string(), PASSWORD).is_none());
    }

    #[test]
    fn note_crypto_wrong_password_is_rejected() {
        let (sealed, token) = encrypt_note(&sample_note("note-wrong"), PASSWORD).unwrap();
        close_session(&token);
        assert!(unlock_note(&sealed, "definitely wrong").is_none());
    }

    #[test]
    fn note_crypto_rejects_a_detached_protection_state_revision() {
        let (sealed, token) = encrypt_note(&sample_note("note-detached-state"), PASSWORD).unwrap();
        assert!(close_session(&token));
        let mut detached: Value = serde_json::from_str(&sealed).unwrap();
        detached["protectionStateRevision"] = json!(2);
        assert!(unlock_note(&detached.to_string(), PASSWORD).is_none());

        detached["protectionStateRevision"] = json!(0);
        let (legacy_unlocked, legacy_token) = unlock_note(&detached.to_string(), PASSWORD)
            .expect("zero remains the legacy omitted-state representation");
        let legacy_unlocked: Value = serde_json::from_str(&legacy_unlocked).unwrap();
        assert_eq!(1, legacy_unlocked["protectionStateRevision"]);
        assert!(close_session(&legacy_token));
    }

    #[test]
    fn note_crypto_tampering_is_rejected() {
        let (sealed, token) = encrypt_note(&sample_note("note-tamper"), PASSWORD).unwrap();
        close_session(&token);
        let mut value: Value = serde_json::from_str(&sealed).unwrap();
        let ciphertext = value["encryption"]["ciphertextBase64"]
            .as_str()
            .unwrap()
            .to_string();
        let mut bytes = decode(&ciphertext).unwrap();
        bytes[0] ^= 0x80;
        value["encryption"]["ciphertextBase64"] = Value::String(encode(&bytes));
        assert!(unlock_note(&value.to_string(), PASSWORD).is_none());
    }

    #[test]
    fn note_crypto_ciphertext_cannot_be_moved_to_another_note_id() {
        let (sealed, token) = encrypt_note(&sample_note("note-original"), PASSWORD).unwrap();
        close_session(&token);
        let mut value: Value = serde_json::from_str(&sealed).unwrap();
        value["id"] = Value::String("note-other".to_string());
        assert!(unlock_note(&value.to_string(), PASSWORD).is_none());
    }

    #[test]
    fn note_crypto_seal_cache_reuses_envelope_for_identical_plaintext() {
        let original = sample_note("note-cache");
        let (first_sealed, token) = encrypt_note(&original, PASSWORD).unwrap();
        let second_sealed = seal_note(&original, &token).expect("seal unchanged note");
        let third_sealed = seal_note(&original, &token).expect("seal unchanged note again");
        assert_eq!(envelope(&first_sealed), envelope(&second_sealed));
        assert_eq!(envelope(&second_sealed), envelope(&third_sealed));

        let mut changed: Value = serde_json::from_str(&original).unwrap();
        changed["content"] = Value::String("Changed private body".to_string());
        let changed_sealed = seal_note(&changed.to_string(), &token).unwrap();
        assert_ne!(
            envelope(&first_sealed).ciphertext_base64,
            envelope(&changed_sealed).ciphertext_base64
        );
        assert!(close_session(&token));
    }

    #[test]
    fn note_crypto_password_change_rotates_data_key_and_password() {
        let original = sample_note("note-password-change");
        let (sealed, token) = encrypt_note(&original, PASSWORD).unwrap();
        let old_envelope = envelope(&sealed);
        let changed = change_password(&original, &token, NEW_PASSWORD).expect("prepare password");
        let new_envelope = envelope(&changed);
        assert_ne!(old_envelope.key_id, new_envelope.key_id);
        assert_ne!(
            old_envelope.wrapped_key_base64,
            new_envelope.wrapped_key_base64
        );
        assert_ne!(
            old_envelope.ciphertext_base64,
            new_envelope.ciphertext_base64
        );
        assert_eq!(
            old_envelope.protection_revision + 1,
            new_envelope.protection_revision
        );
        let changed_value: Value = serde_json::from_str(&changed).unwrap();
        assert_eq!(
            new_envelope.protection_revision,
            changed_value["protectionStateRevision"]
        );

        // Preparing freezes this token so a later seal cannot silently overwrite
        // the candidate. Aborting restores the old generation; committing revokes it.
        assert!(seal_note(&original, &token).is_none());
        assert!(change_password(&original, &token, NEW_PASSWORD).is_none());
        assert!(commit_password_change(&changed, &token));
        assert!(seal_note(&original, &token).is_none());
        assert!(!close_session(&token));

        assert!(unlock_note(&changed, PASSWORD).is_none());
        let (unlocked, new_token) =
            unlock_note(&changed, NEW_PASSWORD).expect("new password works");
        let value: Value = serde_json::from_str(&unlocked).unwrap();
        assert_eq!("Private title", value["title"]);
        assert!(close_session(&new_token));
    }

    #[test]
    fn note_crypto_password_change_abort_preserves_the_old_session() {
        let note_id = "note-password-change-abort";
        let original = sample_note(note_id);
        let (sealed, initial_token) = encrypt_note(&original, PASSWORD).unwrap();
        assert!(close_session(&initial_token));
        let app_data = crate::app_data::upsert_note_app_data_json(
            &crate::app_data::default_app_data_json(1),
            &sealed,
            30,
        )
        .unwrap();
        let (unlocked, token) = unlock_note(&sealed, PASSWORD).unwrap();
        let candidate = change_password(&unlocked, &token, NEW_PASSWORD).unwrap();

        assert!(seal_note(&unlocked, &token).is_none());
        assert!(create_and_seal_note_version(
            &app_data,
            &unlocked,
            &token,
            "",
            &format!("{note_id}:version:1"),
            "pending-password-version-request",
            40,
        )
        .is_none());
        assert!(change_password(&unlocked, &token, NEW_PASSWORD).is_none());
        assert!(abort_password_change(&candidate, &token));
        assert!(!abort_password_change(&candidate, &token));
        assert!(!commit_password_change(&candidate, &token));
        assert!(seal_note(&unlocked, &token).is_some());
        assert!(close_session(&token));
    }

    #[test]
    fn password_change_commit_revokes_every_old_generation_token() {
        let original = sample_note("note-password-change-two-tokens");
        let (sealed, initial_token) = encrypt_note(&original, PASSWORD).unwrap();
        let app_data_v1 = crate::app_data::upsert_note_app_data_json(
            &crate::app_data::default_app_data_json(1),
            &sealed,
            10,
        )
        .unwrap();
        let (unlocked_a, token_a) = unlock_note(&sealed, PASSWORD).unwrap();
        let (unlocked_b, token_b) = unlock_note(&sealed, PASSWORD).unwrap();

        let candidate_a = change_password(&unlocked_a, &token_a, NEW_PASSWORD).unwrap();
        assert!(seal_note(&unlocked_b, &token_b).is_some());
        let candidate_b =
            change_password(&unlocked_b, &token_b, "another sufficiently long password").unwrap();
        assert_ne!(envelope(&candidate_a).key_id, envelope(&candidate_b).key_id);
        assert!(seal_note(&unlocked_b, &token_b).is_none());
        let app_data_v2 =
            crate::app_data::upsert_note_app_data_json(&app_data_v1, &candidate_a, 20).unwrap();
        assert!(
            crate::app_data::upsert_note_app_data_json(&app_data_v2, &candidate_b, 21).is_none()
        );

        assert!(commit_password_change(&candidate_a, &token_a));
        assert!(!commit_password_change(&candidate_b, &token_b));
        for token in [&initial_token, &token_a, &token_b] {
            assert!(seal_note(&unlocked_a, token).is_none());
            assert!(!close_session(token));
        }
    }

    #[test]
    fn password_change_commit_and_old_seal_have_safe_serializable_outcomes() {
        use std::sync::{Arc, Barrier};

        let original = sample_note("note-password-change-race");
        let (sealed, initial_token) = encrypt_note(&original, PASSWORD).unwrap();
        let app_data = crate::app_data::upsert_note_app_data_json(
            &crate::app_data::default_app_data_json(1),
            &sealed,
            10,
        )
        .unwrap();
        let (unlocked_a, token_a) = unlock_note(&sealed, PASSWORD).unwrap();
        let (unlocked_b, token_b) = unlock_note(&sealed, PASSWORD).unwrap();
        let candidate = change_password(&unlocked_a, &token_a, NEW_PASSWORD).unwrap();
        let app_data_v2 =
            crate::app_data::upsert_note_app_data_json(&app_data, &candidate, 20).unwrap();

        let barrier = Arc::new(Barrier::new(3));
        let commit_barrier = Arc::clone(&barrier);
        let commit_candidate = candidate.clone();
        let commit_token = token_a.clone();
        let commit_thread = std::thread::spawn(move || {
            commit_barrier.wait();
            commit_password_change(&commit_candidate, &commit_token)
        });
        let seal_barrier = Arc::clone(&barrier);
        let seal_note_json = unlocked_b.clone();
        let seal_token = token_b.clone();
        let seal_thread = std::thread::spawn(move || {
            seal_barrier.wait();
            seal_note(&seal_note_json, &seal_token)
        });
        barrier.wait();

        assert!(commit_thread.join().unwrap());
        if let Some(old_generation_write) = seal_thread.join().unwrap() {
            assert!(crate::app_data::upsert_note_app_data_json(
                &app_data_v2,
                &old_generation_write,
                30,
            )
            .is_none());
        }
        for token in [&initial_token, &token_a, &token_b] {
            assert!(seal_note(&unlocked_a, token).is_none());
        }
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn scoped_password_commit_preserves_other_scope_same_id_sessions() {
        let original = sample_note("scoped-password-independent-owners");
        let (sealed_a, initial_a) =
            encrypt_note_in_scope(&original, PASSWORD, "workspace-a").unwrap();
        let (plain_a, token_a) = unlock_note_in_scope(&sealed_a, PASSWORD, "workspace-a").unwrap();
        let mut other: Value = serde_json::from_str(&original).unwrap();
        other["protectionStateRevision"] = json!(8);
        let (sealed_b, initial_b) =
            encrypt_note_in_scope(&other.to_string(), PASSWORD, "workspace-b").unwrap();
        let (plain_b, token_b) = unlock_note_in_scope(&sealed_b, PASSWORD, "workspace-b").unwrap();
        let candidate = change_password(&plain_a, &token_a, NEW_PASSWORD).unwrap();

        assert!(commit_password_change(&candidate, &token_a));
        assert!(seal_note(&plain_a, &initial_a).is_none());
        assert!(seal_note(&plain_a, &token_a).is_none());
        assert!(seal_note(&plain_b, &initial_b).is_some());
        assert!(seal_note(&plain_b, &token_b).is_some());
        assert!(close_session(&initial_b));
        assert!(close_session(&token_b));
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn scoped_password_commit_rejects_newer_generation_in_same_scope() {
        let original = sample_note("scoped-password-generation-fence");
        let (sealed, initial) =
            encrypt_note_in_scope(&original, PASSWORD, "workspace-generation").unwrap();
        let (plain, token) =
            unlock_note_in_scope(&sealed, PASSWORD, "workspace-generation").unwrap();
        let candidate = change_password(&plain, &token, NEW_PASSWORD).unwrap();
        let mut newer: Value = serde_json::from_str(&original).unwrap();
        newer["protectionStateRevision"] = json!(12);
        let (newer_sealed, newer_initial) =
            encrypt_note_in_scope(&newer.to_string(), PASSWORD, "workspace-generation").unwrap();
        // Encryption advances generation 12 to 13. Use the actual unlocked
        // record rather than trying to seal the stale pre-encryption fixture.
        let (newer_plain, newer_token) =
            unlock_note_in_scope(&newer_sealed, PASSWORD, "workspace-generation").unwrap();
        assert!(close_session(&newer_initial));

        assert!(!commit_password_change(&candidate, &token));
        assert!(seal_note(&plain, &token).is_none());
        assert!(abort_password_change(&candidate, &token));
        assert!(seal_note(&plain, &token).is_some());
        assert!(seal_note(&newer_plain, &newer_token).is_some());
        assert_eq!(
            envelope(&newer_sealed).protection_revision,
            envelope(&seal_note(&newer_plain, &newer_token).unwrap()).protection_revision
        );
        for active in [&initial, &token, &newer_token] {
            assert!(close_session(active));
        }
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn scoped_password_abort_preserves_sessions_and_cannot_commit_aborted_candidate() {
        let original = sample_note("scoped-password-abort");
        let (sealed, initial) =
            encrypt_note_in_scope(&original, PASSWORD, "workspace-abort").unwrap();
        let (plain, token) = unlock_note_in_scope(&sealed, PASSWORD, "workspace-abort").unwrap();
        let (_, legacy) = unlock_note(&sealed, PASSWORD).unwrap();
        let candidate = change_password(&plain, &token, NEW_PASSWORD).unwrap();

        assert!(seal_note(&plain, &token).is_none());
        assert!(seal_note(&plain, &initial).is_some());
        assert!(seal_note(&plain, &legacy).is_some());
        assert!(abort_password_change(&candidate, &token));
        assert!(!abort_password_change(&candidate, &token));
        assert!(!commit_password_change(&candidate, &token));
        assert!(seal_note(&plain, &token).is_some());
        let retry = change_password(&plain, &token, NEW_PASSWORD).unwrap();
        assert!(commit_password_change(&retry, &token));
        assert!(seal_note(&plain, &initial).is_none());
        assert!(seal_note(&plain, &legacy).is_some());
        assert!(close_session(&legacy));
    }

    #[test]
    fn encrypt_note_advances_the_plaintext_protection_state() {
        let mut note: Value = serde_json::from_str(&sample_note("note-reencrypt")).unwrap();
        note["protectionStateRevision"] = json!(2);
        let (sealed, token) = encrypt_note(&note.to_string(), PASSWORD).unwrap();
        let sealed: Value = serde_json::from_str(&sealed).unwrap();
        assert_eq!(3, sealed["protectionStateRevision"]);
        assert_eq!(3, sealed["encryption"]["protectionRevision"]);
        assert!(close_session(&token));
    }
}
