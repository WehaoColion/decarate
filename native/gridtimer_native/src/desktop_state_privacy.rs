// Windows startup - Reuse an audited head only while its reserved history stays read-only.
// v0.0.14 - Revalidate retained receipts through compact ciphertext identities in the owner transaction.
// v0.0.13 - Distinguish reference metadata updates from note and media privacy changes.
// v0.0.12 - Revalidate historical receipt bindings within the owner transaction.
// v0.0.11 - Exchange retained ciphertexts and reuse a single index per metadata batch.
// v0.0.10 - Upgrade private content declarations without weakening prior proofs.
// v0.0.9 - Export exact-envelope queries and reject incomplete metadata batches.
// v0.0.8 - Persist session references without scheduling unnecessary privacy vacuuming.
// v0.0.7 - Read account policy within the retained-media reservation.
// v0.0.6 - Accept private companion declarations without extending shared note JSON.
// v0.0.5 - Retain bounded sealed declarations and distinguish unresolved media cleanup from deletion fences.
// v0.0.4 - Retain proven attachment deletion fences independently of recovery JSON.
// v0.0.2 - Reuse byte-bound validation and avoid copying unchanged note payloads.
// v0.0.1 - Keep note privacy transitions durable across local recovery history.
use super::*;
use serde::Deserialize;

pub(super) const PRIVACY_TABLE: &str = "desktop_state_privacy_barriers";

/// Only sealed note records and deletion revisions are retained here. A later
/// intentional unseal can advance past a seal; recovery cannot forget a fence.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesktopPrivacyPolicy {
    seals: BTreeMap<String, Value>,
    deletions: BTreeMap<String, i64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    media_deletions: BTreeMap<String, i64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    media_candidates: BTreeMap<String, i64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    opaque_media_deletions: BTreeMap<String, i64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    sealed_media: BTreeMap<String, crate::sealed_media_references::SealedMediaReferences>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    note_attachment_detachments: crate::note_media_intent::NoteAttachmentDetachments,
}

#[derive(PartialEq)]
enum NoteRedaction {
    Unchanged,
    Replaced,
    Removed,
}

impl DesktopPrivacyPolicy {
    /// Reference receipts may not introduce encryption/deletion fences or
    /// replace pending cleanup work through a metadata-only storage path.
    pub(crate) fn has_same_non_reference_state(&self, other: &Self) -> bool {
        self.seals == other.seals
            && self.deletions == other.deletions
            && self.media_deletions == other.media_deletions
            && self.media_candidates == other.media_candidates
            && self.opaque_media_deletions == other.opaque_media_deletions
            && self.note_attachment_detachments == other.note_attachment_detachments
    }

    pub fn private_media_queries(
        &self,
        snapshot: &str,
        send_local: bool,
    ) -> DesktopStateStoreResult<Vec<crate::private_media_protocol::PrivateMediaQuery>> {
        let snapshot: Value = serde_json::from_str(snapshot)?;
        let index = crate::private_media_retained::RetainedSealedNotes::from_snapshot(&snapshot)
            .map_err(integrity)?;
        let mut queries = Vec::new();
        for (fingerprint, note) in index.records() {
            let mut query = crate::private_media_protocol::PrivateMediaQuery {
                note_id: note["id"].as_str().unwrap().to_owned(),
                envelope_sha256: fingerprint.to_owned(),
                declaration: None,
            };
            if send_local {
                query.declaration = self
                    .media_references_for(note)
                    .map(serde_json::to_value)
                    .transpose()?;
            }
            queries.push(query);
        }
        Ok(queries)
    }

    pub(crate) fn sealed_declaration_by_fingerprint(
        &self,
        fingerprint: &str,
    ) -> Option<crate::sealed_media_references::SealedMediaReferences> {
        self.sealed_media
            .get(fingerprint)
            .filter(|value| value.valid_metadata() && value.envelope_sha256() == fingerprint)
            .cloned()
    }

    /// Combine independently retained fences without letting an older recovery
    /// copy lower an encryption generation or forget a permanent deletion.
    pub fn merged_policy(&self, other: &Self) -> DesktopStateStoreResult<Self> {
        let mut value: Value = serde_json::from_str(&app_data::default_app_data_json(0))?;
        for (id, note) in &other.seals {
            if privacy_note_id(note)? != id {
                return Err(integrity("privacy seal identity does not match its key"));
            }
        }
        value["notes"] = Value::Array(other.seals.values().cloned().collect());
        value["tombstones"] = Value::Array(other.deletions.iter().map(|(id, revision)| {
            serde_json::json!({"entityType":"note", "entityId":id, "deletedAtEpochMillis":revision})
        }).collect());
        let mut merged = self
            .including_snapshot(&serde_json::to_string(&value)?)?
            .including_media_work(&other.media_candidates, &other.opaque_media_deletions)?
            .including_media_deletions(&other.media_deletions)?;
        for declaration in other.sealed_media.values() {
            merged.remember_sealed_media(declaration.clone())?;
        }
        merged.including_detachments(&other.note_attachment_detachments)
    }

    pub fn including_snapshot(&self, raw: &str) -> DesktopStateStoreResult<Self> {
        let mut next = self.clone();
        next.observe(raw)?;
        Ok(next)
    }

    pub fn is_empty(&self) -> bool {
        self.seals.is_empty()
            && self.deletions.is_empty()
            && self.media_deletions.is_empty()
            && self.media_candidates.is_empty()
            && self.opaque_media_deletions.is_empty()
            && self.sealed_media.is_empty()
            && self.note_attachment_detachments.is_empty()
    }

    pub(crate) fn note_attachment_detachments(
        &self,
    ) -> &crate::note_media_intent::NoteAttachmentDetachments {
        &self.note_attachment_detachments
    }

    pub(crate) fn including_detachments(
        &self,
        additions: &crate::note_media_intent::NoteAttachmentDetachments,
    ) -> DesktopStateStoreResult<Self> {
        let mut next = self.clone();
        crate::note_media_intent::merge_detachments(
            &mut next.note_attachment_detachments,
            additions,
        )
        .map_err(integrity)?;
        Ok(next)
    }

    /// Plan a revision only for an explicit user-selected recovery point. This
    /// does not change a fence or authorize a write; the commit still verifies
    /// owner, immutable attachment identity and the complete resulting state.
    pub fn next_explicit_attachment_restore_revision(
        &self,
        note_id: &str,
        attachment_ids: &[String],
        now: i64,
    ) -> DesktopStateStoreResult<i64> {
        self.validate_media_deletions()?;
        let mut planned = now.max(0);
        for id in attachment_ids {
            let scoped = self
                .note_attachment_detachments
                .get(note_id)
                .and_then(|entries| entries.get(id))
                .map(|intent| intent.detached_at_epoch_millis);
            for floor in scoped
                .into_iter()
                .chain(self.media_deletions.get(id).copied())
            {
                planned = planned.max(
                    floor
                        .checked_add(1)
                        .ok_or_else(|| integrity("attachment restore revision is exhausted"))?,
                );
            }
        }
        Ok(planned)
    }

    /// Apply note privacy plus durable attachment removals to a live/recovered
    /// head. Ordinary historical rows retain their attachment recovery material.
    pub fn project_current_json(&self, raw: &str) -> DesktopStateStoreResult<Option<String>> {
        let redacted = self.redact_json(raw)?;
        if self.note_attachment_detachments.is_empty() {
            return Ok(redacted);
        }
        let mut value = privacy_document(redacted.as_deref().unwrap_or(raw))?;
        let changed = crate::note_media_intent::project_current_detachments(
            &mut value,
            &self.note_attachment_detachments,
        )
        .map_err(integrity)?;
        if changed {
            Ok(Some(serde_json::to_string(&value)?))
        } else {
            Ok(redacted)
        }
    }

    pub(crate) fn media_deletions(&self) -> &BTreeMap<String, i64> {
        &self.media_deletions
    }

    pub(crate) fn media_candidates(&self) -> &BTreeMap<String, i64> {
        &self.media_candidates
    }
    pub(crate) fn opaque_media_deletions(&self) -> &BTreeMap<String, i64> {
        &self.opaque_media_deletions
    }

    pub(crate) fn media_references_for(
        &self,
        note: &Value,
    ) -> Option<crate::sealed_media_references::SealedMediaReferences> {
        use crate::sealed_media_references::SealedMediaReferences;
        let key = SealedMediaReferences::key_for_note(note)?;
        let declaration = self.sealed_media.get(&key)?;
        declaration
            .valid_for(note.get("id")?.as_str()?, note.get("encryption")?)
            .then(|| declaration.clone())
    }

    /// Called only with a separately authenticated owner's declaration and the
    /// exact retained record. A declaration is not inferred from shared JSON.
    pub(crate) fn including_sealed_media(
        &self,
        note: &Value,
        declaration: crate::sealed_media_references::SealedMediaReferences,
    ) -> DesktopStateStoreResult<Self> {
        let valid = note
            .get("id")
            .and_then(Value::as_str)
            .zip(note.get("encryption"))
            .is_some_and(|(id, envelope)| declaration.valid_for(id, envelope));
        if !valid {
            return Err(integrity(
                "sealed media declaration does not match retained note",
            ));
        }
        let mut next = self.clone();
        next.remember_sealed_media(declaration)?;
        Ok(next)
    }

    pub(crate) fn including_indexed_sealed_media(
        &self,
        index: &crate::desktop_private_media_index::DesktopPrivateMediaReferences,
        note_id: &str,
        declaration: crate::sealed_media_references::SealedMediaReferences,
    ) -> DesktopStateStoreResult<Self> {
        if !index.contains(note_id, declaration.envelope_sha256()) {
            return Err(integrity(
                "sealed reference ciphertext is absent from its retained index",
            ));
        }
        let mut next = self.clone();
        next.remember_sealed_media(declaration)?;
        Ok(next)
    }

    fn remember_sealed_media(
        &mut self,
        declaration: crate::sealed_media_references::SealedMediaReferences,
    ) -> DesktopStateStoreResult<()> {
        if !declaration.valid_metadata() {
            return Err(integrity("invalid sealed media declaration"));
        }
        let key = declaration.envelope_sha256().to_owned();
        if let Some(old) = self.sealed_media.get(&key) {
            if old.covers(&declaration) {
                return Ok(());
            }
            if !declaration.covers(old) {
                return Err(integrity(
                    "conflicting sealed media declarations for one envelope",
                ));
            }
        }
        self.sealed_media.insert(key, declaration);
        // Losing an old declaration only makes references unknown. It never
        // authorizes deletion. Keep this recovery aid bounded across autosaves.
        while self.sealed_media.len() > 4096
            || self
                .sealed_media
                .values()
                .map(|entry| entry.ids().len())
                .sum::<usize>()
                > 40_000
        {
            self.sealed_media.pop_first();
        }
        Ok(())
    }

    pub(crate) fn including_media_work(
        &self,
        candidates: &BTreeMap<String, i64>,
        opaque: &BTreeMap<String, i64>,
    ) -> DesktopStateStoreResult<Self> {
        validate_media_fence_entries(candidates)?;
        if opaque
            .iter()
            .any(|(id, revision)| id.is_empty() || *revision <= 0)
        {
            return Err(integrity("invalid opaque media deletion"));
        }
        let mut next = self.clone();
        for (id, revision) in candidates {
            next.media_candidates
                .entry(id.clone())
                .and_modify(|at| *at = (*at).max(*revision))
                .or_insert(*revision);
        }
        for (id, revision) in opaque {
            next.opaque_media_deletions
                .entry(id.clone())
                .and_modify(|at| *at = (*at).max(*revision))
                .or_insert(*revision);
        }
        next.media_candidates.retain(|id, revision| {
            next.media_deletions
                .get(id)
                .is_none_or(|fence| *fence < *revision)
        });
        if next.media_candidates.len() > 10_000 {
            return Err(integrity(
                "unresolved media cleanup identity limit exceeded",
            ));
        }
        Ok(next)
    }

    /// Only callers that checked all retained account references may add a
    /// media fence. Plain note encryption does not establish such a fence.
    pub(crate) fn including_media_deletions(
        &self,
        additions: &BTreeMap<String, i64>,
    ) -> DesktopStateStoreResult<Self> {
        self.validate_media_deletions()?;
        validate_media_fence_entries(additions)?;
        let mut next = self.clone();
        for (id, revision) in additions {
            next.media_deletions
                .entry(id.clone())
                .and_modify(|at| *at = (*at).max(*revision))
                .or_insert(*revision);
        }
        next.media_candidates.retain(|id, revision| {
            next.media_deletions
                .get(id)
                .is_none_or(|fence| *fence < *revision)
        });
        Ok(next)
    }

    pub(crate) fn validate_media_deletions(&self) -> DesktopStateStoreResult<()> {
        crate::note_media_intent::validate_detachments(&self.note_attachment_detachments)
            .map_err(integrity)?;
        validate_media_fence_entries(&self.media_deletions)?;
        validate_media_fence_entries(&self.media_candidates)?;
        if self.media_candidates.len() > 10_000
            || self.sealed_media.len() > 4096
            || self
                .sealed_media
                .values()
                .map(|entry| entry.ids().len())
                .sum::<usize>()
                > 40_000
            || self
                .sealed_media
                .iter()
                .any(|(key, value)| key != value.envelope_sha256() || !value.valid_metadata())
            || self
                .opaque_media_deletions
                .iter()
                .any(|(id, revision)| id.is_empty() || *revision <= 0)
        {
            return Err(integrity(
                "invalid or oversized sealed media recovery metadata",
            ));
        }
        Ok(())
    }

    fn observe(&mut self, raw: &str) -> DesktopStateStoreResult<()> {
        let value = privacy_document(raw)?;
        self.observe_document(&value)
    }

    fn observe_document(&mut self, value: &Value) -> DesktopStateStoreResult<()> {
        let detachments =
            crate::note_media_intent::infer_snapshot_detachments(value).map_err(integrity)?;
        crate::note_media_intent::merge_detachments(
            &mut self.note_attachment_detachments,
            &detachments,
        )
        .map_err(integrity)?;
        for note in value["notes"].as_array().into_iter().flatten() {
            if note.get("encryption").is_none_or(Value::is_null) {
                continue;
            }
            if !note["title"].as_str().unwrap_or("").is_empty()
                || !note["content"].as_str().unwrap_or("").is_empty()
                || ["attachments", "revisions", "versions"]
                    .iter()
                    .any(|key| note[*key].as_array().is_some_and(|v| !v.is_empty()))
                || !note["document"]["richTextPlainText"]
                    .as_str()
                    .unwrap_or("")
                    .is_empty()
                || note["document"]
                    .get("knowledge")
                    .is_some_and(|value| !value.is_null())
                || note["document"]["blocks"]
                    .as_array()
                    .is_some_and(|v| !v.is_empty())
            {
                return Err(integrity("sealed privacy record still contains plaintext"));
            }
            let id = privacy_note_id(note)?;
            let generation = privacy_generation(note)?;
            if generation <= 0 {
                return Err(integrity("invalid sealed privacy generation"));
            }
            if self
                .seals
                .get(id)
                .map(privacy_generation)
                .transpose()?
                .unwrap_or(-1)
                < generation
            {
                self.seals.insert(id.to_owned(), note.clone());
            }
        }
        for tombstone in value["tombstones"].as_array().into_iter().flatten() {
            if tombstone["entityType"] != "note" {
                continue;
            }
            let id = tombstone["entityId"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| integrity("invalid note deletion identity"))?;
            let revision = tombstone["deletedAtEpochMillis"]
                .as_i64()
                .filter(|r| *r >= 0)
                .ok_or_else(|| integrity("invalid note deletion revision"))?;
            self.deletions
                .entry(id.to_owned())
                .and_modify(|r| *r = (*r).max(revision))
                .or_insert(revision);
        }
        self.seals.retain(|id, note| {
            !self.deletions.get(id).is_some_and(|revision| {
                note["updatedAtEpochMillis"].as_i64().unwrap_or(0) <= *revision
            })
        });
        Ok(())
    }

    /// Return None when the exact input bytes can be kept. Parse unknown or
    /// damaged documents strictly, without discarding unrelated recovery data.
    pub fn redact_json(&self, raw: &str) -> DesktopStateStoreResult<Option<String>> {
        if self.is_empty() {
            return Ok(None);
        }
        let mut value = privacy_document(raw)?;
        if self.redact_document(&mut value)? {
            Ok(Some(serde_json::to_string(&value)?))
        } else {
            Ok(None)
        }
    }

    fn redact_document(&self, value: &mut Value) -> DesktopStateStoreResult<bool> {
        if self.is_empty() {
            return Ok(false);
        }
        let mut changed = false;
        if let Some(notes) = value.get_mut("notes").and_then(Value::as_array_mut) {
            let mut retained = Vec::with_capacity(notes.len());
            for mut note in std::mem::take(notes) {
                match self.redact_note(&mut note)? {
                    NoteRedaction::Removed => {
                        changed = true;
                        continue;
                    }
                    NoteRedaction::Replaced => changed = true,
                    NoteRedaction::Unchanged => {}
                }
                retained.push(note);
            }
            *notes = retained;
        }
        // Conflict payloads are also recovery copies of a complete losing note.
        if let Some(history) = value
            .get_mut("syncConflictHistory")
            .and_then(Value::as_array_mut)
        {
            let mut retained = Vec::with_capacity(history.len());
            for mut conflict in std::mem::take(history) {
                if conflict["entityType"] == "note" {
                    let payload = conflict
                        .get_mut("payload")
                        .ok_or_else(|| integrity("note conflict has no payload"))?;
                    if self.redact_note(payload)? != NoteRedaction::Unchanged {
                        changed = true;
                        continue;
                    }
                }
                retained.push(conflict);
            }
            *history = retained;
        }
        if !changed {
            return Ok(false);
        }
        // Redacted history must continue carrying the deletion evidence when
        // selected for recovery and subsequently synchronized.
        let tombstones = value
            .as_object_mut()
            .ok_or_else(|| integrity("invalid privacy root"))?
            .entry("tombstones")
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| integrity("invalid privacy tombstones"))?;
        for (id, revision) in &self.deletions {
            if !tombstones.iter().any(|t| {
                t["entityType"] == "note"
                    && t["entityId"] == *id
                    && t["deletedAtEpochMillis"].as_i64().unwrap_or(-1) >= *revision
            }) {
                tombstones.retain(|t| !(t["entityType"] == "note" && t["entityId"] == *id));
                tombstones.push(serde_json::json!({"entityType":"note", "entityId":id, "deletedAtEpochMillis":revision}));
            }
        }
        Ok(true)
    }

    fn redact_note(&self, note: &mut Value) -> DesktopStateStoreResult<NoteRedaction> {
        let id = privacy_note_id(note)?;
        if self
            .deletions
            .get(id)
            .is_some_and(|revision| note["updatedAtEpochMillis"].as_i64().unwrap_or(0) <= *revision)
        {
            return Ok(NoteRedaction::Removed);
        }
        let Some(sealed) = self.seals.get(id) else {
            return Ok(NoteRedaction::Unchanged);
        };
        let generation = privacy_generation(note)?;
        let protected_generation = privacy_generation(sealed)?;
        if generation < protected_generation {
            *note = sealed.clone();
            return Ok(NoteRedaction::Replaced);
        } else if generation == protected_generation
            && note.get("encryption").is_none_or(Value::is_null)
        {
            return Err(integrity(
                "plaintext conflicts with a sealed privacy generation",
            ));
        }
        Ok(NoteRedaction::Unchanged)
    }
}

fn privacy_note_id(note: &Value) -> DesktopStateStoreResult<&str> {
    note["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| integrity("invalid privacy note identity"))
}

fn privacy_generation(note: &Value) -> DesktopStateStoreResult<i64> {
    let state = note
        .get("protectionStateRevision")
        .map(|v| v.as_i64())
        .unwrap_or(Some(0))
        .filter(|v| *v >= 0)
        .ok_or_else(|| integrity("invalid privacy generation"))?;
    if let Some(envelope) = note.get("encryption").filter(|v| !v.is_null()) {
        let generation = envelope["protectionRevision"]
            .as_i64()
            .filter(|v| *v > 0)
            .ok_or_else(|| integrity("invalid privacy envelope generation"))?;
        if state != 0 && state != generation {
            return Err(integrity("split privacy generation"));
        }
        Ok(generation)
    } else {
        Ok(state)
    }
}

fn privacy_document(raw: &str) -> DesktopStateStoreResult<Value> {
    // The bounded cache contains semantic metadata, never document plaintext.
    // Its key hashes the bytes supplied here; row/owner integrity decisions
    // remain uncached. Saves already need this same strict analysis.
    analyze_app_data_json_with_value(raw, 0).map(|(_, value)| value)
}

pub(super) fn create_privacy_schema(connection: &Connection) -> DesktopStateStoreResult<()> {
    connection.execute_batch(
        "CREATE TABLE desktop_state_privacy_barriers (
        owner TEXT NOT NULL PRIMARY KEY,
        policy_json TEXT NOT NULL CHECK(json_valid(policy_json)),
        binding_sha256 TEXT NOT NULL CHECK(length(binding_sha256) = 64),
        cleanup_pending INTEGER NOT NULL CHECK(cleanup_pending IN (0, 1)),
        FOREIGN KEY(owner) REFERENCES desktop_state_owners(owner)
            ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED
    ) STRICT;",
    )?;
    Ok(())
}

fn privacy_binding(owner: &str, json: &str) -> String {
    sha256_hex(
        &serde_json::to_vec(&("desktop-privacy-v1", owner, json)).expect("serializable binding"),
    )
}

pub(super) fn read_privacy_policy(
    connection: &Connection,
    owner: &str,
) -> DesktopStateStoreResult<(DesktopPrivacyPolicy, bool)> {
    let record = connection
        .query_row(
            "SELECT policy_json, binding_sha256, cleanup_pending
        FROM desktop_state_privacy_barriers WHERE owner = ?1",
            params![owner],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((json, binding, pending)) = record else {
        return Ok((DesktopPrivacyPolicy::default(), false));
    };
    if binding != privacy_binding(owner, &json) || !matches!(pending, 0 | 1) {
        return Err(integrity("privacy barrier binding is invalid"));
    }
    let policy: DesktopPrivacyPolicy = serde_json::from_str(&json)?;
    policy.validate_media_deletions()?;
    for (id, note) in &policy.seals {
        if privacy_note_id(note)? != id
            || privacy_generation(note)? <= 0
            || note["encryption"].is_null()
        {
            return Err(integrity("privacy barrier contains an invalid seal"));
        }
    }
    if policy
        .deletions
        .iter()
        .any(|(id, revision)| id.is_empty() || *revision < 0)
    {
        return Err(integrity("privacy barrier contains an invalid deletion"));
    }
    Ok((policy, pending != 0))
}

pub(super) fn prepare_privacy_record(
    connection: &Connection,
    owner: &str,
    incoming: &str,
) -> DesktopStateStoreResult<bool> {
    prepare_privacy_transition(connection, owner, incoming)?.commit()
}

/// A transaction-local plan. It retains the exact connection on which the
/// previous policy was read; it cannot be applied to another journal or owner.
pub(super) struct PreparedPrivacyTransition<'transaction> {
    connection: &'transaction Connection,
    owner: String,
    previous: DesktopPrivacyPolicy,
    prospective: DesktopPrivacyPolicy,
}

pub(super) fn prepare_privacy_transition<'transaction>(
    connection: &'transaction Connection,
    owner: &str,
    incoming: &str,
) -> DesktopStateStoreResult<PreparedPrivacyTransition<'transaction>> {
    let (previous, _) = read_privacy_policy(connection, owner)?;
    let mut policy = previous.clone();
    // Strictly parse the exact incoming bytes once for this operation. The
    // document stays on this stack and never enters the shared metadata cache.
    // Both fence observation and restoration checks consume this same value.
    let mut value = privacy_document(incoming)?;
    policy.observe_document(&value)?;
    let behind_note_barrier = policy.redact_document(&mut value)?;
    let behind_attachment_barrier = !behind_note_barrier
        && !policy.note_attachment_detachments.is_empty()
        && crate::note_media_intent::project_current_detachments(
            &mut value,
            &policy.note_attachment_detachments,
        )
        .map_err(integrity)?;
    if behind_note_barrier || behind_attachment_barrier {
        return Err(integrity(
            "incoming snapshot would restore content behind a note privacy barrier",
        ));
    }
    Ok(PreparedPrivacyTransition {
        connection,
        owner: owner.to_owned(),
        previous,
        prospective: policy,
    })
}

impl PreparedPrivacyTransition<'_> {
    pub(super) fn policy(&self) -> &DesktopPrivacyPolicy {
        &self.prospective
    }

    pub(super) fn commit(self) -> DesktopStateStoreResult<bool> {
        if self.prospective == self.previous {
            return Ok(false);
        }
        redact_privacy_rows(self.connection, &self.owner, &self.prospective)?;
        let json = serde_json::to_string(&self.prospective)?;
        self.connection.execute("INSERT INTO desktop_state_privacy_barriers(owner, policy_json, binding_sha256, cleanup_pending)
        VALUES (?1, ?2, ?3, 1) ON CONFLICT(owner) DO UPDATE SET policy_json=excluded.policy_json,
        binding_sha256=excluded.binding_sha256, cleanup_pending=1", params![self.owner, json, privacy_binding(&self.owner, &json)])?;
        Ok(true)
    }
}

pub(super) fn record_media_declarations(
    connection: &Connection,
    owner: &str,
    session_scope: &str,
    snapshot: &str,
    declarations: &[DesktopSealedMediaDeclaration],
) -> DesktopStateStoreResult<bool> {
    if declarations.is_empty() {
        return Ok(false);
    }
    for declaration in declarations {
        declaration.validate_authority(owner, session_scope)?;
    }
    let (previous, pending) = read_privacy_policy(connection, owner)?;
    let current_index =
        crate::desktop_private_media_index::DesktopPrivateMediaReferences::from_snapshot(snapshot)
            .map_err(integrity)?;
    let needs_history = declarations.iter().any(|declaration| {
        declaration.permits_retained_snapshot()
            && declaration
                .apply_to_index(
                    owner,
                    session_scope,
                    &DesktopPrivacyPolicy::default(),
                    &current_index,
                )
                .is_err()
    });
    let retained = if needs_history {
        Some(media_references::private_media_references(
            connection, owner, snapshot,
        )?)
    } else {
        None
    };
    let index = retained.as_ref().unwrap_or(&current_index);
    let mut policy = previous.clone();
    for declaration in declarations {
        policy = declaration.apply_to_index(owner, session_scope, &policy, index)?;
    }
    // Later entries may evict earlier ones. Verify the whole batch before commit.
    for declaration in declarations {
        if declaration.apply_to_index(owner, session_scope, &policy, index)? != policy {
            return Err(integrity(
                "private reference batch exceeds retained metadata capacity",
            ));
        }
    }
    if previous == policy {
        return Ok(false);
    }
    let json = serde_json::to_string(&policy)?;
    connection.execute("INSERT INTO desktop_state_privacy_barriers(owner, policy_json, binding_sha256, cleanup_pending)
        VALUES (?1, ?2, ?3, ?4) ON CONFLICT(owner) DO UPDATE SET policy_json=excluded.policy_json,
        binding_sha256=excluded.binding_sha256, cleanup_pending=excluded.cleanup_pending",
        params![owner, json, privacy_binding(owner, &json), i64::from(pending)])?;
    Ok(true)
}

fn redact_privacy_rows(
    connection: &Connection,
    owner: &str,
    policy: &DesktopPrivacyPolicy,
) -> DesktopStateStoreResult<()> {
    let mut last_id = 0;
    loop {
        let next = connection.query_row("SELECT id FROM desktop_state_snapshots WHERE owner=?1 AND id>?2 ORDER BY id LIMIT 1",
            params![owner, last_id], |row| row.get::<_, i64>(0)).optional()?;
        let Some(id) = next else {
            break;
        };
        last_id = id;
        let snapshot = read_snapshot_by_id(connection, id)?
            .ok_or_else(|| integrity("privacy snapshot disappeared"))?;
        verify_snapshot(&snapshot, Some(owner), 0)?;
        let Some(redacted) = policy.redact_json(&snapshot.app_data_json)? else {
            continue;
        };
        if read_scope_media_transaction_proof(connection, &snapshot.source)?
            .is_some_and(|proof| proof.pinned_snapshot_id.is_some())
        {
            return Err(integrity(
                "privacy cleanup is waiting for an active media transaction",
            ));
        }
        let analysis = analyze_app_data_json(&redacted, 0)?;
        let raw_digest = sha256_hex(redacted.as_bytes());
        let envelope = snapshot_envelope_sha256(
            owner,
            analysis.schema_version,
            analysis.revision,
            analysis.item_count,
            &analysis.semantic_summary,
            &raw_digest,
            &analysis.canonical_json_sha256,
            &snapshot.sync_state_sha256,
            &snapshot.parent_envelope_sha256,
            snapshot.created_at_epoch_millis,
            &snapshot.source,
        );
        connection.execute("UPDATE desktop_state_snapshots SET app_data_json=?1, schema_version=?2, revision=?3,
            item_count=?4, semantic_summary=?5, raw_sha256=?6, canonical_json_sha256=?7, envelope_sha256=?8 WHERE id=?9",
            params![redacted, analysis.schema_version, analysis.revision, analysis.item_count, analysis.semantic_summary,
                raw_digest, analysis.canonical_json_sha256, envelope, id])?;
        // Completed media proof identities stay spent; bind their sanitized
        // evidence to the rewritten row. Active pins were rejected above.
        connection.execute("UPDATE desktop_state_scope_media_transaction_proofs SET raw_sha256=?1, envelope_sha256=?2
            WHERE snapshot_id=?3 AND pinned_snapshot_id IS NULL", params![raw_digest, envelope, id])?;
        verify_snapshot(
            &read_snapshot_by_id(connection, id)?
                .ok_or_else(|| integrity("redacted snapshot disappeared"))?,
            Some(owner),
            0,
        )?;
    }
    let mut last_id = 0;
    loop {
        let row = connection
            .query_row(
                "SELECT id, app_data_json, schema_version FROM desktop_state_snapshot_quarantine
            WHERE owner=?1 AND id>?2 ORDER BY id LIMIT 1",
                params![owner, last_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((id, raw, declared_schema)) = row else {
            break;
        };
        last_id = id;
        if declared_schema > i64::from(app_data::APP_DATA_SCHEMA_VERSION) {
            return Err(integrity(
                "privacy cleanup cannot rewrite future-schema quarantine data",
            ));
        }
        if let Some(redacted) = policy.redact_json(&raw)? {
            connection.execute("UPDATE desktop_state_snapshot_quarantine SET app_data_json=?1,
                observed_raw_sha256=?2, reason=reason || '; note privacy redaction applied' WHERE id=?3",
                params![redacted, sha256_hex(redacted.as_bytes()), id])?;
        }
    }
    Ok(())
}

impl DesktopStateStore {
    pub fn privacy_policy(
        &self,
        owner: &str,
    ) -> DesktopStateStoreResult<(DesktopPrivacyPolicy, bool)> {
        validate_owner(owner)?;
        read_privacy_policy(&self.open_connection(false)?, owner)
    }

    /// Repair pre-upgrade history before presenting any recovered workspace.
    /// The current note and redacted older rows share one SQLite transaction.
    pub fn repair_privacy_history(&self, owner: &str) -> DesktopStateStoreResult<()> {
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        repair_privacy_history_in_transaction(&transaction, owner)?;
        transaction.commit()?;
        Ok(())
    }

    /// Call only after every managed JSON mirror has also been rewritten.
    /// VACUUM removes legacy free-page remnants; the checked TRUNCATE checkpoint
    /// removes old WAL frames. Failure leaves the durable retry flag set.
    pub fn finish_privacy_cleanup(&self, owner: &str) -> DesktopStateStoreResult<()> {
        let mut connection = self.open_connection(false)?;
        let (policy, pending) = read_privacy_policy(&connection, owner)?;
        if !pending {
            return Ok(());
        }
        let binding = privacy_binding(owner, &serde_json::to_string(&policy)?);
        connection.execute_batch("VACUUM;")?;
        privacy_checkpoint(&connection)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if transaction.execute(
            "UPDATE desktop_state_privacy_barriers SET cleanup_pending=0
            WHERE owner=?1 AND binding_sha256=?2",
            params![owner, binding],
        )? != 1
        {
            return Err(integrity(
                "privacy policy advanced during cleanup; retry required",
            ));
        }
        transaction.commit()?;
        privacy_checkpoint(&connection)
    }
}

pub(super) fn repair_privacy_history_in_transaction(
    connection: &Connection,
    owner: &str,
) -> DesktopStateStoreResult<()> {
    let latest = latest_valid_in_transaction(connection, owner, 0)?;
    repair_privacy_history_from_head_in_transaction(connection, owner, latest).map(|_| ())
}

/// Reuse only a head fully audited on this reserved connection. The caller
/// discards an audit hint after quarantine or any intervening SQL write.
/// This function never lends pre-redaction bytes to startup after a write:
/// privacy repair and trigger effects require a fresh verified row read.
pub(super) fn repair_privacy_history_from_head_in_transaction(
    connection: &Connection,
    owner: &str,
    latest: Option<DesktopStateSnapshot>,
) -> DesktopStateStoreResult<Option<DesktopStateSnapshot>> {
    if latest
        .as_ref()
        .is_some_and(|snapshot| snapshot.owner != owner)
    {
        return Err(integrity(
            "privacy repair head belongs to a different owner",
        ));
    }
    let changes_before_repair = connection.total_changes();
    let evidence = read_journal_evidence(connection)?;
    if let Some(latest) = latest.as_ref() {
        let previous = read_privacy_policy(connection, owner)?.0;
        let policy = previous.including_snapshot(&latest.app_data_json)?;
        let redacted = policy.project_current_json(&latest.app_data_json)?;
        // When observation and projection are both unchanged, preparing the
        // identical snapshot again cannot add a fence. A repair keeps the
        // established path whenever either the policy or current bytes change.
        if (policy != previous || redacted.is_some())
            && prepare_privacy_record(
                connection,
                owner,
                redacted.as_deref().unwrap_or(&latest.app_data_json),
            )?
        {
            advance_metadata_commit_sequence(connection, &evidence)?;
        }
    }
    if connection.total_changes() == changes_before_repair {
        Ok(latest)
    } else {
        latest_valid_in_transaction(connection, owner, 0)
    }
}

fn privacy_checkpoint(connection: &Connection) -> DesktopStateStoreResult<()> {
    let (busy, log, checkpointed) =
        connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
    if busy != 0 || log != 0 || checkpointed != 0 {
        return Err(integrity("privacy checkpoint is still busy"));
    }
    Ok(())
}

fn validate_media_fence_entries(entries: &BTreeMap<String, i64>) -> DesktopStateStoreResult<()> {
    for (id, revision) in entries {
        let bytes = id.as_bytes();
        if !(1..=128).contains(&bytes.len())
            || !bytes[0].is_ascii_alphanumeric()
            || !bytes
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            || *revision <= 0
        {
            return Err(integrity("invalid attachment privacy fence"));
        }
    }
    Ok(())
}
