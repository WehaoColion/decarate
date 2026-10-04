// v0.0.4 - Resolve compact private identities without retaining their ciphertext bodies.
// v0.0.3 - Retain bounded content hashes and conflicting private reference state.
// v0.0.2 - Merge retained recovery references without treating unknown copies as empty.
// v0.0.1 - Keep unknown encrypted and conflict references distinct from an empty set.
use crate::app_data::{self, AppDataJsonCompatibility};
use crate::desktop_state_store::DesktopPrivacyPolicy;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

/// A snapshot-bound view of known references. Unknown data can add references,
/// so callers may recover known files but must defer destructive cleanup.
#[derive(Clone, Debug)]
pub struct DesktopMediaReferenceScope {
    ids: HashSet<String>,
    sealed_ids: HashSet<String>,
    sealed_hashes: BTreeMap<String, String>,
    sealed_conflicts: HashSet<String>,
    sealed_revisions: BTreeMap<String, i64>,
    unknown_sealed_notes: usize,
    complete: bool,
    visits: usize,
}

impl DesktopMediaReferenceScope {
    fn empty() -> Self {
        Self {
            ids: HashSet::new(),
            sealed_ids: HashSet::new(),
            sealed_hashes: BTreeMap::new(),
            sealed_conflicts: HashSet::new(),
            sealed_revisions: BTreeMap::new(),
            unknown_sealed_notes: 0,
            complete: false,
            visits: 0,
        }
    }

    pub fn from_snapshot(raw: &str, policy: &DesktopPrivacyPolicy) -> Self {
        Self::from_snapshot_with_private(raw, policy, true)
    }

    pub(crate) fn without_private_declarations(raw: &str) -> Self {
        Self::from_snapshot_with_private(raw, &DesktopPrivacyPolicy::default(), false)
    }

    pub(crate) fn retained_note_body(note: &Value) -> Self {
        let mut scope = Self::empty();
        scope.complete = true;
        scope.note(note, &DesktopPrivacyPolicy::default(), false);
        scope
    }

    fn from_snapshot_with_private(
        raw: &str,
        policy: &DesktopPrivacyPolicy,
        include_private: bool,
    ) -> Self {
        let mut scope = Self::empty();
        let Ok(value) = serde_json::from_str::<Value>(raw) else {
            return scope;
        };
        if !value.is_object() {
            return scope;
        }
        scope.complete = match app_data::app_data_json_compatibility(raw, 0) {
            AppDataJsonCompatibility::CurrentKnown => true,
            AppDataJsonCompatibility::LegacyMigratable => {
                app_data::app_data_media_reference_shape_is_known(&value)
            }
            _ => false,
        };
        if let Some(notes) = value.get("notes") {
            if let Some(notes) = notes.as_array() {
                for note in notes {
                    scope.note(note, policy, include_private);
                }
            } else {
                scope.complete = false;
            }
        }
        if let Some(conflicts) = value.get("syncConflictHistory") {
            if let Some(conflicts) = conflicts.as_array() {
                for conflict in conflicts {
                    match conflict.get("entityType").and_then(Value::as_str) {
                        Some("note") => {
                            if let Some(note) = app_data::validated_note_conflict_payload(conflict)
                            {
                                scope.note(note, policy, include_private);
                            } else {
                                scope.complete = false;
                            }
                        }
                        Some(
                            "slot"
                            | "category"
                            | "session"
                            | "archivedTask"
                            | "noteFolder"
                            | "financeProfile"
                            | "financeDayLedger"
                            | "financeMonthSnapshot",
                        ) => {}
                        _ => scope.complete = false,
                    }
                }
            } else {
                scope.complete = false;
            }
        }
        scope
    }

    pub(crate) fn retain_unknown(&mut self) {
        self.complete = false;
    }

    pub(crate) fn merge_references(&mut self, other: Self) {
        self.complete &= other.complete;
        self.unknown_sealed_notes = self
            .unknown_sealed_notes
            .saturating_add(other.unknown_sealed_notes);
        for id in other.ids {
            if self.ids.len() >= 10_000 && !self.ids.contains(&id) {
                self.complete = false;
                break;
            }
            self.ids.insert(id);
        }
        for id in other.sealed_ids {
            if !self.ids.contains(&id) {
                continue;
            }
            self.sealed_ids.insert(id.clone());
            if other.sealed_conflicts.contains(&id) {
                self.sealed_conflicts.insert(id.clone());
                self.sealed_hashes.remove(&id);
            }
            if let Some(hash) = other.sealed_hashes.get(&id) {
                self.remember_content_hash(&id, hash);
            }
            let revision = other.sealed_revisions.get(&id).copied().unwrap_or(0);
            self.sealed_revisions
                .entry(id)
                .and_modify(|old| *old = (*old).max(revision))
                .or_insert(revision);
        }
    }

    pub(crate) fn sealed_ids(&self) -> &HashSet<String> {
        &self.sealed_ids
    }
    pub(crate) fn sealed_content_hash(&self, id: &str) -> Option<&str> {
        self.sealed_hashes.get(id).map(String::as_str)
    }
    pub(crate) fn sealed_revision(&self, id: &str) -> i64 {
        self.sealed_revisions.get(id).copied().unwrap_or(0)
    }
    pub(crate) fn unknown_sealed_notes(&self) -> usize {
        self.unknown_sealed_notes
    }

    fn remember_content_hash(&mut self, id: &str, hash: &str) {
        if self.sealed_hashes.get(id).is_some_and(|old| old != hash) {
            self.sealed_conflicts.insert(id.to_owned());
            self.sealed_hashes.remove(id);
        } else if !self.sealed_conflicts.contains(id) {
            self.sealed_hashes.insert(id.to_owned(), hash.to_owned());
        }
    }

    pub fn ids(&self) -> &HashSet<String> {
        &self.ids
    }
    pub fn is_complete(&self) -> bool {
        self.complete
    }

    fn visit(&mut self) -> bool {
        self.visits += 1;
        if self.visits > 1_000_000 {
            self.complete = false;
            return false;
        }
        true
    }

    fn id(&mut self, value: &Value) {
        if !self.visit() {
            return;
        }
        let Some(id) = value.as_str() else {
            self.complete = false;
            return;
        };
        let bytes = id.as_bytes();
        if !(1..=128).contains(&bytes.len())
            || !bytes[0].is_ascii_alphanumeric()
            || !bytes
                .iter()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
        {
            self.complete = false;
            return;
        }
        if self.ids.len() >= 10_000 && !self.ids.contains(id) {
            self.complete = false;
            return;
        }
        self.ids.insert(id.to_owned());
    }

    fn note(&mut self, value: &Value, policy: &DesktopPrivacyPolicy, include_private: bool) {
        // Conflict payloads are untyped in AppData; check their note shape too.
        if !app_data::note_media_reference_shape_is_known(value) {
            self.complete = false;
        }
        self.snapshot(value, policy, include_private);
    }

    fn snapshot(&mut self, value: &Value, policy: &DesktopPrivacyPolicy, include_private: bool) {
        if !self.visit() {
            return;
        }
        let Some(object) = value.as_object() else {
            self.complete = false;
            return;
        };
        if include_private && object.get("encryption").is_some_and(|v| !v.is_null()) {
            self.include_private_declaration(
                policy.media_references_for(value).as_ref(),
                value
                    .get("updatedAtEpochMillis")
                    .and_then(Value::as_i64)
                    .unwrap_or(0),
            );
        }
        if let Some(attachments) = object.get("attachments") {
            if let Some(attachments) = attachments.as_array() {
                for attachment in attachments {
                    self.id(&attachment["id"]);
                }
            } else {
                self.complete = false;
            }
        }
        if let Some(ids) = object.get("attachmentIds") {
            if let Some(ids) = ids.as_array() {
                for id in ids {
                    self.id(id);
                }
            } else {
                self.complete = false;
            }
        }
        if let Some(document) = object.get("document") {
            self.document(document);
        }
        for key in ["revisions", "versions"] {
            if let Some(snapshots) = object.get(key) {
                if let Some(snapshots) = snapshots.as_array() {
                    for snapshot in snapshots {
                        self.snapshot(snapshot, policy, true);
                    }
                } else {
                    self.complete = false;
                }
            }
        }
    }

    pub(crate) fn include_private_declaration(
        &mut self,
        declaration: Option<&crate::sealed_media_references::SealedMediaReferences>,
        revision: i64,
    ) {
        let Some(declaration) = declaration else {
            self.mark_unknown_private(1);
            return;
        };
        for id in declaration.ids() {
            self.id(&Value::String(id.clone()));
            if self.ids.contains(id) {
                self.sealed_ids.insert(id.clone());
                if let Some(hash) = declaration.content_hashes().get(id) {
                    self.remember_content_hash(id, hash);
                }
                self.sealed_revisions
                    .entry(id.clone())
                    .and_modify(|old| *old = (*old).max(revision.max(0)))
                    .or_insert(revision.max(0));
            }
        }
    }

    pub(crate) fn mark_unknown_private(&mut self, count: usize) {
        if count > 0 {
            self.complete = false;
            self.unknown_sealed_notes = self.unknown_sealed_notes.saturating_add(count);
        }
    }

    fn document(&mut self, value: &Value) {
        if !self.visit() {
            return;
        }
        match value {
            Value::Object(object) => {
                if let Some(id) = object.get("attachmentId") {
                    if !id.is_null() && id != "" {
                        self.id(id);
                    }
                }
                for child in object.values() {
                    self.document(child);
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.document(item);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn local_media_reference_private_declaration_resolves_only_the_exact_note() {
        let mut data: Value = serde_json::from_str(&app_data::default_app_data_json(100)).unwrap();
        let plain = json!({
            "id": "local-reference-private", "title": "Private", "content": "",
            "attachments": [{"id":"private-image", "mimeType":"image/png",
                "sha256":"a".repeat(64), "sizeBytes":1, "updatedAtEpochMillis":100}],
            "updatedAtEpochMillis":100
        });
        let state = app_data::upsert_note_app_data_json(&data.to_string(), &plain.to_string(), 100)
            .unwrap();
        let note = serde_json::from_str::<Value>(&state).unwrap()["notes"][0].to_string();
        let (sealed, token) =
            crate::note_crypto::encrypt_note(&note, "local-private-reference-password").unwrap();
        let declaration = serde_json::from_str(
            &crate::note_crypto::session_media_references_json(&sealed, &token).unwrap(),
        )
        .unwrap();
        assert!(crate::note_crypto::close_session(&token));
        let sealed: Value = serde_json::from_str(&sealed).unwrap();
        data["notes"] = json!([sealed]);
        let empty = DesktopPrivacyPolicy::default();
        assert!(
            !DesktopMediaReferenceScope::from_snapshot(&data.to_string(), &empty).is_complete()
        );
        let policy = empty.including_sealed_media(&sealed, declaration).unwrap();
        let references = DesktopMediaReferenceScope::from_snapshot(&data.to_string(), &policy);
        assert!(references.is_complete());
        assert!(references.ids().contains("private-image"));
        data["notes"][0]["id"] = json!("different-note");
        assert!(
            !DesktopMediaReferenceScope::from_snapshot(&data.to_string(), &policy).is_complete()
        );
        data["notes"] = json!([]);
        data["schemaVersion"] = json!(0);
        data["futureReferenceContainer"] = json!(["unknown-file"]);
        assert!(
            !DesktopMediaReferenceScope::from_snapshot(&data.to_string(), &empty).is_complete()
        );
    }

    #[test]
    fn finance_profile_conflicts_do_not_make_note_media_references_incomplete() {
        let mut data: Value = serde_json::from_str(&app_data::default_app_data_json(100)).unwrap();
        data["syncConflictHistory"] = json!([{
            "id": "finance-profile-conflict",
            "entityType": "financeProfile",
            "entityId": "finance-profile",
            "losingRevisionEpochMillis": 90,
            "capturedAtEpochMillis": 100,
            "payload": {"profileRevision": 90}
        }]);

        let references = DesktopMediaReferenceScope::from_snapshot(
            &data.to_string(),
            &DesktopPrivacyPolicy::default(),
        );

        assert!(references.is_complete());
        assert!(references.ids().is_empty());
    }
}
