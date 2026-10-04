// v0.0.7 - Preserve physical cleanup state when committing reference-only metadata.
// v0.0.6 - Bind repeated historical privacy checks to compressed bytes and current policy.
// v0.0.5 - Reuse verified unchanged projections with a bounded fingerprint cache.
// v0.0.4 - Validate legacy note data independently of opaque historical timer fields.
// v0.0.2 - Reconcile independent privacy fences before recovery and backup.
// v0.0.1 - Enforce note privacy across server snapshots and sync receipts.
use super::*;
use crate::desktop_state_store::DesktopPrivacyPolicy;

fn failure(error: impl fmt::Display) -> StoreError {
    StoreError::Integrity(format!("server note privacy: {error}"))
}

pub(super) fn schema_sql() -> &'static str {
    "CREATE TABLE account_note_privacy (
        user_id TEXT PRIMARY KEY NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        policy_json TEXT NOT NULL CHECK(json_valid(policy_json)),
        binding_sha256 TEXT NOT NULL CHECK(length(binding_sha256) = 64),
        cleanup_pending INTEGER NOT NULL CHECK(cleanup_pending IN (0, 1))
    ) STRICT"
}

pub(super) fn binding(user_id: &str, policy_json: &str) -> String {
    sha256_hex(
        &serde_json::to_vec(&("server-note-privacy-v1", user_id, policy_json))
            .expect("serializable privacy binding"),
    )
}

pub(super) fn read_policy(
    connection: &Connection,
    user_id: &str,
) -> StoreResult<DesktopPrivacyPolicy> {
    let row = connection
        .query_row(
            "SELECT policy_json, binding_sha256 FROM account_note_privacy WHERE user_id=?1",
            params![user_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let Some((json, digest)) = row else {
        return Ok(DesktopPrivacyPolicy::default());
    };
    if digest != binding(user_id, &json) {
        return Err(failure("privacy binding is invalid"));
    }
    let policy: DesktopPrivacyPolicy = serde_json::from_str(&json)?;
    policy.validate_media_deletions().map_err(failure)?;
    Ok(policy)
}

// The server also retains empty initial states and pre-AppData legacy objects.
// Such documents have no note records to redact. AppData-shaped documents are
// parsed strictly by the shared policy; unknown versions are never normalized.
fn has_note_data(raw: &str) -> StoreResult<bool> {
    let Some(value) = validate_app_data_json_structure(raw)? else {
        return Ok(false);
    };
    Ok([
        "schemaVersion",
        "notes",
        "tombstones",
        "syncConflictHistory",
    ]
    .iter()
    .any(|key| value.get(*key).is_some()))
}

fn has_privacy_transition(raw: &str) -> StoreResult<bool> {
    let Some(value) = validate_app_data_json_structure(raw)? else {
        return Ok(false);
    };
    Ok(value["notes"].as_array().into_iter().flatten().any(|note| {
        note.get("encryption")
            .is_some_and(|envelope| !envelope.is_null())
    }) || value["tombstones"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|tombstone| tombstone["entityType"] == "note")
        || !crate::note_media_intent::infer_snapshot_detachments(&value)
            .map_err(failure)?
            .is_empty())
}

pub(super) fn project(policy: &DesktopPrivacyPolicy, raw: &str) -> StoreResult<Option<String>> {
    if policy.is_empty() {
        return Ok(None);
    }
    // Only cache a successful no-change result for this exact policy and body.
    // This contains no plaintext and does not replace ownership, signature or
    // database integrity checks performed by callers. Errors are never cached.
    static UNCHANGED: std::sync::OnceLock<std::sync::Mutex<std::collections::VecDeque<[u8; 32]>>> =
        std::sync::OnceLock::new();
    struct HashWriter(Sha256);
    impl io::Write for HashWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut policy_hash = HashWriter(Sha256::new());
    serde_json::to_writer(&mut policy_hash, policy)?;
    let mut digest = Sha256::new();
    digest.update(b"server-note-projection-unchanged-v1\0");
    digest.update(policy_hash.0.finalize());
    digest.update(Sha256::digest(raw.as_bytes()));
    let key: [u8; 32] = digest.finalize().into();
    let cache = UNCHANGED.get_or_init(Default::default);
    if cache.lock().is_ok_and(|cache| cache.contains(&key)) {
        return Ok(None);
    }
    let result = project_uncached(policy, raw)?;
    if result.is_none() {
        if let Ok(mut cache) = cache.lock() {
            if !cache.contains(&key) {
                if cache.len() >= 2048 {
                    cache.pop_front();
                }
                cache.push_back(key);
            }
        }
    }
    Ok(result)
}

fn project_uncached(policy: &DesktopPrivacyPolicy, raw: &str) -> StoreResult<Option<String>> {
    if !has_note_data(raw)? {
        return Ok(None);
    }
    match policy.redact_json(raw) {
        Ok(redacted) => Ok(redacted),
        Err(error) => {
            let mut original: serde_json::Value = serde_json::from_str(raw)?;
            let schema = original
                .get("schemaVersion")
                .map(|value| value.as_i64())
                .unwrap_or(Some(0));
            let Some(schema) =
                schema.filter(|version| (0..i64::from(APP_DATA_SCHEMA_VERSION)).contains(version))
            else {
                return Err(failure(error));
            };
            // Consumed server archives can contain old, invalid timer shapes.
            // They are never imported by this operation. Validate all note
            // fields strictly, redact those fields, and retain the other values.
            let mut projection: serde_json::Value =
                serde_json::from_str(&crate::app_data::default_app_data_json(0))?;
            projection["schemaVersion"] = serde_json::json!(schema);
            for key in ["notes", "noteFolders", "tombstones", "syncConflictHistory"] {
                if let Some(value) = original.get(key) {
                    projection[key] = value.clone();
                }
            }
            let Some(redacted) = policy
                .redact_json(&serde_json::to_string(&projection)?)
                .map_err(failure)?
            else {
                return Ok(None);
            };
            let redacted: serde_json::Value = serde_json::from_str(&redacted)?;
            for key in ["notes", "tombstones", "syncConflictHistory"] {
                let new_deletion_evidence = key == "tombstones"
                    && redacted[key]
                        .as_array()
                        .is_some_and(|items| !items.is_empty());
                if original.get(key).is_some() || new_deletion_evidence {
                    original[key] = redacted[key].clone();
                }
            }
            Ok(Some(serde_json::to_string(&original)?))
        }
    }
}

pub(super) fn redact_response(
    policy: &DesktopPrivacyPolicy,
    raw: &str,
) -> StoreResult<Option<String>> {
    if policy.is_empty() {
        return Ok(None);
    }
    let mut response: serde_json::Value = serde_json::from_str(raw)?;
    let Some(data) = response.get("appDataJson").filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let data = data
        .as_str()
        .ok_or_else(|| failure("sync response has an unknown data envelope"))?;
    let Some(redacted) = project_current(policy, data)? else {
        return Ok(None);
    };
    response["appDataJson"] = serde_json::Value::String(redacted);
    Ok(Some(serde_json::to_string(&response)?))
}

/// Detachment floors protect a current/recovered head; archival content keeps
/// its independent restoration references until an explicit privacy deletion.
pub(super) fn project_current(
    policy: &DesktopPrivacyPolicy,
    raw: &str,
) -> StoreResult<Option<String>> {
    let redacted = project(policy, raw)?;
    let base = redacted.as_deref().unwrap_or(raw);
    if !has_note_data(base)? || policy.note_attachment_detachments().is_empty() {
        return Ok(redacted);
    }
    let projected = policy.project_current_json(base).map_err(failure)?;
    Ok(projected.or(redacted))
}

/// Called within the account mutation transaction, after the new current row
/// and its recovery copy exist. Any failure rolls back the whole mutation.
pub(super) fn enforce(
    transaction: &Transaction<'_>,
    user_id: &str,
) -> StoreResult<DesktopPrivacyPolicy> {
    enforce_with_media_intents(
        transaction,
        user_id,
        &Default::default(),
        Default::default(),
    )
}

pub(super) fn enforce_with_media_intents(
    transaction: &Transaction<'_>,
    user_id: &str,
    detachments: &crate::note_media_intent::NoteAttachmentDetachments,
    candidates: media_privacy::Candidates,
) -> StoreResult<DesktopPrivacyPolicy> {
    let current = read_account_in_transaction(transaction, user_id)?
        .ok_or_else(|| failure("account disappeared during privacy update"))?;
    let previous = read_policy(transaction, user_id)?;
    let policy = if has_privacy_transition(&current.app_data_json)? {
        previous
            .including_snapshot(&current.app_data_json)
            .map_err(failure)?
    } else {
        previous.clone()
    }
    .including_detachments(detachments)
    .map_err(failure)?;
    if project_current(&policy, &current.app_data_json)?.is_some() {
        return Err(failure(
            "incoming account would restore content behind a note privacy barrier",
        ));
    }
    let policy = if policy == previous && candidates.is_empty() {
        if policy.is_empty() {
            return Ok(policy);
        }
        let upgraded = complete_media_policy(transaction, user_id, policy, Default::default())?;
        if upgraded == previous {
            return Ok(upgraded);
        }
        upgraded
    } else {
        policy
    };
    let policy = write_policy(transaction, user_id, &policy, candidates)?;
    privacy_journal::stage(transaction, user_id, &policy)?;
    Ok(policy)
}

fn complete_media_policy(
    transaction: &Transaction<'_>,
    user: &str,
    policy: DesktopPrivacyPolicy,
    candidates: media_privacy::Candidates,
) -> StoreResult<DesktopPrivacyPolicy> {
    let result = media_privacy::purge(transaction, user, candidates, &policy)?;
    policy
        .including_media_work(&result.candidates, &result.opaque_notes)
        .map_err(failure)?
        .including_media_deletions(&result.fences)
        .map_err(failure)
}

fn write_policy(
    transaction: &Transaction<'_>,
    user_id: &str,
    policy: &DesktopPrivacyPolicy,
    mut candidates: media_privacy::Candidates,
) -> StoreResult<DesktopPrivacyPolicy> {
    redact_history(transaction, user_id, policy, &mut candidates)?;
    #[cfg(test)]
    private_media_exchange::profile_step("overlay_redact_history");
    let policy = complete_media_policy(transaction, user_id, policy.clone(), candidates)?;
    #[cfg(test)]
    private_media_exchange::profile_step("overlay_resolve_media");
    let mut last_rowid = 0;
    loop {
        let row = transaction.query_row(
            "SELECT rowid, response_json FROM request_dedup WHERE user_id=?1 AND rowid>?2 ORDER BY rowid LIMIT 1",
            params![user_id, last_rowid], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        ).optional()?;
        let Some((rowid, raw)) = row else {
            break;
        };
        last_rowid = rowid;
        if let Some(redacted) = redact_response(&policy, &raw)? {
            // Keep the fingerprint and receipt identity spent. Deleting a
            // receipt would allow an old request to be applied a second time.
            transaction.execute(
                "UPDATE request_dedup SET response_json=?1 WHERE rowid=?2",
                params![redacted, rowid],
            )?;
        }
    }
    #[cfg(test)]
    private_media_exchange::profile_step("overlay_redact_receipts");
    let json = serde_json::to_string(&policy)?;
    transaction.execute(
        "INSERT INTO account_note_privacy(user_id, policy_json, binding_sha256, cleanup_pending)
         VALUES(?1, ?2, ?3, 1) ON CONFLICT(user_id) DO UPDATE SET
         policy_json=excluded.policy_json, binding_sha256=excluded.binding_sha256, cleanup_pending=1",
        params![user_id, json, binding(user_id, &json)],
    )?;
    verify_snapshot_content_index(transaction)?;
    Ok(policy)
}

/// Caller has authenticated the request and matched its ciphertexts while
/// holding this account transaction. Companion metadata does not change note
/// redaction fences, so old notes and request receipts need no reprojection.
pub(super) fn record_reference_metadata(
    transaction: &Transaction<'_>,
    user_id: &str,
    policy: DesktopPrivacyPolicy,
) -> StoreResult<DesktopPrivacyPolicy> {
    let previous = read_policy(transaction, user_id)?;
    if !policy.has_same_non_reference_state(&previous) {
        return Err(failure("reference metadata cannot change privacy fences"));
    }
    policy.validate_media_deletions().map_err(failure)?;
    snapshot_media::reconcile_user(
        transaction,
        user_id,
        Some(&policy),
        system_time_epoch_millis(),
        false,
        false,
    )?;
    // A new declaration can resolve an earlier deletion candidate. Keep the
    // normal reference, quota, tombstone and physical-removal checks for it.
    let policy = complete_media_policy(transaction, user_id, policy, Default::default())?;
    #[cfg(test)]
    private_media_exchange::profile_step("metadata_resolve_media");
    if policy == previous {
        verify_snapshot_content_index(transaction)?;
        return Ok(policy);
    }
    // purge marks real removed bytes pending. Read this state afterward, so
    // metadata writes preserve both existing and newly discovered cleanup.
    let pending: i64 = transaction
        .query_row(
            "SELECT cleanup_pending FROM account_note_privacy WHERE user_id=?1",
            params![user_id],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(0);
    let json = serde_json::to_string(&policy)?;
    transaction.execute(
        "INSERT INTO account_note_privacy(user_id,policy_json,binding_sha256,cleanup_pending)
         VALUES(?1,?2,?3,?4) ON CONFLICT(user_id) DO UPDATE SET
         policy_json=excluded.policy_json,binding_sha256=excluded.binding_sha256,
         cleanup_pending=excluded.cleanup_pending",
        params![user_id, json, binding(user_id, &json), pending],
    )?;
    verify_snapshot_content_index(transaction)?;
    Ok(policy)
}

/// Apply an authoritative external fence to a verified recovery copy. Account
/// revisions and unrelated data remain intact; privacy is not an account edit.
pub(super) fn overlay_policy(
    transaction: &Transaction<'_>,
    user_id: &str,
    external: &DesktopPrivacyPolicy,
) -> StoreResult<()> {
    let Some(current) = read_account_in_transaction(transaction, user_id)? else {
        return Ok(());
    };
    let previous = read_policy(transaction, user_id)?;
    let mut policy = previous.merged_policy(external).map_err(failure)?;
    #[cfg(test)]
    private_media_exchange::profile_step("overlay_merge");
    let redacted = project_current(&policy, &current.app_data_json)?;
    if policy == previous && redacted.is_none() {
        if policy.is_empty() {
            return Ok(());
        }
        policy = complete_media_policy(transaction, user_id, policy, Default::default())?;
        if policy == previous {
            return Ok(());
        }
    }
    #[cfg(test)]
    private_media_exchange::profile_step("overlay_project");
    let mut candidates = media_privacy::Candidates::new();
    if let Some(redacted) = redacted {
        media_privacy::observe_projection(
            &mut candidates,
            &current.app_data_json,
            &redacted,
            &policy,
        )?;
        let generation = restore_generation_in_transaction(transaction, user_id)?;
        transaction.execute("UPDATE account_snapshots SET app_data_json=?1, content_sha256=?2, envelope_sha256=?3 WHERE user_id=?4",
            params![redacted, sha256_hex(redacted.as_bytes()), account_snapshot_envelope_sha256(user_id, &redacted, current.revision, current.updated_at_epoch_millis, generation), user_id])?;
        replace_current_snapshot_media_identities(transaction, user_id, &redacted)?;
    }
    write_policy(transaction, user_id, &policy, candidates).map(|_| ())
}

pub(super) fn redact_history(
    transaction: &Transaction<'_>,
    user_id: &str,
    policy: &DesktopPrivacyPolicy,
    candidates: &mut media_privacy::Candidates,
) -> StoreResult<()> {
    // Archive manifests keep their source format's interpretation. Safety
    // checks in media_privacy still inspect typed and sealed references.
    let include_typed_conflicts = current_schema_version(transaction)? >= 16;
    let stored_reference_ids = |raw: &str| -> StoreResult<HashSet<String>> {
        let mut ids = collected_referenced_attachment_ids_at_format(raw, include_typed_conflicts)?;
        ids.retain(|id| valid_media_attachment_id(id));
        Ok(ids)
    };
    let mut last_revision = -1;
    loop {
        let stored = transaction.query_row(
            "SELECT h.user_id, h.revision, h.created_at_epoch_millis,
                    c.sha256, c.compression, c.uncompressed_size_bytes, c.compressed_size_bytes, c.content
             FROM account_snapshot_history h JOIN snapshot_contents c ON c.sha256=h.content_sha256
             WHERE h.user_id=?1 AND h.revision>?2 ORDER BY h.revision LIMIT 1",
            params![user_id, last_revision], snapshot_content_row,
        ).optional()?;
        let Some(stored) = stored else {
            break;
        };
        let history = decode_snapshot_history(stored)?;
        verify_snapshot_history_entry(&history)?;
        last_revision = history.revision;
        let Some(redacted) = project(policy, &history.app_data_json)? else {
            if include_typed_conflicts {
                snapshot_media::reconcile_one(
                    transaction,
                    user_id,
                    history.revision,
                    &history.app_data_json,
                    history.created_at_epoch_millis,
                    Some(policy),
                    false,
                )?;
            }
            continue;
        };
        media_privacy::observe_projection(candidates, &history.app_data_json, &redacted, policy)?;
        let digest = sha256_hex(redacted.as_bytes());
        ensure_snapshot_content(
            transaction,
            &digest,
            &redacted,
            history.created_at_epoch_millis,
        )?;
        transaction.execute(
            "UPDATE account_snapshot_history SET content_sha256=?1 WHERE user_id=?2 AND revision=?3",
            params![digest, user_id, history.revision],
        )?;
        // Existing ref-count triggers release only the changed row's old
        // content. Shared content belonging to other accounts stays intact.
        if include_typed_conflicts {
            snapshot_media::reconcile_one(
                transaction,
                user_id,
                history.revision,
                &redacted,
                history.created_at_epoch_millis,
                Some(policy),
                false,
            )?;
            continue;
        }
        let retained_ids = stored_reference_ids(&redacted)?;
        let previous_ids = stored_reference_ids(&history.app_data_json)?;
        for attachment_id in previous_ids.difference(&retained_ids) {
            transaction.execute(
                "DELETE FROM account_snapshot_media_history WHERE user_id=?1 AND account_revision=?2 AND attachment_id=?3",
                params![user_id, history.revision, attachment_id],
            )?;
        }
        // Sealed envelopes can introduce references to new encrypted media.
        // Never label these as recoverable from an older attachment snapshot.
        for attachment_id in retained_ids.difference(&previous_ids) {
            transaction.execute(
                "INSERT INTO account_snapshot_media_history(user_id, account_revision, attachment_id,
                    content_sha256, declared_sha256, mime_type, size_bytes, updated_at_epoch_millis, missing_reason)
                 VALUES (?1, ?2, ?3, NULL, '', '', 0, 0, 'privacy_generation_media_unavailable')",
                params![user_id, history.revision, attachment_id],
            )?;
        }
        if retained_ids.is_empty() {
            transaction.execute("UPDATE account_snapshot_history SET media_snapshot_complete=1 WHERE user_id=?1 AND revision=?2",
                params![user_id, history.revision])?;
        } else if !retained_ids.is_subset(&previous_ids) {
            transaction.execute("UPDATE account_snapshot_history SET media_snapshot_complete=0 WHERE user_id=?1 AND revision=?2",
                params![user_id, history.revision])?;
        }
    }
    if !include_typed_conflicts {
        verify_snapshot_content_index(transaction)?;
    }
    Ok(())
}

pub(super) fn repair_all(connection: &mut Connection) -> StoreResult<()> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut last_id: Option<String> = None;
    loop {
        let user_id = transaction.query_row(
            "SELECT user_id FROM account_snapshots WHERE ?1 IS NULL OR user_id>?1 ORDER BY user_id LIMIT 1",
            params![last_id], |row| row.get::<_, String>(0),
        ).optional()?;
        let Some(user_id) = user_id else {
            break;
        };
        enforce(&transaction, &user_id)?;
        last_id = Some(user_id);
    }
    privacy_journal::commit(transaction)?;
    Ok(())
}

pub(super) fn verify(connection: &Connection) -> StoreResult<()> {
    let mut statement =
        connection.prepare("SELECT user_id FROM account_note_privacy ORDER BY user_id")?;
    for user_id in statement.query_map([], |row| row.get::<_, String>(0))? {
        let user_id = user_id?;
        let policy = read_policy(connection, &user_id)?;
        let raw: String = connection.query_row(
            "SELECT app_data_json FROM account_snapshots WHERE user_id=?1",
            params![user_id],
            |row| row.get(0),
        )?;
        if project(&policy, &raw)?.is_some() {
            return Err(failure("current snapshot violates privacy barrier"));
        }
        let mut history = connection.prepare(
            "SELECT h.user_id, h.revision, h.created_at_epoch_millis,
                    c.sha256, c.compression, c.uncompressed_size_bytes, c.compressed_size_bytes, c.content
             FROM account_snapshot_history h JOIN snapshot_contents c ON c.sha256=h.content_sha256
             WHERE h.user_id=?1 ORDER BY h.revision")?;
        for row in history.query_map(params![user_id], snapshot_content_row)? {
            content_verification::verify_private_history(&policy, row?)?;
        }
        let mut receipts =
            connection.prepare("SELECT response_json FROM request_dedup WHERE user_id=?1")?;
        for raw in receipts.query_map(params![user_id], |row| row.get::<_, String>(0))? {
            if redact_response(&policy, &raw?)?.is_some() {
                return Err(failure("sync receipt violates privacy barrier"));
            }
        }
    }
    Ok(())
}

impl SqliteServerStore {
    /// Completes live SQLite file cleanup. Existing external recovery backups
    /// require their own identity-verified rewrite and are not covered here.
    pub fn finish_note_privacy_cleanup(&self) -> StoreResult<()> {
        let mut connection = self.open_connection(false)?;
        privacy_journal::reconcile(&mut connection)?;
        let pending = {
            let mut statement = connection.prepare(
                "SELECT user_id, binding_sha256 FROM account_note_privacy WHERE cleanup_pending=1",
            )?;
            let records = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            records
        };
        if pending.is_empty() {
            return Ok(());
        }
        connection.execute_batch("VACUUM;")?;
        checked_checkpoint(&connection)?;
        for (user_id, digest) in &pending {
            connection.execute("UPDATE account_note_privacy SET cleanup_pending=0 WHERE user_id=?1 AND binding_sha256=?2",
                params![user_id, digest])?;
        }
        if let Err(error) = checked_checkpoint(&connection) {
            for (user_id, digest) in pending {
                connection.execute("UPDATE account_note_privacy SET cleanup_pending=1 WHERE user_id=?1 AND binding_sha256=?2",
                    params![user_id, digest])?;
            }
            return Err(error);
        }
        Ok(())
    }
}

fn checked_checkpoint(connection: &Connection) -> StoreResult<()> {
    let (busy, log, done) = connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    if busy != 0 || log != done {
        return Err(failure("SQLite privacy checkpoint is busy"));
    }
    Ok(())
}
