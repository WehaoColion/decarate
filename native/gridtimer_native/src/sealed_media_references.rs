// v0.0.3 - Bind authenticated content hashes without changing the note encryption envelope.
// v0.0.2 - Keep reference metadata private and independent of shared note fields.
// v0.0.1 - Bind an authenticated owner's attachment declaration to one exact sealed note.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};

const MAX_IDS: usize = 10_000;
const MAX_VISITS: usize = 1_000_000;

/// This is client-generated reference metadata, not a server proof that it can
/// decrypt the note. It has the same owner-authorization boundary as a note
/// deletion. A server may use it only after account/envelope integrity checks.
/// Missing, unsupported or stale metadata means unknown references, never none.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SealedMediaReferences {
    format_version: u32,
    envelope_sha256: String,
    attachment_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    content_sha256_by_id: BTreeMap<String, String>,
    binding_sha256: String,
}

struct HashWriter(Sha256);
impl Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn hex(hash: Sha256) -> String {
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn canonical(value: &Value, output: &mut HashWriter) -> Option<()> {
    match value {
        Value::Object(object) => {
            output.write_all(b"{").ok()?;
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index != 0 {
                    output.write_all(b",").ok()?;
                }
                serde_json::to_writer(&mut *output, key).ok()?;
                output.write_all(b":").ok()?;
                canonical(&object[key], output)?;
            }
            output.write_all(b"}").ok()?;
        }
        Value::Array(items) => {
            output.write_all(b"[").ok()?;
            for (index, item) in items.iter().enumerate() {
                if index != 0 {
                    output.write_all(b",").ok()?;
                }
                canonical(item, output)?;
            }
            output.write_all(b"]").ok()?;
        }
        _ => serde_json::to_writer(output, value).ok()?,
    }
    Some(())
}

fn envelope_fingerprint(note_id: &str, envelope: &Value) -> Option<String> {
    if note_id.is_empty() || note_id.len() > 1024 || !envelope.is_object() {
        return None;
    }
    let object = envelope.as_object()?;
    let fields = [
        "formatVersion",
        "keyId",
        "protectionRevision",
        "cipherSuite",
        "kdf",
        "memoryKiB",
        "iterations",
        "parallelism",
        "saltBase64",
        "keyNonceBase64",
        "wrappedKeyBase64",
        "contentNonceBase64",
        "ciphertextBase64",
    ];
    if object.len() != fields.len()
        || fields.iter().any(|field| !object.contains_key(*field))
        || envelope["formatVersion"] != 1
        || envelope["cipherSuite"] != "AES-256-GCM"
        || envelope["kdf"] != "Argon2id"
        || envelope["protectionRevision"]
            .as_i64()
            .is_none_or(|revision| revision <= 0)
    {
        return None;
    }
    let mut output = HashWriter(Sha256::new());
    output.write_all(b"sealed-note-media-envelope-v1\0").ok()?;
    serde_json::to_writer(&mut output, note_id).ok()?;
    output.write_all(b"\0").ok()?;
    canonical(envelope, &mut output)?;
    Some(hex(output.0))
}

fn binding(envelope: &str, ids: &[String]) -> Option<String> {
    let mut output = HashWriter(Sha256::new());
    serde_json::to_writer(
        &mut output,
        &("sealed-note-media-declaration-v1", envelope, ids),
    )
    .ok()?;
    Some(hex(output.0))
}

fn valid_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    (1..=128).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn content_binding(
    envelope: &str,
    ids: &[String],
    hashes: &BTreeMap<String, String>,
) -> Option<String> {
    let mut output = HashWriter(Sha256::new());
    serde_json::to_writer(
        &mut output,
        &("sealed-note-media-declaration-v2", envelope, ids, hashes),
    )
    .ok()?;
    Some(hex(output.0))
}

#[derive(Default)]
struct References {
    ids: BTreeSet<String>,
    hashes: BTreeMap<String, String>,
    conflicting_hashes: BTreeSet<String>,
    visits: usize,
}
impl References {
    fn attachment(&mut self, value: &Value) -> Option<()> {
        let object = value.as_object()?;
        let id = object.get("id")?;
        self.id(id)?;
        let id = id.as_str()?;
        let Some(hash) = object
            .get("sha256")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        else {
            return Some(());
        };
        let hash = hash.trim().to_ascii_lowercase();
        if !valid_sha256(&hash) || self.hashes.get(id).is_some_and(|old| *old != hash) {
            self.conflicting_hashes.insert(id.to_owned());
            self.hashes.remove(id);
        } else if !self.conflicting_hashes.contains(id) {
            self.hashes.insert(id.to_owned(), hash);
        }
        Some(())
    }

    fn id(&mut self, value: &Value) -> Option<()> {
        self.visits += 1;
        if self.visits > MAX_VISITS {
            return None;
        }
        let id = value.as_str()?;
        if !valid_id(id) {
            return None;
        }
        self.ids.insert(id.to_owned());
        (self.ids.len() <= MAX_IDS).then_some(())
    }
    fn document(&mut self, value: &Value) -> Option<()> {
        self.visits += 1;
        if self.visits > MAX_VISITS {
            return None;
        }
        match value {
            Value::Object(object) => {
                if let Some(id) = object.get("attachmentId") {
                    if !id.is_null() && id != "" {
                        self.id(id)?;
                    }
                }
                for child in object.values() {
                    self.document(child)?;
                }
            }
            Value::Array(values) => {
                for child in values {
                    self.document(child)?;
                }
            }
            _ => {}
        }
        Some(())
    }
    fn snapshot(&mut self, snapshot: &Value) -> Option<()> {
        self.visits += 1;
        if self.visits > MAX_VISITS {
            return None;
        }
        let snapshot = snapshot.as_object()?;
        if let Some(items) = snapshot.get("attachments") {
            for item in items.as_array()? {
                self.attachment(item)?;
            }
        }
        if let Some(items) = snapshot.get("attachmentIds") {
            for id in items.as_array()? {
                self.id(id)?;
            }
        }
        if let Some(document) = snapshot.get("document") {
            self.document(document)?;
        }
        for field in ["revisions", "versions"] {
            if let Some(items) = snapshot.get(field) {
                for item in items.as_array()? {
                    self.snapshot(item)?;
                }
            }
        }
        Some(())
    }
}

impl SealedMediaReferences {
    pub(crate) fn has_complete_content_hashes(&self) -> bool {
        self.format_version == 2 && self.attachment_ids.len() == self.content_sha256_by_id.len()
    }
    pub(crate) fn from_payload(
        note_id: &str,
        envelope: &Value,
        attachments: &Value,
        document: &Value,
        revisions: &Value,
        versions: &Value,
    ) -> Option<Self> {
        let mut references = References::default();
        for item in attachments.as_array()? {
            references.attachment(item)?;
        }
        references.document(document)?;
        for history in [revisions, versions] {
            for snapshot in history.as_array()? {
                references.snapshot(snapshot)?;
            }
        }
        let envelope_sha256 = envelope_fingerprint(note_id, envelope)?;
        let attachment_ids = references.ids.into_iter().collect::<Vec<_>>();
        let content_sha256_by_id = references.hashes;
        let binding_sha256 =
            content_binding(&envelope_sha256, &attachment_ids, &content_sha256_by_id)?;
        Some(Self {
            format_version: 2,
            envelope_sha256,
            attachment_ids,
            content_sha256_by_id,
            binding_sha256,
        })
    }

    pub(crate) fn valid_for(&self, note_id: &str, envelope: &Value) -> bool {
        self.valid_metadata()
            && envelope_fingerprint(note_id, envelope).as_ref() == Some(&self.envelope_sha256)
    }

    pub(crate) fn valid_metadata(&self) -> bool {
        self.attachment_ids.len() <= MAX_IDS
            && self.attachment_ids.iter().all(|id| valid_id(id))
            && self.attachment_ids.windows(2).all(|pair| pair[0] < pair[1])
            && self.envelope_sha256.len() == 64
            && self
                .envelope_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            && self.content_sha256_by_id.iter().all(|(id, hash)| {
                self.attachment_ids.binary_search(id).is_ok() && valid_sha256(hash)
            })
            && match self.format_version {
                1 => {
                    self.content_sha256_by_id.is_empty()
                        && binding(&self.envelope_sha256, &self.attachment_ids).as_ref()
                            == Some(&self.binding_sha256)
                }
                2 => {
                    content_binding(
                        &self.envelope_sha256,
                        &self.attachment_ids,
                        &self.content_sha256_by_id,
                    )
                    .as_ref()
                        == Some(&self.binding_sha256)
                }
                _ => false,
            }
    }

    /// A later decoder may add content proofs, but cannot change the referenced
    /// identities or any already attested hash for the same immutable ciphertext.
    pub(crate) fn covers(&self, other: &Self) -> bool {
        self.valid_metadata()
            && other.valid_metadata()
            && (self == other
                || (self.format_version == 2
                    && self.envelope_sha256 == other.envelope_sha256
                    && self.attachment_ids == other.attachment_ids
                    && other
                        .content_sha256_by_id
                        .iter()
                        .all(|(id, hash)| self.content_sha256_by_id.get(id) == Some(hash))))
    }

    pub(crate) fn content_hashes(&self) -> &BTreeMap<String, String> {
        &self.content_sha256_by_id
    }

    pub(crate) fn ids(&self) -> &[String] {
        &self.attachment_ids
    }
    pub(crate) fn envelope_sha256(&self) -> &str {
        &self.envelope_sha256
    }

    pub(crate) fn key_for_note(note: &Value) -> Option<String> {
        envelope_fingerprint(note.get("id")?.as_str()?, note.get("encryption")?)
    }
}
