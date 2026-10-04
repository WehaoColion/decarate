// v0.0.4 - Stream verified history into identities without a cumulative ciphertext byte limit.
// v0.0.3 - Stream verified recovery records into a bounded transient attachment view.
// v0.0.2 - Keep encryption's current-file audit separate from recovery retention.
// v0.0.1 - Retain local recovery attachments under one journal write reservation.
use super::*;
use crate::desktop_media_references::DesktopMediaReferenceScope;
use crate::desktop_private_media_index::DesktopPrivateMediaReferences;
use std::ops::Deref;

const MAX_REFERENCE_SNAPSHOT_BYTES: i64 = 32 * 1024 * 1024;
const MAX_REFERENCE_SYNC_BYTES: i64 = 1024 * 1024;

/// Keep this guard alive until media cleanup finishes. BEGIN IMMEDIATE prevents
/// a new journal reference from committing between the scan and file removal.
/// Dropping it rolls back the read-only transaction and releases the reservation.
#[must_use]
pub struct DesktopMediaRetentionGuard {
    connection: Connection,
    references: DesktopMediaReferenceScope,
    policy: DesktopPrivacyPolicy,
    evidence: DesktopStateJournalEvidence,
}

impl Deref for DesktopMediaRetentionGuard {
    type Target = DesktopMediaReferenceScope;
    fn deref(&self) -> &Self::Target {
        &self.references
    }
}

impl Drop for DesktopMediaRetentionGuard {
    fn drop(&mut self) {
        let _ = self.connection.execute_batch("ROLLBACK");
    }
}

impl DesktopMediaRetentionGuard {
    /// Encryption audits files against the active snapshot. Historical references
    /// retain bytes for recovery but must not excuse leftover plaintext files.
    pub fn current_snapshot_references(&self, raw: &str) -> DesktopMediaReferenceScope {
        DesktopMediaReferenceScope::from_snapshot(raw, &self.policy)
    }

    pub fn journal_evidence(&self) -> &DesktopStateJournalEvidence {
        &self.evidence
    }

    /// Apply the current privacy fences before retaining an old copy. Unknown
    /// or damaged recovery content cannot become proof that media is unused.
    pub fn include_recovery_snapshot(&mut self, raw: &str) {
        match self.policy.redact_json(raw) {
            Ok(redacted) => {
                self.references
                    .merge_references(DesktopMediaReferenceScope::from_snapshot(
                        redacted.as_deref().unwrap_or(raw),
                        &self.policy,
                    ))
            }
            Err(_) => self.references.retain_unknown(),
        }
    }
}

impl DesktopStateStore {
    pub fn include_verified_private_media_recovery_file(
        &self,
        owner: &str,
        path: &Path,
        raw: &str,
        references: &mut DesktopPrivateMediaReferences,
    ) -> DesktopStateStoreResult<bool> {
        validate_owner(owner)?;
        let mut connection = self.open_connection(false)?;
        verify_required_schema(&connection)?;
        let transaction = connection.transaction()?;
        let proven = mirror_provenance::verified_source(&transaction, owner, path, raw)?;
        let exact_journal = if proven {
            true
        } else {
            read_snapshot_by_raw_digest(&transaction, owner, &sha256_hex(raw.as_bytes()), 0)?
                .is_some_and(|snapshot| snapshot.app_data_json == raw)
        };
        if !exact_journal {
            references.mark_unverified_recovery_file(path);
            return Ok(false);
        }
        let policy = privacy::read_privacy_policy(&transaction, owner)?.0;
        let redacted = policy.redact_json(raw)?;
        if references
            .include_retained_snapshot(redacted.as_deref().unwrap_or(raw))
            .is_err()
        {
            references.mark_unverified_recovery_file(path);
            return Ok(false);
        }
        Ok(true)
    }

    /// Admit mirror bytes only when a verified row proves the exact owner and
    /// raw hash in the same read transaction as the current privacy fences.
    /// An unproven legacy mirror stays pending; its filename is not authority.
    pub fn include_verified_private_media_recovery_snapshot(
        &self,
        owner: &str,
        raw: &str,
        references: &mut DesktopPrivateMediaReferences,
    ) -> DesktopStateStoreResult<bool> {
        validate_owner(owner)?;
        if raw.len() as i64 > MAX_REFERENCE_SNAPSHOT_BYTES {
            references.mark_unverified_recovery_source();
            return Ok(false);
        }
        let mut connection = self.open_connection(false)?;
        verify_required_schema(&connection)?;
        let transaction = connection.transaction()?;
        if !owner_registry_initialized(&transaction, owner)? {
            references.mark_unverified_recovery_source();
            return Ok(false);
        }
        let hash = sha256_hex(raw.as_bytes());
        let verified = read_snapshot_by_raw_digest(&transaction, owner, &hash, 0)?
            .is_some_and(|snapshot| snapshot.app_data_json == raw);
        if !verified {
            references.mark_unverified_recovery_source();
            return Ok(false);
        }
        let policy = privacy::read_privacy_policy(&transaction, owner)?.0;
        let redacted = policy.redact_json(raw)?;
        match references.include_retained_snapshot(redacted.as_deref().unwrap_or(raw)) {
            Ok(()) => Ok(true),
            Err(_) => {
                references.mark_unverified_recovery_source();
                Ok(false)
            }
        }
    }

    /// Build the compact attachment index used by a workspace sync worker.
    /// This view is never published as AppData or saved as the current snapshot.
    pub fn private_media_references(
        &self,
        owner: &str,
        current_snapshot: &str,
    ) -> DesktopStateStoreResult<DesktopPrivateMediaReferences> {
        validate_owner(owner)?;
        let mut connection = self.open_connection(false)?;
        verify_required_schema(&connection)?;
        let transaction = connection.transaction()?;
        private_media_references(&transaction, owner, current_snapshot)
    }

    /// Opens an existing current-schema journal without collecting or repairing
    /// every account's history. The caller validates the returned journal
    /// identity against its independent workspace evidence before deleting files.
    pub fn lock_retained_media_references(
        database_path: &Path,
        owner: &str,
        current_snapshot: &str,
    ) -> DesktopStateStoreResult<DesktopMediaRetentionGuard> {
        validate_owner(owner)?;
        let store = Self {
            database_path: database_path.to_path_buf(),
        };
        let connection = store.open_connection(false)?;
        connection.execute_batch("BEGIN IMMEDIATE")?;
        verify_required_schema(&connection)?;
        let evidence = read_journal_evidence(&connection)?;
        let policy = privacy::read_privacy_policy(&connection, owner)?.0;
        let references = DesktopMediaReferenceScope::from_snapshot(current_snapshot, &policy);
        let mut guard = DesktopMediaRetentionGuard {
            connection,
            references,
            policy,
            evidence,
        };
        let has_snapshot: bool = guard.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM desktop_state_snapshots WHERE owner=?1)",
            params![owner],
            |row| row.get(0),
        )?;
        if owner_registry_initialized(&guard.connection, owner)? && !has_snapshot {
            guard.references.retain_unknown();
            return Ok(guard);
        }
        let quarantined: bool = guard.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM desktop_state_snapshot_quarantine WHERE owner=?1)",
            params![owner],
            |row| row.get(0),
        )?;
        if quarantined {
            guard.references.retain_unknown();
            return Ok(guard);
        }
        // Read one bounded snapshot at a time; never materialize the retained
        // history as a Vec of complete JSON documents.
        let mut previous = 0;
        loop {
            let next = guard
                .connection
                .query_row(
                    "SELECT id, length(CAST(app_data_json AS BLOB)), length(protected_sync_state)
                 FROM desktop_state_snapshots WHERE owner=?1 AND id>?2 ORDER BY id LIMIT 1",
                    params![owner, previous],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    },
                )
                .optional()?;
            let Some((id, json_bytes, sync_bytes)) = next else {
                break;
            };
            previous = id;
            if !(0..=MAX_REFERENCE_SNAPSHOT_BYTES).contains(&json_bytes)
                || !(0..=MAX_REFERENCE_SYNC_BYTES).contains(&sync_bytes)
            {
                guard.references.retain_unknown();
                break;
            }
            let snapshot = read_snapshot_by_id(&guard.connection, id)?
                .ok_or_else(|| integrity("media reference snapshot disappeared"))?;
            if verify_snapshot(&snapshot, Some(owner), 0).is_err() {
                guard.references.retain_unknown();
                break;
            }
            guard.include_recovery_snapshot(&snapshot.app_data_json);
            if !guard.references.is_complete() {
                break;
            }
        }
        Ok(guard)
    }
}

pub(super) fn private_media_references(
    connection: &Connection,
    owner: &str,
    current_snapshot: &str,
) -> DesktopStateStoreResult<DesktopPrivateMediaReferences> {
    validate_owner(owner)?;
    let (policy, _) = privacy::read_privacy_policy(connection, owner)?;
    if policy.redact_json(current_snapshot)?.is_some() {
        return Err(integrity(
            "current private reference state is behind its privacy fence",
        ));
    }
    let mut view =
        DesktopPrivateMediaReferences::from_snapshot(current_snapshot).map_err(integrity)?;
    let quarantined: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM desktop_state_snapshot_quarantine WHERE owner=?1)",
        params![owner],
        |row| row.get(0),
    )?;
    if quarantined {
        return Err(integrity(
            "private recovery references contain quarantined history",
        ));
    }
    let mut previous = 0;
    let mut found = false;
    loop {
        crate::runtime::cancellation::check_current()
            .map_err(|_| integrity("private recovery reference scan cancelled"))?;
        let next=connection.query_row(
            "SELECT id, length(CAST(app_data_json AS BLOB)), length(protected_sync_state) FROM desktop_state_snapshots WHERE owner=?1 AND id>?2 ORDER BY id LIMIT 1",
            params![owner,previous],|row| Ok((row.get::<_,i64>(0)?,row.get::<_,i64>(1)?,row.get::<_,i64>(2)?))).optional()?;
        let Some((id, json_bytes, sync_bytes)) = next else {
            break;
        };
        previous = id;
        found = true;
        if !(0..=MAX_REFERENCE_SNAPSHOT_BYTES).contains(&json_bytes)
            || !(0..=MAX_REFERENCE_SYNC_BYTES).contains(&sync_bytes)
        {
            return Err(integrity(
                "private recovery snapshot exceeds its byte limit",
            ));
        }
        let retained = read_snapshot_by_id(connection, id)?
            .ok_or_else(|| integrity("private recovery snapshot disappeared"))?;
        verify_snapshot(&retained, Some(owner), 0)?;
        let redacted = policy.redact_json(&retained.app_data_json)?;
        view.include_retained_snapshot(redacted.as_deref().unwrap_or(&retained.app_data_json))
            .map_err(integrity)?;
    }
    if !found && owner_registry_initialized(connection, owner)? {
        return Err(integrity(
            "private recovery history is missing for an initialized owner",
        ));
    }
    mirror_provenance::include_sources(connection, owner, &mut view)?;
    Ok(view)
}
