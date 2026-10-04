// v0.0.3 - Distinguish verified retained ciphertexts from the current note head.
// v0.0.2 - Validate monotonic content acknowledgements within bounded byte limits.
// v0.0.1 - Bind private reference exchanges to exact ciphertexts and requests.
#![cfg(not(target_os = "android"))]
use crate::sealed_media_references::SealedMediaReferences;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

pub const PRIVATE_MEDIA_MAX_QUERIES: usize = 32;
pub const PRIVATE_MEDIA_MAX_REQUEST_BYTES: usize = 4 * 1024 * 1024;
pub const PRIVATE_MEDIA_MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrivateMediaQuery {
    pub note_id: String,
    pub envelope_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declaration: Option<Value>,
}

impl PrivateMediaQuery {
    pub fn for_note(note: &Value) -> Option<Self> {
        Some(Self {
            note_id: note.get("id")?.as_str()?.to_owned(),
            envelope_sha256: SealedMediaReferences::key_for_note(note)?,
            declaration: None,
        })
    }

    pub(crate) fn parsed_declaration(&self) -> Result<Option<SealedMediaReferences>, String> {
        self.declaration
            .as_ref()
            .map(|value| {
                let declaration: SealedMediaReferences = serde_json::from_value(value.clone())
                    .map_err(|_| "Invalid private attachment declaration.".to_owned())?;
                if !declaration.valid_metadata()
                    || declaration.envelope_sha256() != self.envelope_sha256
                {
                    return Err(
                        "Private attachment declaration does not match its ciphertext fingerprint."
                            .into(),
                    );
                }
                Ok(declaration)
            })
            .transpose()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PrivateMediaRequest {
    pub format_version: u32,
    pub request_id: String,
    pub acknowledged_generation: i64,
    #[serde(default)]
    pub restore_receipt: String,
    pub queries: Vec<PrivateMediaQuery>,
}

impl PrivateMediaRequest {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.format_version != 1
            || self.acknowledged_generation < 0
            || self.request_id.is_empty()
            || self.request_id.len() > 128
            || !self
                .request_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
            || self.queries.is_empty()
            || self.queries.len() > PRIVATE_MEDIA_MAX_QUERIES
            || self.restore_receipt.len() > 4096
        {
            return Err("Invalid private attachment exchange request.".into());
        }
        let mut keys = BTreeSet::new();
        for query in &self.queries {
            if query.note_id.is_empty()
                || query.note_id.len() > 1024
                || query.envelope_sha256.len() != 64
                || !query
                    .envelope_sha256
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
                || !keys.insert((&query.note_id, &query.envelope_sha256))
            {
                return Err("Invalid or duplicate private attachment query.".into());
            }
            query.parsed_declaration()?;
        }
        if serde_json::to_vec(self)
            .map_err(|error| error.to_string())?
            .len()
            > PRIVATE_MEDIA_MAX_REQUEST_BYTES
        {
            return Err("Private attachment exchange request exceeds its byte limit.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrivateMediaReplyEntry {
    pub note_id: String,
    pub envelope_sha256: String,
    pub current_head_matches: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub retained_note_matches: bool,
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declaration: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrivateMediaReply {
    pub format_version: u32,
    pub request_id: String,
    pub generation: i64,
    pub entries: Vec<PrivateMediaReplyEntry>,
}

impl PrivateMediaReply {
    pub(crate) fn validate_for(&self, request: &PrivateMediaRequest) -> Result<(), String> {
        request.validate()?;
        if self.format_version != 1
            || self.request_id != request.request_id
            || self.generation != request.acknowledged_generation
            || self.entries.len() != request.queries.len()
            || serde_json::to_vec(self)
                .map_err(|error| error.to_string())?
                .len()
                > PRIVATE_MEDIA_MAX_RESPONSE_BYTES
        {
            return Err("Private attachment response does not match the active request.".into());
        }
        for (entry, query) in self.entries.iter().zip(&request.queries) {
            if entry.note_id != query.note_id || entry.envelope_sha256 != query.envelope_sha256 {
                return Err("Private attachment response contains another ciphertext.".into());
            }
            let returned = PrivateMediaQuery {
                note_id: entry.note_id.clone(),
                envelope_sha256: entry.envelope_sha256.clone(),
                declaration: entry.declaration.clone(),
            }
            .parsed_declaration()?;
            if entry.current_head_matches && entry.retained_note_matches {
                return Err("Private attachment response has contradictory origins.".into());
            }
            if entry.accepted
                && (!(entry.current_head_matches || entry.retained_note_matches)
                    || query.declaration.is_none()
                    || !returned
                        .as_ref()
                        .zip(query.parsed_declaration()?.as_ref())
                        .is_some_and(|(retained, sent)| retained.covers(sent)))
            {
                return Err(
                    "Private attachment acknowledgement did not retain the submitted declaration."
                        .into(),
                );
            }
            if query.declaration.is_some()
                && (entry.current_head_matches || entry.retained_note_matches)
                && !entry.accepted
            {
                return Err(
                    "Private attachment response omitted a current declaration acknowledgement."
                        .into(),
                );
            }
        }
        Ok(())
    }
}

/// Only the hardened HTTP adapter can construct this account-bound receipt.
/// Reading a JSON response or cloning a request is insufficient authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedPrivateMediaReply {
    pub(crate) user_id: String,
    pub(crate) server_instance_id: String,
    pub(crate) account_namespace: String,
    pub(crate) token_id: String,
    pub(crate) reply: PrivateMediaReply,
}

impl VerifiedPrivateMediaReply {
    pub fn generation(&self) -> i64 {
        self.reply.generation
    }
    pub fn entries(&self) -> &[PrivateMediaReplyEntry] {
        &self.reply.entries
    }
}

pub struct PrivateMediaExchangeOutcome {
    pub result: crate::sync_core::SyncClientResult,
    pub verified: Option<VerifiedPrivateMediaReply>,
}
