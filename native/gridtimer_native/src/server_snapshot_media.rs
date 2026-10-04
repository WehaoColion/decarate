// Account-bound historical media resolution; ciphertext alone never proves an empty snapshot.
use super::*;
use crate::desktop_state_store::DesktopPrivacyPolicy;

pub(super) struct ReferenceView {
    pub(super) ids: HashSet<String>,
    pub(super) unresolved: bool,
    plain: HashMap<String, ReferencedMediaExpectation>,
    sealed_hashes: HashMap<String, String>,
}

impl ReferenceView {
    pub(super) fn sealed(&self, id: &str) -> bool {
        self.sealed_hashes.contains_key(id)
    }
    fn hash(&self, id: &str) -> Option<&str> {
        self.plain
            .get(id)
            .map(|value| value.sha256.as_str())
            .filter(|hash| valid_sha256_hex(hash))
            .or_else(|| self.sealed_hashes.get(id).map(String::as_str))
    }

    fn matches(&self, entry: &LoadedMediaSnapshotEntry) -> bool {
        if !self.ids.contains(&entry.attachment_id)
            || self
                .sealed_hashes
                .get(&entry.attachment_id)
                .is_some_and(|hash| !hash.eq_ignore_ascii_case(&entry.sha256))
        {
            return false;
        }
        if let Some(expected) = self.plain.get(&entry.attachment_id) {
            return expected.sha256.eq_ignore_ascii_case(&entry.sha256)
                && expected.size_bytes_declared
                && expected.size_bytes == entry.size_bytes
                && expected
                    .mime_type
                    .eq_ignore_ascii_case(entry.mime_type.trim())
                && expected.updated_at_epoch_millis == entry.updated_at_epoch_millis;
        }
        self.sealed_hashes
            .get(&entry.attachment_id)
            .is_some_and(|hash| hash.eq_ignore_ascii_case(&entry.sha256))
    }
}

pub(super) fn references(
    connection: &Connection,
    user: &str,
    raw: &str,
    policy: Option<&DesktopPrivacyPolicy>,
) -> StoreResult<ReferenceView> {
    let owned_policy;
    let policy = if let Some(policy) = policy {
        policy
    } else {
        owned_policy = note_privacy::read_policy(connection, user)?;
        &owned_policy
    };
    let scope = media_privacy::historical_reference_scope(raw, policy)?;
    let mut unresolved = scope.unresolved;
    let plain = match referenced_media_expectations(raw) {
        Ok(plain) => plain,
        Err(StoreError::Integrity(_)) => {
            unresolved = true;
            HashMap::new()
        }
        Err(error) => return Err(error),
    };
    for (id, hash) in &scope.sealed_hashes {
        if plain
            .get(id)
            .is_some_and(|expected| !expected.sha256.eq_ignore_ascii_case(hash))
        {
            unresolved = true;
        }
    }
    unresolved |= scope.ids.iter().any(|id| !valid_media_attachment_id(id));
    Ok(ReferenceView {
        ids: scope.ids,
        unresolved,
        plain,
        sealed_hashes: scope.sealed_hashes,
    })
}

fn valid_entry(entry: &LoadedMediaSnapshotEntry) -> bool {
    valid_media_attachment_id(&entry.attachment_id)
        && valid_sha256_hex(&entry.sha256)
        && valid_media_mime_type(entry.mime_type.trim())
        && (0..=MAX_MEDIA_BYTES as i64).contains(&entry.size_bytes)
        && entry.updated_at_epoch_millis >= 0
        && entry.content.len() as i64 == entry.size_bytes
        && entry
            .sha256
            .eq_ignore_ascii_case(&sha256_hex(&entry.content))
}

pub(super) fn historical_entry(
    connection: &Connection,
    user: &str,
    revision: i64,
    id: &str,
) -> StoreResult<Option<LoadedMediaSnapshotEntry>> {
    let row = connection.query_row(
        "SELECT h.content_sha256,h.declared_sha256,h.mime_type,h.size_bytes,h.updated_at_epoch_millis,h.missing_reason,c.size_bytes,
          CASE WHEN c.size_bytes BETWEEN 0 AND ?4 AND length(c.content)<=?4 THEN c.content ELSE NULL END
         FROM account_snapshot_media_history h LEFT JOIN media_snapshot_contents c ON c.sha256=h.content_sha256
         WHERE h.user_id=?1 AND h.account_revision=?2 AND h.attachment_id=?3",
        params![user,revision,id,MAX_MEDIA_BYTES as i64], |row| Ok((
            row.get::<_,Option<String>>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,
            row.get::<_,i64>(3)?,row.get::<_,i64>(4)?,row.get::<_,String>(5)?,
            row.get::<_,Option<i64>>(6)?,row.get::<_,Option<Vec<u8>>>(7)?,
        )),
    ).optional()?;
    let Some((
        Some(sha256),
        declared,
        mime_type,
        size_bytes,
        updated_at_epoch_millis,
        missing,
        stored_size,
        Some(content),
    )) = row
    else {
        return Ok(None);
    };
    let entry = LoadedMediaSnapshotEntry {
        attachment_id: id.to_owned(),
        sha256,
        mime_type,
        size_bytes,
        updated_at_epoch_millis,
        content,
    };
    Ok((missing.is_empty()
        && stored_size == Some(size_bytes)
        && declared.eq_ignore_ascii_case(&entry.sha256)
        && valid_entry(&entry))
    .then_some(entry))
}

pub(super) fn verified_entry(
    connection: &Connection,
    user: &str,
    revision: i64,
    id: &str,
    references: &ReferenceView,
) -> StoreResult<bool> {
    Ok(historical_entry(connection, user, revision, id)?
        .is_some_and(|entry| references.matches(&entry)))
}

fn source_entry(
    transaction: &Transaction<'_>,
    user: &str,
    revision: i64,
    id: &str,
    references: &ReferenceView,
) -> StoreResult<Option<LoadedMediaSnapshotEntry>> {
    if let Some(entry) = historical_entry(transaction, user, revision, id)? {
        if references.matches(&entry) {
            return Ok(Some(entry));
        }
    }
    let Some(hash) = references.hash(id) else {
        return Ok(None);
    };
    // A retained deleted row is still byte evidence for this account. It does
    // not authorize a client upload or remove any deletion/restore barrier.
    let live = transaction
        .query_row(
            "SELECT sha256,mime_type,size_bytes,updated_at_epoch_millis,
          CASE WHEN size_bytes BETWEEN 0 AND ?3 AND length(content)<=?3 THEN content ELSE NULL END
         FROM note_media WHERE user_id=?1 AND attachment_id=?2",
            params![user, id, MAX_MEDIA_BYTES as i64],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<Vec<u8>>>(4)?,
                ))
            },
        )
        .optional()?;
    if let Some((sha256, mime_type, size_bytes, updated, Some(content))) = live {
        let entry = LoadedMediaSnapshotEntry {
            attachment_id: id.to_owned(),
            sha256,
            mime_type,
            size_bytes,
            updated_at_epoch_millis: references
                .plain
                .get(id)
                .map(|value| value.updated_at_epoch_millis)
                .unwrap_or(updated),
            content,
        };
        if valid_entry(&entry) && references.matches(&entry) {
            return Ok(Some(entry));
        }
    }
    // The account+attachment association is mandatory. A global deduplicated
    // hash, by itself, cannot supply evidence belonging to another account.
    let revisions = {
        let mut statement = transaction.prepare(
            "SELECT account_revision FROM account_snapshot_media_history WHERE user_id=?1 AND attachment_id=?2
             AND lower(content_sha256)=lower(?3) AND account_revision<>?4 ORDER BY account_revision DESC")?;
        let rows = statement.query_map(params![user, id, hash, revision], |row| {
            row.get::<_, i64>(0)
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for source_revision in revisions {
        if let Some(mut entry) = historical_entry(transaction, user, source_revision, id)? {
            if let Some(expected) = references.plain.get(id) {
                entry.updated_at_epoch_millis = expected.updated_at_epoch_millis;
            }
            if references.matches(&entry) {
                return Ok(Some(entry));
            }
        }
    }
    Ok(None)
}

pub(super) fn reconcile_one(
    transaction: &Transaction<'_>,
    user: &str,
    revision: i64,
    raw: &str,
    now: i64,
    policy: Option<&DesktopPrivacyPolicy>,
    migration: bool,
) -> StoreResult<()> {
    let references = references(transaction, user, raw, policy)?;
    let mut complete = !references.unresolved;
    let mut ids = references
        .ids
        .iter()
        .filter(|id| valid_media_attachment_id(id))
        .cloned()
        .collect::<Vec<_>>();
    ids.sort();
    let existing_ids = {
        let mut statement = transaction.prepare("SELECT attachment_id FROM account_snapshot_media_history WHERE user_id=?1 AND account_revision=?2")?;
        let rows = statement.query_map(params![user, revision], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<HashSet<_>, _>>()?
    };
    if !migration {
        validate_media_identity_batch_quota(transaction, user, &ids.iter().cloned().collect())?;
        let added = ids.iter().filter(|id| !existing_ids.contains(*id)).count() as i64;
        let removed = if references.unresolved {
            0
        } else {
            existing_ids
                .iter()
                .filter(|id| !references.ids.contains(*id))
                .count() as i64
        };
        let metadata_growth = added
            .saturating_sub(removed)
            .max(0)
            .saturating_mul(SCHEMA_MIGRATION_INDEX_ROW_ESTIMATE_BYTES as i64);
        let usage_bytes = snapshot_history_charged_bytes(transaction, user)?;
        let projected_bytes = usage_bytes.saturating_add(metadata_growth);
        if projected_bytes > SNAPSHOT_HISTORY_HARD_LIMIT_BYTES_PER_USER
            && projected_bytes > usage_bytes
        {
            return Err(StoreError::SnapshotHistoryQuotaExceeded {
                usage_bytes,
                projected_bytes,
                limit_bytes: SNAPSHOT_HISTORY_HARD_LIMIT_BYTES_PER_USER,
            });
        }
        ensure_snapshot_write_capacity(transaction, metadata_growth as u64)?;
    }
    transaction.execute("UPDATE account_snapshot_history SET media_snapshot_complete=0 WHERE user_id=?1 AND revision=?2",params![user,revision])?;
    for id in &ids {
        let mut captured = false;
        if let Some(entry) = source_entry(transaction, user, revision, id, &references)? {
            let capture = validate_media_history_repair_quota(
                transaction,
                user,
                &entry.sha256,
                entry.size_bytes,
            )
            .and_then(|()| {
                ensure_media_snapshot_content(transaction, &entry.sha256, &entry.content, now)
            });
            let available = match capture {
                Ok(()) => true,
                Err(StoreError::SnapshotHistoryQuotaExceeded { .. }) if migration => false,
                Err(error) if migration => migration_media_snapshot_content_available(Err(error))?,
                Err(error) => return Err(error),
            };
            if available {
                transaction.execute(
                    "INSERT INTO account_snapshot_media_history(user_id,account_revision,attachment_id,content_sha256,declared_sha256,mime_type,size_bytes,updated_at_epoch_millis,missing_reason)
                     VALUES(?1,?2,?3,?4,?4,?5,?6,?7,'') ON CONFLICT(user_id,account_revision,attachment_id) DO UPDATE SET
                     content_sha256=excluded.content_sha256,declared_sha256=excluded.declared_sha256,mime_type=excluded.mime_type,
                     size_bytes=excluded.size_bytes,updated_at_epoch_millis=excluded.updated_at_epoch_millis,missing_reason=''
                     WHERE content_sha256 IS NOT excluded.content_sha256 OR declared_sha256<>excluded.declared_sha256 OR mime_type<>excluded.mime_type
                     OR size_bytes<>excluded.size_bytes OR updated_at_epoch_millis<>excluded.updated_at_epoch_millis OR missing_reason<>''",
                    params![user,revision,id,entry.sha256,entry.mime_type,entry.size_bytes,entry.updated_at_epoch_millis])?;
                captured = true;
            }
        }
        if !captured {
            complete = false;
            let expected = references.plain.get(id);
            let missing_reason = if expected.is_some_and(|value| {
                valid_sha256_hex(&value.sha256)
                    && valid_media_mime_type(&value.mime_type)
                    && value.size_bytes_declared
                    && (0..=MAX_MEDIA_BYTES as i64).contains(&value.size_bytes)
            }) {
                "content_unavailable_at_snapshot"
            } else {
                "reference_metadata_unavailable_at_snapshot"
            };
            transaction.execute(
                "INSERT INTO account_snapshot_media_history(user_id,account_revision,attachment_id,content_sha256,declared_sha256,mime_type,size_bytes,updated_at_epoch_millis,missing_reason)
                 VALUES(?1,?2,?3,NULL,?4,?5,?6,?7,?8) ON CONFLICT(user_id,account_revision,attachment_id) DO NOTHING",
                params![user,revision,id,references.hash(id).unwrap_or_default(),
                    expected.map(|value|value.mime_type.as_str()).unwrap_or_default(),
                    expected.filter(|value|value.size_bytes_declared && (0..=MAX_MEDIA_BYTES as i64).contains(&value.size_bytes)).map(|value|value.size_bytes).unwrap_or(0),
                    expected.map(|value|value.updated_at_epoch_millis.max(0)).unwrap_or(0),missing_reason])?;
        }
    }
    if !references.unresolved {
        for id in existing_ids {
            if !references.ids.contains(&id) {
                transaction.execute("DELETE FROM account_snapshot_media_history WHERE user_id=?1 AND account_revision=?2 AND attachment_id=?3",params![user,revision,id])?;
            }
        }
    }
    transaction.execute("UPDATE account_snapshot_history SET media_snapshot_complete=?1 WHERE user_id=?2 AND revision=?3",params![i64::from(complete),user,revision])?;
    Ok(())
}

pub(super) fn reconcile_user(
    transaction: &Transaction<'_>,
    user: &str,
    policy: Option<&DesktopPrivacyPolicy>,
    now: i64,
    only_incomplete: bool,
    migration: bool,
) -> StoreResult<()> {
    let mut last = -1;
    loop {
        let stored = transaction.query_row(
            "SELECT h.user_id,h.revision,h.created_at_epoch_millis,c.sha256,c.compression,c.uncompressed_size_bytes,c.compressed_size_bytes,c.content
             FROM account_snapshot_history h JOIN snapshot_contents c ON c.sha256=h.content_sha256
             WHERE h.user_id=?1 AND h.revision>?2 AND (?3=0 OR h.media_snapshot_complete=0) ORDER BY h.revision LIMIT 1",
            params![user,last,i64::from(only_incomplete)],snapshot_content_row).optional()?;
        let Some(stored) = stored else {
            break;
        };
        let history = decode_snapshot_history(stored)?;
        last = history.revision;
        reconcile_one(
            transaction,
            user,
            history.revision,
            &history.app_data_json,
            now,
            policy,
            migration,
        )?;
    }
    Ok(())
}

pub(super) fn reconcile_all(transaction: &Transaction<'_>, now: i64) -> StoreResult<()> {
    let users = {
        let mut statement =
            transaction.prepare("SELECT user_id FROM account_snapshots ORDER BY user_id")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for user in users {
        reconcile_user(transaction, &user, None, now, false, true)?;
    }
    Ok(())
}

pub(super) fn load_complete(
    connection: &Connection,
    user: &str,
    revision: i64,
    raw: &str,
) -> StoreResult<VerifiedMediaSnapshotManifest> {
    let references = references(connection, user, raw, None)?;
    if references.unresolved {
        return Err(StoreError::Integrity(
            "historical media references are opaque or unresolved; restore was not applied".into(),
        ));
    }
    let stored_ids = {
        let mut statement = connection.prepare("SELECT attachment_id FROM account_snapshot_media_history WHERE user_id=?1 AND account_revision=?2")?;
        let rows = statement.query_map(params![user, revision], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<HashSet<_>, _>>()?
    };
    if stored_ids != references.ids {
        return Err(StoreError::Integrity(
            "historical media manifest does not match resolved references".into(),
        ));
    }
    let mut ids = references.ids.iter().collect::<Vec<_>>();
    ids.sort();
    let mut entries = Vec::with_capacity(ids.len());
    for id in ids {
        let entry = historical_entry(connection, user, revision, id)?
            .filter(|entry| references.matches(entry))
            .ok_or_else(|| {
                StoreError::Integrity(format!(
                    "historical media lacks verified independent bytes for attachment {id}"
                ))
            })?;
        entries.push(VerifiedMediaSnapshotEntry::from(&entry));
    }
    Ok(VerifiedMediaSnapshotManifest {
        user_id: user.to_owned(),
        account_revision: revision,
        entries,
    })
}
