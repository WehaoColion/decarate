// v0.0.1 - Keep exact private note identities and reference state, never accumulated ciphertext.
use crate::desktop_media_references::DesktopMediaReferenceScope;
use crate::desktop_state_store::DesktopPrivacyPolicy;
use crate::private_media_protocol::PrivateMediaQuery;
use crate::private_media_retained::RetainedSealedNotes;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug)]
struct PrivateNoteIdentity {
    note_id: String,
    revision: i64,
}

/// Built from validated note identities. This is neither AppData nor a stored
/// snapshot; the current document remains the source of edits and tombstones.
#[derive(Debug)]
pub struct DesktopPrivateMediaReferences {
    current_sha256: [u8; 32],
    records: BTreeMap<String, PrivateNoteIdentity>,
    body_scope: DesktopMediaReferenceScope,
    unverified_recovery_sources: usize,
    unverified_recovery_file_keys: BTreeSet<[u8; 32]>,
}

impl DesktopPrivateMediaReferences {
    pub fn from_snapshot(raw: &str) -> Result<Self, String> {
        let value: Value = serde_json::from_str(raw).map_err(|error| error.to_string())?;
        let mut result = Self {
            current_sha256: Sha256::digest(raw.as_bytes()).into(),
            records: BTreeMap::new(),
            body_scope: DesktopMediaReferenceScope::without_private_declarations(raw),
            unverified_recovery_sources: 0,
            unverified_recovery_file_keys: BTreeSet::new(),
        };
        result.include(&value, false)?;
        Ok(result)
    }

    pub(crate) fn include_retained_snapshot(&mut self, raw: &str) -> Result<(), String> {
        let value: Value = serde_json::from_str(raw).map_err(|error| error.to_string())?;
        self.include(&value, true)
    }

    /// A managed filename does not prove which account owns its contents. Keep
    /// such recovery data pending without granting it private exchange identity.
    pub fn mark_unverified_recovery_source(&mut self) {
        self.unverified_recovery_sources = self.unverified_recovery_sources.saturating_add(1);
        self.body_scope.retain_unknown();
    }

    pub fn unverified_recovery_sources(&self) -> usize {
        self.unverified_recovery_sources
    }

    pub fn mark_unverified_recovery_file(&mut self, path: &std::path::Path) {
        let normalized = path
            .parent()
            .and_then(|parent| std::fs::canonicalize(parent).ok())
            .and_then(|parent| path.file_name().map(|name| parent.join(name)))
            .unwrap_or_else(|| path.to_path_buf());
        let key: [u8; 32] = Sha256::digest(
            normalized
                .to_string_lossy()
                .replace('/', "\\")
                .to_ascii_lowercase()
                .as_bytes(),
        )
        .into();
        if self.unverified_recovery_file_keys.contains(&key) {
            return;
        }
        // The proof table has at most 32 sources; cap legacy inventory memory.
        if self.unverified_recovery_file_keys.len() < 2048 {
            self.unverified_recovery_file_keys.insert(key);
        }
        self.mark_unverified_recovery_source();
    }

    fn include(&mut self, value: &Value, retained: bool) -> Result<(), String> {
        let index = RetainedSealedNotes::from_snapshot(value)?;
        self.body_scope.mark_unknown_private(index.unresolved());
        if retained {
            let notes = value["notes"].as_array().into_iter().flatten().chain(
                value["syncConflictHistory"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(crate::app_data::validated_note_conflict_payload),
            );
            for note in notes.filter(|note| note.get("encryption").is_some_and(|v| !v.is_null())) {
                self.body_scope
                    .merge_references(DesktopMediaReferenceScope::retained_note_body(note));
            }
        }
        for (fingerprint, note) in index.records() {
            if self.records.len() >= 40_000 && !self.records.contains_key(fingerprint) {
                return Err("Private recovery reference index exceeds its identity limit.".into());
            }
            let revision = note["updatedAtEpochMillis"].as_i64().unwrap_or(0).max(0);
            self.records
                .entry(fingerprint.to_owned())
                .and_modify(|entry| entry.revision = entry.revision.max(revision))
                .or_insert_with(|| PrivateNoteIdentity {
                    note_id: note["id"].as_str().unwrap().to_owned(),
                    revision,
                });
        }
        Ok(())
    }

    pub(crate) fn matches_snapshot(&self, raw: &str) -> bool {
        self.current_sha256 == <[u8; 32]>::from(Sha256::digest(raw.as_bytes()))
    }

    pub(crate) fn contains(&self, note_id: &str, fingerprint: &str) -> bool {
        self.records
            .get(fingerprint)
            .is_some_and(|note| note.note_id == note_id)
    }

    pub fn queries(
        &self,
        policy: &DesktopPrivacyPolicy,
        send_local: bool,
    ) -> Result<Vec<PrivateMediaQuery>, String> {
        self.records
            .iter()
            .map(|(fingerprint, note)| {
                Ok(PrivateMediaQuery {
                    note_id: note.note_id.clone(),
                    envelope_sha256: fingerprint.clone(),
                    declaration: if send_local {
                        policy
                            .sealed_declaration_by_fingerprint(fingerprint)
                            .map(serde_json::to_value)
                            .transpose()
                            .map_err(|e| e.to_string())?
                    } else {
                        None
                    },
                })
            })
            .collect()
    }

    pub fn scope(&self, policy: &DesktopPrivacyPolicy) -> DesktopMediaReferenceScope {
        let mut scope = self.body_scope.clone();
        for (fingerprint, note) in &self.records {
            scope.include_private_declaration(
                policy
                    .sealed_declaration_by_fingerprint(fingerprint)
                    .as_ref(),
                note.revision,
            );
        }
        scope
    }
}
