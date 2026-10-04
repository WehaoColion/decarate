// v0.0.5 - Use the reference-only commit path for direct declaration registration.
// v0.0.4 - Protect ordinary deletion and retention GC with current and opaque history references.
// v0.0.3 - Preserve opaque sealed references and retain unresolved cleanup work explicitly.
// v0.0.2 - Return durable account-scoped media fences for recovery and stale uploads.
use super::*;
use crate::desktop_state_store::DesktopPrivacyPolicy;
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Candidates {
    ids: HashMap<String, i64>,
    opaque_notes: BTreeMap<String, i64>,
}
impl Candidates {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.ids.is_empty() && self.opaque_notes.is_empty()
    }

    pub(super) fn from_ids(ids: &BTreeMap<String, i64>) -> StoreResult<Self> {
        let mut result = Self::new();
        for (id, revision) in ids {
            if !valid_media_attachment_id(id) || *revision <= 0 {
                return Err(StoreError::Integrity(
                    "invalid media deletion candidate".into(),
                ));
            }
            observe(&mut result, id, *revision);
        }
        Ok(result)
    }
}

#[derive(Default)]
pub(super) struct Outcome {
    pub(super) fences: BTreeMap<String, i64>,
    pub(super) candidates: BTreeMap<String, i64>,
    pub(super) opaque_notes: BTreeMap<String, i64>,
}

#[derive(Default)]
struct References {
    ids: HashSet<String>,
    unknown: bool,
}

fn document(raw: &str) -> StoreResult<serde_json::Value> {
    if raw.trim().is_empty() {
        Ok(serde_json::json!({}))
    } else {
        serde_json::from_str(raw).map_err(StoreError::from)
    }
}

fn observe(candidates: &mut Candidates, id: &str, revision: i64) {
    if valid_media_attachment_id(id) && revision > 0 {
        candidates
            .ids
            .entry(id.to_owned())
            .and_modify(|r| *r = (*r).max(revision))
            .or_insert(revision);
    }
}

fn note_snapshots(value: &serde_json::Value, mut visit: impl FnMut(&serde_json::Value)) -> bool {
    let mut complete = value.is_object()
        && value.get("notes").is_none_or(serde_json::Value::is_array)
        && value
            .get("syncConflictHistory")
            .is_none_or(serde_json::Value::is_array);
    for note in value["notes"].as_array().into_iter().flatten() {
        visit(note);
    }
    for conflict in value["syncConflictHistory"]
        .as_array()
        .into_iter()
        .flatten()
    {
        match conflict
            .get("entityType")
            .and_then(serde_json::Value::as_str)
        {
            Some("note") => match validated_note_conflict_payload(conflict) {
                Some(note) => visit(note),
                None => complete = false,
            },
            None
            | Some(
                "slot"
                | "category"
                | "session"
                | "archivedTask"
                | "noteFolder"
                | "financeDayLedger"
                | "financeMonthSnapshot",
            ) => {}
            Some(_) => complete = false,
        }
    }
    complete
}

fn note_references(
    note: &serde_json::Value,
    scope: &mut References,
    policy: &DesktopPrivacyPolicy,
) {
    // A future or malformed note can hide references even when its visible
    // arrays look empty. Preserve visible IDs, but never grant deletion from it.
    if !crate::app_data::note_media_reference_shape_is_known(note) {
        scope.unknown = true;
    }
    if note.get("encryption").is_some_and(|value| !value.is_null()) {
        match policy.media_references_for(note) {
            Some(declaration) => scope.ids.extend(declaration.ids().iter().cloned()),
            None => scope.unknown = true,
        }
    }
    collect_note_snapshot_attachment_ids(note, &mut scope.ids);
    for field in ["revisions", "versions"] {
        for snapshot in note[field].as_array().into_iter().flatten() {
            collect_note_snapshot_attachment_ids(snapshot, &mut scope.ids);
        }
    }
}

fn references(value: &serde_json::Value, policy: &DesktopPrivacyPolicy) -> References {
    let mut scope = References::default();
    let complete = note_snapshots(value, |note| note_references(note, &mut scope, policy));
    scope.unknown |= !complete
        || value.get("schemaVersion").is_some_and(|version| {
            version
                .as_i64()
                .is_none_or(|version| version < 0 || version > i64::from(APP_DATA_SCHEMA_VERSION))
        });
    scope
}

pub(super) struct HistoricalReferenceScope {
    pub(super) ids: HashSet<String>,
    pub(super) sealed_hashes: HashMap<String, String>,
    pub(super) unresolved: bool,
}

pub(super) fn historical_reference_scope(
    raw: &str,
    policy: &DesktopPrivacyPolicy,
) -> StoreResult<HistoricalReferenceScope> {
    let value = document(raw)?;
    let scope = references(&value, policy);
    let mut result = HistoricalReferenceScope {
        ids: scope.ids,
        sealed_hashes: HashMap::new(),
        unresolved: scope.unknown || reference_format_unknown(raw, &value),
    };
    note_snapshots(&value, |note| {
        if note
            .get("encryption")
            .is_none_or(serde_json::Value::is_null)
        {
            return;
        }
        let Some(declaration) = policy.media_references_for(note) else {
            result.unresolved = true;
            return;
        };
        result.unresolved |= !declaration.has_complete_content_hashes();
        for id in declaration.ids() {
            let Some(hash) = declaration.content_hashes().get(id) else {
                result.unresolved = true;
                continue;
            };
            if result
                .sealed_hashes
                .get(id)
                .is_some_and(|previous| previous != hash)
            {
                result.unresolved = true;
            } else {
                result.sealed_hashes.insert(id.clone(), hash.clone());
            }
        }
    });
    Ok(result)
}

/// Bound to the caller's account revision. This describes the supplied merge
/// view only; independent retained database snapshots are checked by DELETE/GC.
#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) struct MediaDeletionReferenceScope {
    pub(crate) referenced_ids: HashSet<String>,
    pub(crate) unresolved: bool,
}

impl SqliteServerStore {
    pub(crate) fn media_deletion_reference_scope(
        &self,
        user: &str,
        expected_account_revision: i64,
        combined_post_note_deletion_json: &str,
    ) -> StoreResult<MediaDeletionReferenceScope> {
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let current = read_account_in_transaction(&transaction, user)?
            .ok_or_else(|| StoreError::NotFound("media reference account is missing".into()))?;
        if current.revision != expected_account_revision {
            return Err(StoreError::RevisionConflict {
                expected_revision: expected_account_revision,
                actual_revision: current.revision,
            });
        }
        let policy = note_privacy::read_policy(&transaction, user)?;
        let value = document(combined_post_note_deletion_json)?;
        let scope = references(&value, &policy);
        Ok(MediaDeletionReferenceScope {
            referenced_ids: scope.ids,
            unresolved: scope.unknown
                || reference_format_unknown(combined_post_note_deletion_json, &value),
        })
    }
}

/// The projected note tombstone proves deletion. Encryption alone is not proof
/// that an external file was encrypted, and does not authorize removing it.
pub(super) fn observe_projection(
    candidates: &mut Candidates,
    before: &str,
    after: &str,
    policy: &DesktopPrivacyPolicy,
) -> StoreResult<()> {
    let old: serde_json::Value = serde_json::from_str(before)?;
    let projected: serde_json::Value = serde_json::from_str(after)?;
    let mut deleted = HashMap::<String, i64>::new();
    for tombstone in projected["tombstones"].as_array().into_iter().flatten() {
        if tombstone["entityType"] == "note" {
            if let (Some(id), Some(revision)) = (
                tombstone["entityId"].as_str(),
                tombstone["deletedAtEpochMillis"].as_i64(),
            ) {
                deleted
                    .entry(id.to_owned())
                    .and_modify(|r| *r = (*r).max(revision))
                    .or_insert(revision);
            }
        }
    }
    note_snapshots(&old, |note| {
        let Some(revision) = note["id"].as_str().and_then(|id| deleted.get(id)) else {
            return;
        };
        if note["updatedAtEpochMillis"].as_i64().unwrap_or(0) > *revision {
            return;
        }
        let mut scope = References::default();
        note_references(note, &mut scope, policy);
        if scope.unknown {
            candidates
                .opaque_notes
                .entry(note["id"].as_str().unwrap().to_owned())
                .and_modify(|at| *at = (*at).max(*revision))
                .or_insert(*revision);
        }
        for id in scope.ids {
            observe(candidates, &id, *revision);
        }
    });
    Ok(())
}

/// Runs in the same IMMEDIATE transaction as the account/privacy mutation.
/// Current JSON also repairs stores already redacted by earlier releases.
pub(super) fn purge(
    transaction: &Transaction<'_>,
    user: &str,
    mut candidates: Candidates,
    policy: &DesktopPrivacyPolicy,
) -> StoreResult<Outcome> {
    let known = policy.media_deletions();
    for (id, revision) in policy.media_candidates() {
        observe(&mut candidates, id, *revision);
    }
    for (id, revision) in known {
        observe(&mut candidates, id, *revision);
    }
    let Some(current) = read_account_in_transaction(transaction, user)? else {
        return Ok(Default::default());
    };
    let value = document(&current.app_data_json)?;
    for tombstone in value["tombstones"].as_array().into_iter().flatten() {
        if tombstone["entityType"] == "noteMedia" {
            if let (Some(id), Some(revision)) = (
                tombstone["entityId"].as_str(),
                tombstone["deletedAtEpochMillis"].as_i64(),
            ) {
                observe(&mut candidates, id, revision);
            }
        }
    }
    let mut outcome = Outcome {
        candidates: candidates
            .ids
            .iter()
            .map(|(id, at)| (id.clone(), *at))
            .collect(),
        opaque_notes: candidates.opaque_notes,
        ..Outcome::default()
    };
    let current_refs = references(&value, policy);
    let mut unknown = current_refs.unknown;
    candidates
        .ids
        .retain(|id, _| !current_refs.ids.contains(id));
    // Avoid decompressing history when no old bytes or missing barrier need work.
    let mut pending = HashMap::new();
    for (id, revision) in candidates.ids {
        let live_revision: Option<i64> = transaction.query_row(
            "SELECT updated_at_epoch_millis FROM note_media WHERE user_id=?1 AND attachment_id=?2",
            params![user, id], |row| row.get(0)).optional()?;
        if live_revision.is_some_and(|live| live > revision) {
            continue;
        }
        if known.get(&id).is_none_or(|at| *at < revision)
            || live_revision.is_some()
            || media_tombstone_revision(transaction, user, &id)?.is_none_or(|at| at < revision)
        {
            pending.insert(id, revision);
        }
    }
    if pending.is_empty() {
        return Ok(outcome);
    }
    let mut last = -1;
    loop {
        let stored = transaction.query_row(
            "SELECT h.user_id,h.revision,h.created_at_epoch_millis,c.sha256,c.compression,c.uncompressed_size_bytes,c.compressed_size_bytes,c.content
             FROM account_snapshot_history h JOIN snapshot_contents c ON c.sha256=h.content_sha256
             WHERE h.user_id=?1 AND h.revision>?2 ORDER BY h.revision LIMIT 1",
            params![user, last], snapshot_content_row).optional()?;
        let Some(stored) = stored else {
            break;
        };
        let history = decode_snapshot_history(stored)?;
        verify_snapshot_history_entry(&history)?;
        last = history.revision;
        let retained = references(&document(&history.app_data_json)?, policy);
        unknown |= retained.unknown;
        pending.retain(|id, _| !retained.ids.contains(id));
        if pending.is_empty() {
            return Ok(outcome);
        }
    }
    // Unknown ciphertext is not evidence that an attachment is unreferenced.
    // Keep the candidate identity for later retry after reference resolution.
    if unknown {
        return Ok(outcome);
    }
    if current_schema_version(transaction)? >= 13 {
        validate_media_identity_batch_quota(transaction, user, &pending.keys().cloned().collect())?;
    }
    outcome.fences = pending
        .iter()
        .map(|(id, revision)| (id.clone(), *revision))
        .collect();
    let mut removed = 0;
    for (id, revision) in pending {
        // Do not remove a live file belonging to another user, or one uploaded
        // after this deletion. Shared historical hashes use existing refcounts.
        transaction.execute(
            "INSERT INTO note_media_tombstones(user_id,attachment_id,deleted_revision_epoch_millis,recorded_at_epoch_millis)
             VALUES(?1,?2,?3,?4) ON CONFLICT(user_id,attachment_id) DO UPDATE SET
             deleted_revision_epoch_millis=excluded.deleted_revision_epoch_millis,recorded_at_epoch_millis=excluded.recorded_at_epoch_millis
             WHERE excluded.deleted_revision_epoch_millis>note_media_tombstones.deleted_revision_epoch_millis",
            params![user,id,revision,system_time_epoch_millis()])?;
        removed += transaction.execute(
            "DELETE FROM note_media WHERE user_id=?1 AND attachment_id=?2 AND updated_at_epoch_millis<=?3",
            params![user,id,revision])?;
    }
    if removed > 0 && current_schema_version(transaction)? >= 15 {
        transaction.execute(
            "UPDATE account_note_privacy SET cleanup_pending=1 WHERE user_id=?1",
            params![user],
        )?;
    }
    Ok(outcome)
}

#[derive(Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaPrivacyStatus {
    pub awaiting_reference_resolution: u64,
    pub ready_to_purge: u64,
    pub opaque_note_deletions: u64,
    pub retained_deleted_media: u64,
}

impl SqliteServerStore {
    /// Explicit logical-cleanup state. Physical VACUUM completion does not
    /// imply that references hidden by a legacy encrypted note were resolved.
    pub fn media_privacy_status(&self) -> StoreResult<MediaPrivacyStatus> {
        let mut database = self.open_connection(false)?;
        let connection = database.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let mut users =
            connection.prepare("SELECT user_id FROM account_note_privacy ORDER BY user_id")?;
        let mut result = MediaPrivacyStatus::default();
        result.retained_deleted_media = retained_deleted_count(&connection)?;
        for user in users.query_map([], |row| row.get::<_, String>(0))? {
            let user = user?;
            let policy = note_privacy::read_policy(&connection, &user)?;
            result.opaque_note_deletions += policy.opaque_media_deletions().len() as u64;
            if policy.media_candidates().is_empty() {
                continue;
            }
            let current = read_account_in_transaction(&connection, &user)?
                .ok_or_else(|| StoreError::Integrity("media cleanup account is missing".into()))?;
            let mut scope = references(&document(&current.app_data_json)?, &policy);
            let mut statement = connection.prepare("SELECT h.user_id,h.revision,h.created_at_epoch_millis,c.sha256,c.compression,c.uncompressed_size_bytes,c.compressed_size_bytes,c.content
                FROM account_snapshot_history h JOIN snapshot_contents c ON c.sha256=h.content_sha256
                WHERE h.user_id=?1 ORDER BY h.revision")?;
            for row in statement.query_map(params![user], snapshot_content_row)? {
                let history = decode_snapshot_history(row?)?;
                let found = references(&document(&history.app_data_json)?, &policy);
                scope.ids.extend(found.ids);
                scope.unknown |= found.unknown;
            }
            for (id, revision) in policy.media_candidates() {
                if scope.ids.contains(id) {
                    continue;
                }
                let newer: Option<i64> = connection.query_row("SELECT updated_at_epoch_millis FROM note_media WHERE user_id=?1 AND attachment_id=?2",
                    params![user, id], |row| row.get(0)).optional()?;
                if newer.is_some_and(|at| at > *revision) {
                    continue;
                }
                if scope.unknown {
                    result.awaiting_reference_resolution += 1;
                } else {
                    result.ready_to_purge += 1;
                }
            }
        }
        Ok(result)
    }
}

impl SqliteServerStore {
    /// Private storage boundary. The future HTTP adapter must supply a verified
    /// account identity; no unauthenticated endpoint exposes this operation.
    /// Revision and restore generation must still match inside the write lock.
    pub(crate) fn register_sealed_media_references(
        &self,
        user: &str,
        note_id: &str,
        expected_revision: i64,
        expected_generation: i64,
        declaration_json: &str,
    ) -> StoreResult<bool> {
        if declaration_json.len() > 1_500_000 {
            return Err(StoreError::Integrity(
                "sealed media declaration too large".into(),
            ));
        }
        let declaration = serde_json::from_str(declaration_json)?;
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(current) = read_account_in_transaction(&transaction, user)? else {
            return Ok(false);
        };
        if current.revision != expected_revision
            || restore_generation_in_transaction(&transaction, user)? != expected_generation
        {
            return Ok(false);
        }
        let value = document(&current.app_data_json)?;
        let Some(note) = value["notes"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|note| note["id"].as_str() == Some(note_id))
        else {
            return Ok(false);
        };
        let previous = note_privacy::read_policy(&transaction, user)?;
        let policy = previous
            .including_sealed_media(note, declaration)
            .map_err(|error| StoreError::Integrity(error.to_string()))?;
        let committed = note_privacy::record_reference_metadata(&transaction, user, policy)?;
        if committed != previous {
            privacy_journal::stage(&transaction, user, &committed)?;
        }
        privacy_journal::commit(transaction)?;
        Ok(true)
    }
}

/// Plaintext history has its own verified content snapshots and remains
/// restorable after ordinary live-content GC. Sealed history has no visible
/// attachment manifest, so an unresolved or positive sealed reference must
/// retain the live bytes until that reference is resolved.
pub(super) struct OrdinaryDeletionScope {
    ids: HashSet<String>,
    unknown: bool,
}
impl OrdinaryDeletionScope {
    pub(super) fn allow_delete(&self, id: &str) -> StoreResult<()> {
        if self.ids.contains(id) {
            return Err(StoreError::MediaReferenceConflict { opaque: false });
        }
        if self.unknown {
            return Err(StoreError::MediaReferenceConflict { opaque: true });
        }
        Ok(())
    }
}

pub(super) fn ordinary_deletion_scope(
    transaction: &Transaction<'_>,
    user: &str,
    verify_history_backups: bool,
) -> StoreResult<OrdinaryDeletionScope> {
    let Some(current) = read_account_in_transaction(transaction, user)? else {
        return Err(StoreError::NotFound("media account is missing".into()));
    };
    let policy = note_privacy::read_policy(transaction, user)?;
    let current_value = document(&current.app_data_json)?;
    let mut found = references(&current_value, &policy);
    let unsupported = |value: &serde_json::Value| {
        value.get("schemaVersion").is_some_and(|v| {
            v.as_i64()
                .is_none_or(|n| n < 0 || n > i64::from(APP_DATA_SCHEMA_VERSION))
        })
    };
    found.unknown |= unsupported(&current_value)
        || reference_format_unknown(&current.app_data_json, &current_value);
    // Unknown current ciphertext already protects every candidate. Avoid
    // repeatedly decompressing archives while waiting for its declaration.
    if !found.unknown {
        let mut last = -1;
        loop {
            let stored = transaction.query_row(
                "SELECT h.user_id,h.revision,h.created_at_epoch_millis,c.sha256,c.compression,c.uncompressed_size_bytes,c.compressed_size_bytes,c.content
                 FROM account_snapshot_history h JOIN snapshot_contents c ON c.sha256=h.content_sha256
                 WHERE h.user_id=?1 AND h.revision>?2 ORDER BY h.revision LIMIT 1",
                params![user,last],snapshot_content_row).optional()?;
            let Some(stored) = stored else {
                break;
            };
            let history = decode_snapshot_history(stored)?;
            verify_snapshot_history_entry(&history)?;
            last = history.revision;
            let value = document(&history.app_data_json)?;
            found.unknown |=
                unsupported(&value) || reference_format_unknown(&history.app_data_json, &value);
            let historical = snapshot_media::references(
                transaction,
                user,
                &history.app_data_json,
                Some(&policy),
            )?;
            found.unknown |= historical.unresolved;
            if found.unknown {
                break;
            }
            for id in &historical.ids {
                if found.ids.contains(id) {
                    continue;
                }
                // Sealed history can release its live fallback once the exact
                // declared hash has its own verified same-account snapshot.
                if (verify_history_backups || historical.sealed(id))
                    && !snapshot_media::verified_entry(
                        transaction,
                        user,
                        history.revision,
                        id,
                        &historical,
                    )?
                {
                    found.ids.insert(id.clone());
                }
            }
        }
    }
    Ok(OrdinaryDeletionScope {
        ids: found.ids,
        unknown: found.unknown,
    })
}

fn reference_format_unknown(raw: &str, value: &serde_json::Value) -> bool {
    // Legacy server fixtures may have only notes without an AppData envelope.
    // A declared shared schema must be understood before absence is authority.
    value.get("schemaVersion").is_some()
        && !matches!(
            crate::app_data::app_data_json_compatibility(raw, 0),
            crate::app_data::AppDataJsonCompatibility::CurrentKnown
                | crate::app_data::AppDataJsonCompatibility::LegacyMigratable
        )
}

pub(super) fn prune_retained_media(
    transaction: &Transaction<'_>,
    cutoff: i64,
) -> StoreResult<usize> {
    let mut removed = 0;
    let mut last_user = String::new();
    loop {
        let user: Option<String> = transaction
            .query_row(
                "SELECT m.user_id FROM note_media m JOIN note_media_tombstones t
             ON t.user_id=m.user_id AND t.attachment_id=m.attachment_id
             WHERE m.user_id>?1 AND t.deleted_revision_epoch_millis>=m.updated_at_epoch_millis
             AND t.recorded_at_epoch_millis<?2 ORDER BY m.user_id LIMIT 1",
                params![last_user, cutoff],
                |row| row.get(0),
            )
            .optional()?;
        let Some(user) = user else {
            break;
        };
        last_user = user.clone();
        let scope = ordinary_deletion_scope(transaction, &user, true)?;
        if scope.unknown {
            continue;
        }
        let mut last_id = String::new();
        loop {
            // Bound temporary memory even when a server has many retained blobs.
            let ids:Vec<String>=transaction.prepare(
                "SELECT m.attachment_id FROM note_media m JOIN note_media_tombstones t
                 ON t.user_id=m.user_id AND t.attachment_id=m.attachment_id
                 WHERE m.user_id=?1 AND m.attachment_id>?2 AND t.deleted_revision_epoch_millis>=m.updated_at_epoch_millis
                 AND t.recorded_at_epoch_millis<?3 ORDER BY m.attachment_id LIMIT 256")?
                .query_map(params![user,last_id,cutoff],|row|row.get(0))?
                .collect::<Result<_,_>>()?;
            if ids.is_empty() {
                break;
            }
            for id in ids {
                last_id = id.clone();
                if scope.allow_delete(&id).is_err() {
                    continue;
                }
                removed += transaction.execute(
                    "DELETE FROM note_media WHERE user_id=?1 AND attachment_id=?2",
                    params![user, id],
                )?;
            }
        }
    }
    Ok(removed)
}

fn retained_deleted_count(transaction: &Transaction<'_>) -> StoreResult<u64> {
    let mut count = 0;
    let mut last_user = String::new();
    loop {
        let user: Option<String> = transaction
            .query_row(
                "SELECT m.user_id FROM note_media m JOIN note_media_tombstones t
             ON t.user_id=m.user_id AND t.attachment_id=m.attachment_id
             WHERE m.user_id>?1 AND t.deleted_revision_epoch_millis>=m.updated_at_epoch_millis
             ORDER BY m.user_id LIMIT 1",
                params![last_user],
                |row| row.get(0),
            )
            .optional()?;
        let Some(user) = user else {
            break;
        };
        last_user = user.clone();
        let scope = ordinary_deletion_scope(transaction, &user, true)?;
        let mut statement = transaction.prepare(
            "SELECT m.attachment_id FROM note_media m JOIN note_media_tombstones t
             ON t.user_id=m.user_id AND t.attachment_id=m.attachment_id
             WHERE m.user_id=?1 AND t.deleted_revision_epoch_millis>=m.updated_at_epoch_millis",
        )?;
        for id in statement.query_map(params![user], |row| row.get::<_, String>(0))? {
            if scope.allow_delete(&id?).is_err() {
                count += 1;
            }
        }
    }
    Ok(count)
}
