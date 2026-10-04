// v0.0.4 - Preserve unresolved encrypted records when building compact indexes.
// v0.0.3 - Scope envelope hashing to requested note identities while checking every owner.
// v0.0.2 - Leave unsupported records unresolved while indexing independent known notes.
// v0.0.1 - Index exact encrypted note identities in current and conflict records.
use crate::sealed_media_references::SealedMediaReferences;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct RetainedSealedNotes<'a> {
    notes: BTreeMap<String, &'a Value>,
    current: BTreeSet<String>,
    unresolved: usize,
}

impl<'a> RetainedSealedNotes<'a> {
    pub(crate) fn from_snapshot(snapshot: &'a Value) -> Result<Self, String> {
        Self::from_snapshot_matching(snapshot, None)
    }

    /// Filtering changes only which ciphertexts are hashed and retained. The
    /// full document still receives duplicate-identity and conflict-owner checks.
    pub(crate) fn for_note_ids(
        snapshot: &'a Value,
        note_ids: &BTreeSet<&str>,
    ) -> Result<Self, String> {
        Self::from_snapshot_matching(snapshot, Some(note_ids))
    }

    fn from_snapshot_matching(
        snapshot: &'a Value,
        note_ids: Option<&BTreeSet<&str>>,
    ) -> Result<Self, String> {
        if !snapshot.is_object() {
            return Err("Private reference snapshot is not an object.".into());
        }
        let mut result = Self {
            notes: BTreeMap::new(),
            current: BTreeSet::new(),
            unresolved: 0,
        };
        let mut heads = BTreeSet::new();
        if let Some(notes) = snapshot.get("notes") {
            for note in notes
                .as_array()
                .ok_or("Private reference notes are not an array.")?
            {
                let id = note
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or("Private note identity is missing.")?;
                if !heads.insert(id) {
                    return Err("Private note identity is ambiguous.".into());
                }
                result.insert(note, true, note_ids)?;
            }
        }
        if let Some(conflicts) = snapshot.get("syncConflictHistory") {
            for conflict in conflicts
                .as_array()
                .ok_or("Private conflict history is not an array.")?
            {
                if conflict.get("entityType").and_then(Value::as_str) == Some("note") {
                    let note = conflict
                        .get("payload")
                        .ok_or("Private conflict note is missing.")?;
                    if conflict.get("entityId").and_then(Value::as_str)
                        != note.get("id").and_then(Value::as_str)
                    {
                        return Err("Private conflict owner does not match its payload.".into());
                    }
                    result.insert(note, false, note_ids)?;
                }
            }
        }
        Ok(result)
    }

    fn insert(
        &mut self,
        note: &'a Value,
        current: bool,
        note_ids: Option<&BTreeSet<&str>>,
    ) -> Result<(), String> {
        if note_ids.is_some_and(|wanted| {
            note.get("id")
                .and_then(Value::as_str)
                .is_none_or(|id| !wanted.contains(id))
        }) {
            return Ok(());
        }
        if note.get("encryption").is_none_or(Value::is_null) {
            return Ok(());
        }
        if !crate::app_data::note_media_reference_shape_is_known(note) {
            self.unresolved += 1;
            return Ok(());
        }
        let Some(key) = SealedMediaReferences::key_for_note(note) else {
            self.unresolved += 1;
            // This is an index of provable identities, not an all-reference
            // completeness check. The retention scope still counts unsupported
            // records as unknown and prevents deletion of unverified files.
            return Ok(());
        };
        if self.notes.len() >= 40_000 && !self.notes.contains_key(&key) {
            return Err("Private note reference index exceeds its capacity.".into());
        }
        if current {
            self.current.insert(key.clone());
        }
        self.notes.entry(key).or_insert(note);
        Ok(())
    }

    pub(crate) fn get(&self, id: &str, fingerprint: &str) -> Option<&'a Value> {
        self.notes
            .get(fingerprint)
            .copied()
            .filter(|note| note.get("id").and_then(Value::as_str) == Some(id))
    }

    pub(crate) fn is_current(&self, id: &str, fingerprint: &str) -> bool {
        self.current.contains(fingerprint) && self.get(id, fingerprint).is_some()
    }

    pub(crate) fn unresolved(&self) -> usize {
        self.unresolved
    }

    pub(crate) fn records(&self) -> impl Iterator<Item = (&str, &'a Value)> {
        self.notes
            .iter()
            .map(|(fingerprint, note)| (fingerprint.as_str(), *note))
    }
}
