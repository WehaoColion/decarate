// v0.0.3 - Reuse only byte-identical, already validated archives in the current process.
// v0.0.2 - Keep schema 10/11 archive data and authenticated recovery provenance intact.
// v0.0.1 - Redact schema 12/13 archives and authenticate their original recovery references.
use super::*;
use crate::desktop_state_store::DesktopPrivacyPolicy;

fn failure(message: &str) -> StoreError {
    StoreError::Integrity(format!("pre-schema privacy: {message}"))
}

fn open_read(path: &Path) -> StoreResult<Connection> {
    if !legacy_privacy::ordinary(path)? {
        return Err(failure("archive is missing"));
    }
    for suffix in ["-wal", "-shm", "-journal"] {
        if fs::symlink_metadata(sqlite_sidecar_path(path, suffix)).is_ok() {
            return Err(failure("archive has an unexpected SQLite sidecar"));
        }
    }
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )?;
    connection.busy_timeout(DEFAULT_BUSY_TIMEOUT)?;
    connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA trusted_schema=OFF;")?;
    Ok(connection)
}

pub(super) fn supports_archive_schema(version: i64) -> bool {
    (10..=14).contains(&version)
}

pub(super) fn verify(path: &Path, timestamp: i64) -> StoreResult<VerifiedBackupReport> {
    backup_verification::verify(
        path,
        timestamp,
        backup_verification::Domain::PreSchema,
        || verify_uncached(path, timestamp),
    )
}

fn verify_uncached(path: &Path, timestamp: i64) -> StoreResult<VerifiedBackupReport> {
    let connection = open_read(path)?;
    let version = current_schema_version(&connection)?;
    if !supports_archive_schema(version) {
        return Err(failure("unsupported pre-schema archive"));
    }
    verify_quick_check(&connection)?;
    verify_integrity_check(&connection)?;
    verify_foreign_keys(&connection)?;
    if version == 14 {
        verify_required_schema_at_version(&connection, version)?;
    }
    verify_semantic_storage_integrity_at_schema(&connection, version)?;
    let server = connection.query_row(
        "SELECT server_instance_id FROM server_identity WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    drop(connection);
    Ok(VerifiedBackupReport {
        destination: path.into(),
        size_bytes: fs::metadata(path)?.len(),
        sha256: sha256_file(path)?,
        created_at_epoch_millis: timestamp,
        server_instance_id: server,
    })
}

fn redact_account(
    transaction: &Transaction<'_>,
    user: &str,
    policy: &DesktopPrivacyPolicy,
    version: i64,
) -> StoreResult<()> {
    let Some(current) = read_account_in_transaction(transaction, user)? else {
        return Ok(());
    };
    let mut candidates = media_privacy::Candidates::new();
    if let Some(redacted) = note_privacy::project(policy, &current.app_data_json)? {
        media_privacy::observe_projection(
            &mut candidates,
            &current.app_data_json,
            &redacted,
            policy,
        )?;
        let generation = restore_generation_in_transaction(transaction, user)?;
        transaction.execute("UPDATE account_snapshots SET app_data_json=?1, content_sha256=?2, envelope_sha256=?3 WHERE user_id=?4",
            params![redacted,sha256_hex(redacted.as_bytes()),account_snapshot_envelope_sha256(user,&redacted,current.revision,current.updated_at_epoch_millis,generation),user])?;
        if version >= 13 {
            replace_current_snapshot_media_identities(transaction, user, &redacted)?;
        }
    }
    note_privacy::redact_history(transaction, user, policy, &mut candidates)?;
    media_privacy::purge(transaction, user, candidates, policy)?;
    let mut last = 0;
    loop {
        let row=transaction.query_row("SELECT rowid,response_json FROM request_dedup WHERE user_id=?1 AND rowid>?2 ORDER BY rowid LIMIT 1",params![user,last],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?))).optional()?;
        let Some((id, raw)) = row else { break };
        if let Some(redacted) = note_privacy::redact_response(policy, &raw)? {
            transaction.execute(
                "UPDATE request_dedup SET response_json=?1 WHERE rowid=?2",
                params![redacted, id],
            )?;
        }
        last = id;
    }
    Ok(())
}

pub(super) fn redact_copy(
    candidate: &Path,
    database: &Path,
    timestamp: i64,
) -> StoreResult<VerifiedBackupReport> {
    let before = verify(candidate, timestamp)?;
    let policies = privacy_journal::recovery_policies(database, &before.server_instance_id)?;
    if policies.is_empty() {
        return Ok(before);
    }
    let mut connection = Connection::open_with_flags(
        candidate,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )?;
    connection.busy_timeout(DEFAULT_BUSY_TIMEOUT)?;
    connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA trusted_schema=OFF; PRAGMA journal_mode=DELETE; PRAGMA synchronous=EXTRA; PRAGMA secure_delete=ON;")?;
    let version = current_schema_version(&connection)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for (user, policy) in policies {
        redact_account(&transaction, &user, &policy, version)?;
    }
    transaction.commit()?;
    verify_semantic_storage_integrity_at_schema(&connection, version)?;
    verify_foreign_keys(&connection)?;
    connection.execute_batch("VACUUM;")?;
    drop(connection);
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(candidate)?
        .sync_all()?;
    verify(candidate, timestamp)
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Image {
    sha256: String,
    size: u64,
    sources: Vec<LegacySnapshotRepairSource>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Proof {
    version: u32,
    target: String,
    server: String,
    file: String,
    schema: i64,
    timestamp: i64,
    origin: Image,
    current: Image,
    previous: Option<Image>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Signed {
    proof: Proof,
    mac: String,
}

fn proof_path(database: &Path, file: &str) -> StoreResult<PathBuf> {
    let file = if cfg!(windows) {
        file.to_lowercase()
    } else {
        file.into()
    };
    let target = privacy_journal::target_fingerprint(database)?;
    Ok(database.with_file_name(format!(
        ".preschema_privacy_{}_{}.json",
        &target[..16],
        &sha256_hex(file.as_bytes())[..24]
    )))
}
fn key(database: &Path) -> StoreResult<(String, Vec<u8>)> {
    let connection = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let (server,secret):(String,String)=connection.query_row("SELECT server_instance_id,workspace_capability_secret FROM server_identity WHERE singleton=1",[],|r|Ok((r.get(0)?,r.get(1)?)))?;
    Ok((server, decode_lowercase_hex_32(&secret)?.to_vec()))
}
fn mac(proof: &Proof, key: &[u8]) -> StoreResult<String> {
    let mut bytes = b"preschema-privacy-provenance-v1\0".to_vec();
    bytes.extend_from_slice(&serde_json::to_vec(proof)?);
    Ok(hmac_sha256_hex(key, &bytes))
}
fn valid_image(image: &Image) -> bool {
    image.size > 0
        && valid_lowercase_opaque_identifier(&image.sha256)
        && image.sources.iter().all(|source| {
            source.revision >= 0 && valid_lowercase_opaque_identifier(&source.content_sha256)
        })
        && image
            .sources
            .windows(2)
            .all(|pair| pair[0].user_id < pair[1].user_id)
}
fn read_proof(database: &Path, file: &str, server: &str, key: &[u8]) -> StoreResult<Option<Proof>> {
    let path = proof_path(database, file)?;
    if !legacy_privacy::ordinary(&path)? {
        return Ok(None);
    }
    if fs::metadata(&path)?.len() > 128 * 1024 {
        return Err(failure("provenance is too large"));
    }
    let signed: Signed = serde_json::from_slice(&fs::read(&path)?)?;
    let proof = signed.proof;
    if !constant_time_bytes_eq(signed.mac.as_bytes(), mac(&proof, key)?.as_bytes()) {
        return Err(failure("provenance authentication failed"));
    }
    let same_file = if cfg!(windows) {
        proof.file.to_lowercase() == file.to_lowercase()
    } else {
        proof.file == file
    };
    if proof.version != 1
        || proof.target != privacy_journal::target_fingerprint(database)?
        || proof.server != server
        || !same_file
        || !supports_archive_schema(proof.schema)
        || proof.timestamp < 0
        || !valid_image(&proof.origin)
        || !valid_image(&proof.current)
        || proof
            .previous
            .as_ref()
            .is_some_and(|image| !valid_image(image))
    {
        return Err(failure("provenance identity failed"));
    }
    Ok(Some(proof))
}
fn image(report: &VerifiedBackupReport) -> StoreResult<Image> {
    verify_published_backup_identity(report)?;
    let connection = open_read(&report.destination)?;
    let sources = legacy_snapshot_repair_manifest(&connection)?;
    drop(connection);
    verify_published_backup_identity(report)?;
    Ok(Image {
        sha256: report.sha256.clone(),
        size: report.size_bytes,
        sources,
    })
}
fn matches_image(report: &VerifiedBackupReport, image: &Image) -> bool {
    report.sha256 == image.sha256 && report.size_bytes == image.size
}

pub(super) fn record_rewrite(
    database: &Path,
    original: &Path,
    replacement: &Path,
    timestamp: i64,
) -> StoreResult<()> {
    let before = verify(original, timestamp)?;
    let after = verify(replacement, timestamp)?;
    let (server, key) = key(database)?;
    if before.server_instance_id != server || after.server_instance_id != server {
        return Err(failure("rewrite changed service identity"));
    }
    let file = controlled_backup_file_name(original)?;
    let before_image = image(&before)?;
    let after_image = image(&after)?;
    if before_image
        .sources
        .iter()
        .map(|s| (&s.user_id, s.revision))
        .collect::<Vec<_>>()
        != after_image
            .sources
            .iter()
            .map(|s| (&s.user_id, s.revision))
            .collect::<Vec<_>>()
    {
        return Err(failure("rewrite changed accounts or revisions"));
    }
    let schema = current_schema_version(&open_read(original)?)?;
    if current_schema_version(&open_read(replacement)?)? != schema {
        return Err(failure("rewrite changed schema"));
    }
    let proof = if let Some(mut proof) = read_proof(database, &file, &server, &key)? {
        if proof.schema != schema
            || proof.timestamp != timestamp
            || !(matches_image(&before, &proof.current)
                || matches_image(&before, &proof.origin)
                || proof
                    .previous
                    .as_ref()
                    .is_some_and(|old| matches_image(&before, old)))
        {
            return Err(failure("archive was replaced outside cleanup"));
        }
        proof.previous = Some(before_image);
        proof.current = after_image;
        proof
    } else {
        Proof {
            version: 1,
            target: privacy_journal::target_fingerprint(database)?,
            server,
            file: file.clone(),
            schema,
            timestamp,
            origin: before_image.clone(),
            current: after_image,
            previous: Some(before_image),
        }
    };
    let path = proof_path(database, &file)?;
    let temporary = path.with_file_name(format!(
        ".preschema_metadata_{}.tmp",
        random_opaque_identifier()
    ));
    let bytes = serde_json::to_vec(&Signed {
        mac: mac(&proof, &key)?,
        proof,
    })?;
    if bytes.len() > 128 * 1024 {
        return Err(failure("provenance exceeds its limit"));
    }
    let result = (|| -> StoreResult<()> {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        output.write_all(&bytes)?;
        output.sync_all()?;
        drop(output);
        legacy_privacy::publish(&temporary, &path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

pub(super) fn contains_source(
    database: &Path,
    backup: &LegacySnapshotRepairBackupIdentity,
    source: &LegacySnapshotRepairSource,
) -> StoreResult<bool> {
    let (server, key) = key(database)?;
    let Some(proof) = read_proof(database, &backup.file_name, &server, &key)? else {
        return Ok(false);
    };
    if proof.schema != backup.schema_version
        || proof.timestamp != backup.created_at_epoch_millis
        || proof.origin.sha256 != backup.sha256
        || proof.origin.size != backup.size_bytes as u64
        || !proof.origin.sources.contains(source)
    {
        return Ok(false);
    }
    let path = database.with_file_name(&backup.file_name);
    let current = verify(&path, proof.timestamp)?;
    let recorded = if matches_image(&current, &proof.current) {
        &proof.current
    } else if let Some(previous) = proof
        .previous
        .as_ref()
        .filter(|previous| matches_image(&current, previous))
    {
        previous
    } else {
        return Err(failure("redacted archive fingerprint changed"));
    };
    // Keep the original audit and one-shot allowance roots immutable. Only an
    // authenticated privacy transform can substitute their physical archive.
    let actual = image(&current)?;
    Ok(actual.sources == recorded.sources
        && actual
            .sources
            .iter()
            .any(|row| row.user_id == source.user_id && row.revision == source.revision))
}

impl SqliteServerStore {
    pub(crate) fn verify_managed_privacy_backup(
        path: &Path,
        timestamp: i64,
    ) -> StoreResult<VerifiedBackupReport> {
        let connection = open_read(path)?;
        let schema = current_schema_version(&connection)?;
        drop(connection);
        if supports_archive_schema(schema) {
            verify(path, timestamp)
        } else {
            Self::verify_existing_backup(path, timestamp)
        }
    }
    pub(crate) fn record_preschema_privacy_rewrite(
        database: &Path,
        original: &Path,
        replacement: &Path,
        timestamp: i64,
    ) -> StoreResult<()> {
        let connection = open_read(original)?;
        let schema = current_schema_version(&connection)?;
        drop(connection);
        if supports_archive_schema(schema) {
            record_rewrite(database, original, replacement, timestamp)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
impl SqliteServerStore {
    pub(crate) fn preschema_test_seed_history_and_receipts(&self) -> StoreResult<()> {
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction()?;
        for user in ["user-1", "user-2"] {
            let current = read_account_in_transaction(&transaction, user)?.unwrap();
            insert_snapshot_history(&transaction, &current, 120)?;
            let response =
                serde_json::json!({"ok":true,"appDataJson":current.app_data_json}).to_string();
            transaction.execute("INSERT INTO request_dedup(request_id,user_id,request_fingerprint,response_json,account_revision,created_at_epoch_millis) VALUES(?1,?2,?3,?4,0,120)",
                params![format!("old-receipt-{user}"),user,sha256_hex(user.as_bytes()),response])?;
        }
        transaction.commit()?;
        Ok(())
    }
    pub(crate) fn preschema_test_v13_allowances(&self) -> StoreResult<()> {
        let connection = self.open_connection(false)?;
        connection.execute_batch("DROP TABLE legacy_snapshot_repair_allowances;")?;
        connection.execute_batch(legacy_snapshot_repair_allowance_v13_table_sql())?;
        connection.execute("INSERT INTO legacy_snapshot_repair_allowances SELECT user_id,revision,content_sha256,130 FROM account_snapshots",[])?;
        Ok(())
    }
    pub(crate) fn preschema_test_repair(
        &self,
        user: &str,
        incoming: &str,
        consume: bool,
    ) -> StoreResult<bool> {
        let current = self.read_account(user)?;
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction()?;
        let Some(backup) = legacy_snapshot_repair_authorization(
            &transaction,
            self.database_path(),
            &current,
            incoming,
        )?
        else {
            return Ok(false);
        };
        if consume {
            record_legacy_snapshot_repair_history_omission(&transaction, &current, 300, &backup)?;
            consume_legacy_snapshot_repair_allowance(&transaction, &current, &backup)?;
            transaction.commit()?;
        }
        Ok(true)
    }
}

#[cfg(test)]
thread_local! {
    pub(super) static MIGRATE_ONLY_THROUGH: std::cell::Cell<i64> = const { std::cell::Cell::new(16) };
}

#[cfg(test)]
impl SqliteServerStore {
    /// Build the genuine v14 layout by applying only its forward migrations.
    /// No version marker is reset and no later-format tables are removed.
    pub(crate) fn preschema_test_create_native14(
        database: &Path,
        accounts: &[(&str, &str)],
        media: &[(&str, &str, &[u8])],
    ) -> StoreResult<VerifiedBackupReport> {
        Self::preschema_test_create_native_layout(database, 14, accounts, media)
    }

    pub(crate) fn preschema_test_create_native_layout(
        database: &Path,
        version: i64,
        accounts: &[(&str, &str)],
        media: &[(&str, &str, &[u8])],
    ) -> StoreResult<VerifiedBackupReport> {
        if !matches!(version, 14 | 15) || database.exists() {
            return Err(failure("native test source version/path is invalid"));
        }
        if let Some(parent) = database.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut connection = Connection::open(database)?;
        connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=DELETE;")?;
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                MIGRATE_ONLY_THROUGH.with(|value| value.set(16));
            }
        }
        {
            MIGRATE_ONLY_THROUGH.with(|value| value.set(version));
            let _reset = Reset;
            apply_schema_migrations(&mut connection, 100, None)?;
        }
        let server: String = connection.query_row(
            "SELECT server_instance_id FROM server_identity WHERE singleton=1",
            [],
            |row| row.get(0),
        )?;
        let transaction = connection.transaction()?;
        for (user, raw) in accounts {
            let digest = sha256_hex(raw.as_bytes());
            transaction.execute(
                "INSERT INTO users(id,email,password_salt,password_hash,password_scheme,created_at_epoch_millis,updated_at_epoch_millis)
                 VALUES(?1,?2,'fixture-salt',?3,'legacy_sha256',100,100)",
                params![user, format!("{user}@native14.example.test"), sha256_hex(b"fixture-password")],
            )?;
            transaction.execute("INSERT INTO account_namespaces(user_id,account_namespace,created_at_epoch_millis) VALUES(?1,?2,100)",
                params![user,account_namespace_identifier(&server,user)])?;
            transaction.execute("INSERT INTO account_snapshots(user_id,app_data_json,revision,updated_at_epoch_millis,content_sha256,envelope_sha256)
                VALUES(?1,?2,0,100,?3,?4)",params![user,raw,digest,account_snapshot_envelope_sha256(user,raw,0,100,0)])?;
            replace_current_snapshot_media_identities(&transaction, user, raw)?;
            ensure_snapshot_content(&transaction, &digest, raw, 100)?;
            transaction.execute("INSERT INTO account_snapshot_history(user_id,revision,content_sha256,created_at_epoch_millis,media_snapshot_complete)
                VALUES(?1,0,?2,100,1)", params![user,digest])?;
        }
        for (user, id, bytes) in media {
            transaction.execute("INSERT INTO note_media(user_id,attachment_id,sha256,mime_type,size_bytes,content,updated_at_epoch_millis,deleted_at_epoch_millis)
                VALUES(?1,?2,?3,'application/octet-stream',?4,?5,100,NULL)",
                params![user,id,sha256_hex(bytes),bytes.len() as i64,bytes])?;
        }
        for (user, raw) in accounts {
            for (id, expected) in referenced_media_expectations_at_format(raw, false)? {
                let bytes = media
                    .iter()
                    .find(|(owner, attachment, _)| owner == user && *attachment == id)
                    .ok_or_else(|| failure("native v14 fixture media is absent"))?
                    .2;
                if sha256_hex(bytes) != expected.sha256 || bytes.len() as i64 != expected.size_bytes
                {
                    return Err(failure("native v14 fixture media does not match its note"));
                }
                ensure_media_snapshot_content(&transaction, &expected.sha256, bytes, 100)?;
                transaction.execute("INSERT INTO account_snapshot_media_history(user_id,account_revision,attachment_id,content_sha256,declared_sha256,mime_type,size_bytes,updated_at_epoch_millis,missing_reason)
                    VALUES(?1,0,?2,?3,?3,?4,?5,?6,'')",params![user,id,expected.sha256,expected.mime_type,expected.size_bytes,expected.updated_at_epoch_millis])?;
            }
        }
        transaction.commit()?;
        verify_required_schema_at_version(&connection, version)?;
        verify_semantic_storage_integrity_at_schema(&connection, version)?;
        drop(connection);
        Self::verify_existing_backup(database, 100)
    }
}
