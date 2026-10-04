// v0.0.8 - Migrate the independent privacy format without changing committed or pending fences.
// v0.0.7 - Recheck old completion certificates under attachment-aware privacy maintenance.
// v0.0.6 - Preserve schema 10/11 when applying external privacy to managed archives.
// v0.0.5 - Expose verified recovery fences to legacy-file maintenance.
// v0.0.4 - Publish a fully initialized journal without replacing existing evidence.
// v0.0.3 - Wake backup maintenance after a durable main privacy commit.
// v0.0.2 - Bind managed backup cleanup to a verified privacy generation.
// v0.0.1 - Retain committed privacy fences independently of the recoverable main database.
use super::*;
use crate::desktop_state_store::DesktopPrivacyPolicy;

fn failure(message: impl fmt::Display) -> StoreError {
    StoreError::Integrity(format!("independent privacy journal: {message}"))
}

pub(super) fn witness_schema_sql() -> &'static str {
    "CREATE TABLE note_privacy_commit_witnesses (
        commit_id TEXT PRIMARY KEY NOT NULL CHECK(length(commit_id)=64),
        user_id TEXT NOT NULL,
        binding_sha256 TEXT NOT NULL CHECK(length(binding_sha256)=64)
    ) STRICT"
}

const JOURNAL_SCHEMA: &str = "
    CREATE TABLE privacy_journal_identity (
        singleton INTEGER PRIMARY KEY CHECK(singleton=1),
        format_version INTEGER NOT NULL CHECK(format_version=2),
        target_fingerprint TEXT NOT NULL,
        server_instance_id TEXT NOT NULL
    ) STRICT;
    CREATE TABLE privacy_journal_policies (
        user_id TEXT PRIMARY KEY NOT NULL,
        policy_json TEXT NOT NULL CHECK(json_valid(policy_json)),
        binding_sha256 TEXT NOT NULL CHECK(length(binding_sha256)=64)
    ) STRICT;
    CREATE TABLE privacy_journal_pending (
        commit_id TEXT PRIMARY KEY NOT NULL CHECK(length(commit_id)=64),
        user_id TEXT NOT NULL,
        policy_json TEXT NOT NULL CHECK(json_valid(policy_json)),
        binding_sha256 TEXT NOT NULL CHECK(length(binding_sha256)=64)
    ) STRICT;
    CREATE TABLE privacy_journal_commits (
        commit_id TEXT PRIMARY KEY NOT NULL CHECK(length(commit_id)=64),
        user_id TEXT NOT NULL,
        binding_sha256 TEXT NOT NULL CHECK(length(binding_sha256)=64)
    ) STRICT;
";

#[cfg(test)]
pub(super) fn create_native_v1_fixture(store: &SqliteServerStore) -> StoreResult<()> {
    // Construct the original physical schema, then seed account-bound policy
    // rows. Never relabel a format-2 identity table as a format-1 database.
    const V1_SCHEMA: &str = "
        CREATE TABLE privacy_journal_identity (
            singleton INTEGER PRIMARY KEY CHECK(singleton=1),
            format_version INTEGER NOT NULL CHECK(format_version=1),
            target_fingerprint TEXT NOT NULL, server_instance_id TEXT NOT NULL
        ) STRICT;
        CREATE TABLE privacy_journal_policies (
            user_id TEXT PRIMARY KEY NOT NULL, policy_json TEXT NOT NULL CHECK(json_valid(policy_json)),
            binding_sha256 TEXT NOT NULL CHECK(length(binding_sha256)=64)
        ) STRICT;
        CREATE TABLE privacy_journal_pending (
            commit_id TEXT PRIMARY KEY NOT NULL CHECK(length(commit_id)=64), user_id TEXT NOT NULL,
            policy_json TEXT NOT NULL CHECK(json_valid(policy_json)), binding_sha256 TEXT NOT NULL CHECK(length(binding_sha256)=64)
        ) STRICT;
        CREATE TABLE privacy_journal_commits (
            commit_id TEXT PRIMARY KEY NOT NULL CHECK(length(commit_id)=64), user_id TEXT NOT NULL,
            binding_sha256 TEXT NOT NULL CHECK(length(binding_sha256)=64)
        ) STRICT;";
    let source = store.open_connection(false)?;
    if current_schema_version(&source)? != 15 {
        return Err(failure(
            "native journal fixture requires a verified format-15 source",
        ));
    }
    verify_required_schema_at_version(&source, 15)?;
    verify_semantic_storage_integrity_at_schema(&source, 15)?;
    let identity = store.server_instance_id()?;
    let path = journal_path(store.database_path());
    let temporary = path.with_extension("native-v1-fixture.sqlite3");
    if temporary.exists() {
        return Err(failure("native fixture temporary already exists"));
    }
    let journal = Connection::open(&temporary)?;
    journal.execute_batch(V1_SCHEMA)?;
    journal.execute(
        "INSERT INTO privacy_journal_identity VALUES(1,1,?1,?2)",
        params![target_fingerprint(store.database_path())?, identity],
    )?;
    let mut statement = source.prepare(
        "SELECT user_id,policy_json,binding_sha256 FROM account_note_privacy ORDER BY user_id",
    )?;
    for row in statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })? {
        let (user, json, binding) = row?;
        journal.execute(
            "INSERT INTO privacy_journal_policies VALUES(?1,?2,?3)",
            params![user, json, binding],
        )?;
    }
    verify_journal(&journal, store.database_path(), &identity)?;
    drop(journal);
    if path.exists() {
        fs::remove_file(&path)?;
    }
    fs::rename(&temporary, &path)?;
    let reopened = connect_journal(&path)?;
    verify_journal(&reopened, store.database_path(), &identity)
}

pub(super) fn journal_path(database: &Path) -> PathBuf {
    let mut name = database.file_name().unwrap_or_default().to_os_string();
    name.push(".note_privacy.sqlite3");
    database.with_file_name(name)
}

fn database_path(connection: &Connection) -> StoreResult<PathBuf> {
    connection
        .path()
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| failure("a file-backed database is required"))
}

fn server_id(connection: &Connection) -> StoreResult<String> {
    let value: String = connection.query_row(
        "SELECT server_instance_id FROM server_identity WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    if !valid_lowercase_opaque_identifier(&value) {
        return Err(failure("invalid server identity"));
    }
    Ok(value)
}

pub(super) fn target_fingerprint(database: &Path) -> StoreResult<String> {
    let absolute = absolute_path(database)?;
    let parent = fs::canonicalize(
        absolute
            .parent()
            .ok_or_else(|| failure("database has no parent"))?,
    )?;
    let normalized = parent.join(
        absolute
            .file_name()
            .ok_or_else(|| failure("database has no name"))?,
    );
    let mut path = normalized.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        path = path.to_lowercase();
    }
    Ok(sha256_hex(
        format!("independent-note-privacy-v1\0{path}").as_bytes(),
    ))
}

fn ordinary_file(path: &Path) -> StoreResult<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            #[cfg(target_os = "windows")]
            let reparse = {
                use std::os::windows::fs::MetadataExt;
                metadata.file_attributes() & 0x400 != 0
            };
            #[cfg(not(target_os = "windows"))]
            let reparse = metadata.file_type().is_symlink();
            if !metadata.is_file() || reparse {
                return Err(failure("journal path is not an ordinary file"));
            }
            if metadata.len() == 0 {
                return Err(failure("existing journal is empty"));
            }
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn open_journal(
    database: &Path,
    expected_server: &str,
    create: bool,
) -> StoreResult<Option<Connection>> {
    let path = journal_path(database);
    let existed = ordinary_file(&path)?;
    if !existed && !create {
        return Ok(None);
    }
    for suffix in ["-wal", "-shm", "-journal"] {
        let sidecar = sqlite_sidecar_path(&path, suffix);
        match fs::symlink_metadata(&sidecar) {
            Ok(_) => {
                ordinary_file(&sidecar)?;
                if !existed {
                    return Err(failure("orphaned journal sidecar requires inspection"));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    if !existed {
        initialize_journal(database, expected_server)?;
    }
    let mut journal = connect_journal(&path)?;
    verify_journal(&journal, database, expected_server)?;
    journal.pragma_update(None, "journal_mode", "DELETE")?;
    if create {
        upgrade_journal_format(&mut journal, database, expected_server)?;
    }
    Ok(Some(journal))
}

fn upgrade_journal_format(
    journal: &mut Connection,
    database: &Path,
    expected_server: &str,
) -> StoreResult<()> {
    let current: i64 = journal.query_row(
        "SELECT format_version FROM privacy_journal_identity WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    if current == 2 {
        return Ok(());
    }
    let transaction = journal.transaction_with_behavior(TransactionBehavior::Immediate)?;
    verify_journal(&transaction, database, expected_server)?;
    let version: i64 = transaction.query_row(
        "SELECT format_version FROM privacy_journal_identity WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    if version == 1 {
        transaction.execute_batch(
            "ALTER TABLE privacy_journal_identity RENAME TO privacy_journal_identity_v1;",
        )?;
        transaction.execute_batch(JOURNAL_SCHEMA.split(';').next().expect("identity schema"))?;
        transaction.execute_batch("INSERT INTO privacy_journal_identity SELECT singleton,2,target_fingerprint,server_instance_id FROM privacy_journal_identity_v1;
            DROP TABLE privacy_journal_identity_v1;")?;
        verify_journal(&transaction, database, expected_server)?;
        #[cfg(test)]
        if FORMAT_MIGRATION_INTERRUPTION.with(|point| point.get()) {
            return Err(failure("injected format migration interruption"));
        }
        #[cfg(test)]
        format_process_checkpoint(0);
    }
    transaction.commit()?;
    #[cfg(test)]
    if version == 1 {
        format_process_checkpoint(1);
    }
    Ok(())
}

#[cfg(test)]
thread_local! { pub(super) static FORMAT_MIGRATION_INTERRUPTION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

#[cfg(test)]
thread_local! { pub(super) static FORMAT_PROCESS_INTERRUPTION: std::cell::Cell<Option<u8>> = const { std::cell::Cell::new(None) }; }

#[cfg(test)]
fn format_process_checkpoint(point: u8) {
    if FORMAT_PROCESS_INTERRUPTION.with(|value| value.get() == Some(point)) {
        std::process::exit(87);
    }
}

fn connect_journal(path: &Path) -> StoreResult<Connection> {
    let journal = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )?;
    journal.busy_timeout(DEFAULT_BUSY_TIMEOUT)?;
    journal.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA foreign_keys=ON; PRAGMA synchronous=EXTRA; PRAGMA secure_delete=ON;")?;
    Ok(journal)
}

fn verify_journal(journal: &Connection, database: &Path, expected_server: &str) -> StoreResult<()> {
    verify_integrity_check(journal)?;
    let identity = journal.query_row("SELECT format_version,target_fingerprint,server_instance_id FROM privacy_journal_identity WHERE singleton=1", [],
        |row| Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?)))?;
    if !matches!(identity.0, 1 | 2) {
        return Err(failure("unsupported journal format version"));
    }
    if identity.1 != target_fingerprint(database)? || identity.2 != expected_server {
        return Err(failure(
            "journal does not belong to this database and server",
        ));
    }
    // Exact tables and no triggers prevent a divergent sidecar from changing
    // the meaning of a durable publication or silently discarding a fence.
    let mut actual: Vec<String> = journal.prepare("SELECT sql FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")?
        .query_map([], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?;
    actual
        .iter_mut()
        .for_each(|sql| *sql = normalized_schema_sql(sql));
    let schema = if identity.0 == 1 {
        JOURNAL_SCHEMA.replace("format_version=2", "format_version=1")
    } else {
        JOURNAL_SCHEMA.to_owned()
    };
    let mut expected: Vec<String> = schema
        .split(';')
        .filter(|sql| !sql.trim().is_empty())
        .map(normalized_schema_sql)
        .collect();
    actual.sort();
    expected.sort();
    let triggers: i64 = journal.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='trigger'",
        [],
        |row| row.get(0),
    )?;
    if actual != expected || triggers != 0 {
        return Err(failure("journal schema is divergent"));
    }
    Ok(())
}

pub(super) fn initialization_prefix(database: &Path, expected_server: &str) -> StoreResult<String> {
    let binding = sha256_hex(
        format!(
            "journal-init-v1\0{}\0{expected_server}",
            target_fingerprint(database)?
        )
        .as_bytes(),
    );
    Ok(format!(".npi_{}_", &binding[..16]))
}

fn cleanup_unused_initializations(database: &Path, expected_server: &str) -> StoreResult<()> {
    let parent = database
        .parent()
        .ok_or_else(|| failure("database has no parent"))?;
    let prefix = initialization_prefix(database, expected_server)?;
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(random) = name
            .strip_prefix(&prefix)
            .and_then(|s| s.strip_suffix(".sqlite3"))
        else {
            continue;
        };
        if random.len() != 32
            || !random
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            continue;
        }
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        #[cfg(target_os = "windows")]
        let reparse = {
            use std::os::windows::fs::MetadataExt;
            metadata.file_attributes() & 0x400 != 0
        };
        #[cfg(not(target_os = "windows"))]
        let reparse = metadata.file_type().is_symlink();
        if !metadata.is_file() || reparse || ["-wal", "-shm", "-journal"].iter().any(|suffix| {
            // An inaccessible sidecar is also uncertain, so preserve it.
            !matches!(fs::symlink_metadata(sqlite_sidecar_path(&path, suffix)), Err(e) if e.kind() == io::ErrorKind::NotFound)
        }) { continue }
        let unused = if metadata.len() == 0 {
            true
        } else {
            let check = || -> StoreResult<bool> {
                let journal = Connection::open_with_flags(
                    &path,
                    OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
                )?;
                journal.busy_timeout(DEFAULT_BUSY_TIMEOUT)?;
                journal.pragma_update(None, "trusted_schema", "OFF")?;
                verify_journal(&journal, database, expected_server)?;
                Ok(journal.query_row("SELECT NOT EXISTS(SELECT 1 FROM privacy_journal_policies) AND NOT EXISTS(SELECT 1 FROM privacy_journal_pending) AND NOT EXISTS(SELECT 1 FROM privacy_journal_commits)", [], |row| row.get(0))?)
            };
            check().unwrap_or(false)
        };
        if unused {
            let _ = fs::remove_file(&path);
        }
    }
    Ok(())
}

fn initialize_journal(database: &Path, expected_server: &str) -> StoreResult<()> {
    // Callers hold the main database write lock. Temporary files contain only
    // schema and identity; privacy intents are written after publication.
    cleanup_unused_initializations(database, expected_server)?;
    let parent = database
        .parent()
        .ok_or_else(|| failure("database has no parent"))?;
    let temporary = parent.join(format!(
        "{}{}.sqlite3",
        initialization_prefix(database, expected_server)?,
        &random_opaque_identifier()[..32]
    ));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?
        .sync_all()?;
    #[cfg(test)]
    init_checkpoint(0);
    let mut journal = connect_journal(&temporary)?;
    journal.pragma_update(None, "journal_mode", "DELETE")?;
    let transaction = journal.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(JOURNAL_SCHEMA)?;
    transaction.execute(
        "INSERT INTO privacy_journal_identity VALUES(1,2,?1,?2)",
        params![target_fingerprint(database)?, expected_server],
    )?;
    transaction.commit()?;
    #[cfg(test)]
    init_checkpoint(1);
    verify_journal(&journal, database, expected_server)?;
    journal
        .close()
        .map_err(|(_, error)| StoreError::from(error))?;
    OpenOptions::new()
        .write(true)
        .open(&temporary)?
        .sync_all()?;
    #[cfg(test)]
    init_checkpoint(2);
    publish_initial_journal(&temporary, &journal_path(database))?;
    #[cfg(test)]
    init_checkpoint(3);
    Ok(())
}

pub(super) fn publish_initial_journal(temporary: &Path, destination: &Path) -> StoreResult<()> {
    if temporary.parent() != destination.parent() {
        return Err(failure("initialization must stay in the journal directory"));
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
        }
        let source = fs::canonicalize(temporary)?;
        let target = source
            .parent()
            .ok_or_else(|| failure("temporary journal has no parent"))?
            .join(
                destination
                    .file_name()
                    .ok_or_else(|| failure("journal has no name"))?,
            );
        let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        // WRITE_THROUGH only: replacing an existing journal is forbidden.
        if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), 0x8) } == 0 {
            return Err(io::Error::last_os_error().into());
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        fs::hard_link(temporary, destination)?;
        fs::remove_file(temporary)?;
        File::open(
            destination
                .parent()
                .ok_or_else(|| failure("journal has no parent"))?,
        )?
        .sync_all()?;
    }
    Ok(())
}

fn decode_policy(user: &str, json: &str, digest: &str) -> StoreResult<DesktopPrivacyPolicy> {
    if user.is_empty() || digest != note_privacy::binding(user, json) {
        return Err(failure("policy binding is invalid"));
    }
    let policy: DesktopPrivacyPolicy = serde_json::from_str(json)?;
    DesktopPrivacyPolicy::default()
        .merged_policy(&policy)
        .map_err(failure)
}

#[cfg(test)]
thread_local! { pub(super) static INIT_PROCESS_INTERRUPTION: std::cell::Cell<Option<u8>> = const { std::cell::Cell::new(None) }; }

#[cfg(test)]
fn init_checkpoint(point: u8) {
    if INIT_PROCESS_INTERRUPTION.with(|value| value.get() == Some(point)) {
        // End the process without unwinding; no RAII cleanup may help recovery.
        std::process::exit(86);
    }
}

fn for_each_policy<F>(connection: &Connection, mut visit: F) -> StoreResult<()>
where
    F: FnMut(&str, &DesktopPrivacyPolicy) -> StoreResult<()>,
{
    let mut statement = connection.prepare(
        "SELECT user_id,policy_json,binding_sha256 FROM privacy_journal_policies ORDER BY user_id",
    )?;
    for row in statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })? {
        let (user, json, digest) = row?;
        visit(&user, &decode_policy(&user, &json, &digest)?)?;
    }
    Ok(())
}

fn merge_committed(
    journal: &Connection,
    user: &str,
    incoming: &DesktopPrivacyPolicy,
) -> StoreResult<()> {
    let old = journal
        .query_row(
            "SELECT policy_json,binding_sha256 FROM privacy_journal_policies WHERE user_id=?1",
            params![user],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let previous = match old {
        Some((json, digest)) => decode_policy(user, &json, &digest)?,
        None => DesktopPrivacyPolicy::default(),
    };
    let merged = previous.merged_policy(incoming).map_err(failure)?;
    if merged == previous {
        return Ok(());
    }
    let json = serde_json::to_string(&merged)?;
    journal.execute("INSERT INTO privacy_journal_policies VALUES(?1,?2,?3) ON CONFLICT(user_id) DO UPDATE SET policy_json=excluded.policy_json,binding_sha256=excluded.binding_sha256",
        params![user,json,note_privacy::binding(user,&json)])?;
    Ok(())
}

pub(super) fn stage(
    main: &Transaction<'_>,
    user: &str,
    policy: &DesktopPrivacyPolicy,
) -> StoreResult<()> {
    let database = database_path(main)?;
    let identity = server_id(main)?;
    let mut journal = open_journal(&database, &identity, true)?
        .ok_or_else(|| failure("journal was not created"))?;
    let json = serde_json::to_string(policy)?;
    let digest = note_privacy::binding(user, &json);
    let commit_id =
        sha256_hex(format!("privacy-commit-v1\0{identity}\0{user}\0{digest}").as_bytes());
    let transaction = journal.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute("INSERT INTO privacy_journal_pending VALUES(?1,?2,?3,?4) ON CONFLICT(commit_id) DO UPDATE SET user_id=excluded.user_id,policy_json=excluded.policy_json,binding_sha256=excluded.binding_sha256",
        params![commit_id,user,json,digest])?;
    transaction.commit()?;
    // This witness commits atomically with the account and its cleaned history.
    // The sidecar has only a pending intent until that commit is proven.
    main.execute("INSERT INTO note_privacy_commit_witnesses VALUES(?1,?2,?3) ON CONFLICT(commit_id) DO NOTHING",params![commit_id,user,digest])?;
    Ok(())
}

pub(super) fn commit(transaction: Transaction<'_>) -> StoreResult<()> {
    let pending: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM note_privacy_commit_witnesses)",
        [],
        |row| row.get(0),
    )?;
    let database = if pending {
        Some(database_path(&transaction)?)
    } else {
        None
    };
    #[cfg(test)]
    if pending && COMMIT_INTERRUPTION.with(|point| point.get() == 1) {
        return Err(failure("injected interruption before main commit"));
    }
    let notification_key = database.as_deref().map(target_fingerprint).transpose()?;
    transaction.commit()?;
    if let Some(key) = notification_key {
        privacy_notifications::notify(&key);
    }
    #[cfg(test)]
    if pending && COMMIT_INTERRUPTION.with(|point| point.get() == 2) {
        return Err(failure("injected interruption after main commit"));
    }
    if let Some(database_path) = database {
        let mut connection = SqliteServerStore { database_path }.open_connection(false)?;
        reconcile(&mut connection)?;
    }
    Ok(())
}

#[cfg(test)]
thread_local! { pub(super) static COMMIT_INTERRUPTION: std::cell::Cell<u8> = const { std::cell::Cell::new(0) }; }

/// Lock ordering is always main database, then independent journal. The main
/// write lock proves an intent without a witness rolled back, not still in flight.
pub(super) fn reconcile(connection: &mut Connection) -> StoreResult<()> {
    let database = database_path(connection)?;
    let identity = server_id(connection)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let witness_count: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM note_privacy_commit_witnesses",
        [],
        |row| row.get(0),
    )?;
    let policy_count: i64 =
        transaction.query_row("SELECT COUNT(*) FROM account_note_privacy", [], |row| {
            row.get(0)
        })?;
    let exists = ordinary_file(&journal_path(&database))?;
    if !exists && witness_count > 0 {
        return Err(failure("committed privacy intent lost its journal"));
    }
    if !exists && policy_count == 0 {
        transaction.commit()?;
        return Ok(());
    }
    let mut journal =
        open_journal(&database, &identity, true)?.ok_or_else(|| failure("journal unavailable"))?;
    let journal_transaction = journal.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // Retire acknowledgements only while the main write lock proves no current
    // commit depends on them. Cleanup after releasing that lock races a writer
    // which has durably published but has not yet removed its main witness.
    let mut last_ack = String::new();
    loop {
        let id=journal_transaction.query_row("SELECT commit_id FROM privacy_journal_commits WHERE commit_id>?1 ORDER BY commit_id LIMIT 1",params![last_ack],|row|row.get::<_,String>(0)).optional()?;
        let Some(id) = id else {
            break;
        };
        let needed: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM note_privacy_commit_witnesses WHERE commit_id=?1)",
            params![id],
            |row| row.get(0),
        )?;
        if !needed {
            journal_transaction.execute(
                "DELETE FROM privacy_journal_commits WHERE commit_id=?1",
                params![id],
            )?;
        }
        last_ack = id;
    }
    let mut last_pending = String::new();
    loop {
        let pending=journal_transaction.query_row("SELECT commit_id,user_id,policy_json,binding_sha256 FROM privacy_journal_pending WHERE commit_id>?1 ORDER BY commit_id LIMIT 1",params![last_pending],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?))).optional()?;
        let Some((id, user, json, digest)) = pending else {
            break;
        };
        last_pending = id.clone();
        let policy = decode_policy(&user, &json, &digest)?;
        let witness=transaction.query_row("SELECT user_id,binding_sha256 FROM note_privacy_commit_witnesses WHERE commit_id=?1",params![id],
            |row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?))).optional()?;
        if let Some(witness) = witness {
            if witness != (user.clone(), digest.clone()) {
                return Err(failure("commit witness disagrees with intent"));
            }
            merge_committed(&journal_transaction, &user, &policy)?;
            journal_transaction.execute(
                "INSERT OR REPLACE INTO privacy_journal_commits VALUES(?1,?2,?3)",
                params![id, user, digest],
            )?;
        }
        journal_transaction.execute(
            "DELETE FROM privacy_journal_pending WHERE commit_id=?1",
            params![id],
        )?;
    }
    let mut witnesses = transaction
        .prepare("SELECT commit_id,user_id,binding_sha256 FROM note_privacy_commit_witnesses")?;
    for row in witnesses.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })? {
        let (id, user, digest) = row?;
        let published = journal_transaction
            .query_row(
                "SELECT user_id,binding_sha256 FROM privacy_journal_commits WHERE commit_id=?1",
                params![id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        if published != Some((user, digest)) {
            return Err(failure("committed privacy witness has no durable policy"));
        }
    }
    drop(witnesses);
    for_each_policy(&journal_transaction, |user, policy| {
        note_privacy::overlay_policy(&transaction, user, policy)
    })?;
    let users = {
        let mut statement =
            transaction.prepare("SELECT user_id FROM account_note_privacy ORDER BY user_id")?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for user in users {
        merge_committed(
            &journal_transaction,
            &user,
            &note_privacy::read_policy(&transaction, &user)?,
        )?;
    }
    journal_transaction.commit()?;
    #[cfg(test)]
    if COMMIT_INTERRUPTION.with(|point| point.get() == 3) {
        return Err(failure("injected interruption after journal publication"));
    }
    transaction.execute("DELETE FROM note_privacy_commit_witnesses", [])?;
    transaction.commit()?;
    Ok(())
}

fn recovery_journal(database: &Path, identity: &str) -> StoreResult<Option<Connection>> {
    let Some(journal) = open_journal(database, identity, false)? else {
        return Ok(None);
    };
    let pending: i64 =
        journal.query_row("SELECT COUNT(*) FROM privacy_journal_pending", [], |row| {
            row.get(0)
        })?;
    if pending != 0 {
        return Err(StoreError::Io(io::Error::new(
            io::ErrorKind::WouldBlock,
            "missing main database cannot resolve an unfinished privacy commit",
        )));
    }
    Ok(Some(journal))
}

pub(super) fn recovery_policies(
    database: &Path,
    identity: &str,
) -> StoreResult<Vec<(String, DesktopPrivacyPolicy)>> {
    let mut result = Vec::new();
    if let Some(journal) = recovery_journal(database, identity)? {
        for_each_policy(&journal, |user, policy| {
            result.push((user.to_string(), policy.clone()));
            Ok(())
        })?;
    }
    Ok(result)
}

impl SqliteServerStore {
    /// A content-bound generation, including fences for accounts absent from
    /// the current main database but still present in an older backup.
    pub fn backup_privacy_generation(&self) -> StoreResult<Option<String>> {
        self.finish_note_privacy_cleanup()?;
        let identity = self.server_instance_id()?;
        let Some(journal) = recovery_journal(self.database_path(), &identity)? else {
            return Ok(None);
        };
        let mut digest = Sha256::new();
        digest.update(b"managed-backup-privacy-v3-storage16\0");
        let mut count = 0;
        for_each_policy(&journal, |user, policy| {
            let json = serde_json::to_string(policy)?;
            digest.update(note_privacy::binding(user, &json).as_bytes());
            count += 1;
            Ok(())
        })?;
        Ok((count > 0).then(|| format!("{:x}", digest.finalize())))
    }

    /// Transform a verified temporary recovery copy before it becomes the live
    /// database. The original backup remains the provenance for schema migration.
    pub fn apply_external_privacy_to_recovery_copy(
        candidate: &Path,
        original_database: &Path,
        now: i64,
    ) -> StoreResult<VerifiedBackupReport> {
        Self::apply_external_privacy_to_copy(candidate, original_database, now, false)
    }

    /// Managed archives retain their original supported schema, including a
    /// cleaned schema-14 rollback copy. The independent journal keeps fences.
    pub fn apply_external_privacy_to_backup_copy(
        candidate: &Path,
        original_database: &Path,
        now: i64,
    ) -> StoreResult<VerifiedBackupReport> {
        let connection = Connection::open_with_flags(candidate, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let schema = current_schema_version(&connection)?;
        drop(connection);
        if preschema_privacy::supports_archive_schema(schema) {
            return preschema_privacy::redact_copy(candidate, original_database, now);
        }
        // A format-16 migration backup must remain usable by the format-16
        // service for rollback. Privacy projection can be applied in place
        // because this layout already has the complete note privacy tables.
        Self::apply_external_privacy_to_copy(candidate, original_database, now, schema == 16)
    }

    fn apply_external_privacy_to_copy(
        candidate: &Path,
        original_database: &Path,
        now: i64,
        preserve_schema_16: bool,
    ) -> StoreResult<VerifiedBackupReport> {
        let verified = Self::verify_existing_backup(candidate, now)?;
        let Some(journal) = recovery_journal(original_database, &verified.server_instance_id)?
        else {
            return Ok(verified);
        };
        let has_policies: bool = journal.query_row(
            "SELECT EXISTS(SELECT 1 FROM privacy_journal_policies)",
            [],
            |row| row.get(0),
        )?;
        if !has_policies {
            return Ok(verified);
        }
        // Fresh backups already carry the fences. Avoid copying their pages
        // through VACUUM again when no account needs a stronger policy.
        let existing = Connection::open_with_flags(candidate, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let original_schema = current_schema_version(&existing)?;
        if original_schema == SCHEMA_VERSION || (preserve_schema_16 && original_schema == 16) {
            let mut complete = true;
            for_each_policy(&journal, |user, policy| {
                let exists: bool = existing.query_row(
                    "SELECT EXISTS(SELECT 1 FROM account_snapshots WHERE user_id=?1)",
                    params![user],
                    |row| row.get(0),
                )?;
                if exists {
                    let previous = note_privacy::read_policy(&existing, user)?;
                    complete &= previous.merged_policy(policy).map_err(failure)? == previous;
                }
                Ok(())
            })?;
            if complete {
                return Ok(verified);
            }
        }
        drop(existing);
        let mut connection = Connection::open_with_flags(
            candidate,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
        )?;
        connection.busy_timeout(DEFAULT_BUSY_TIMEOUT)?;
        connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA trusted_schema=OFF; PRAGMA journal_mode=DELETE; PRAGMA synchronous=EXTRA; PRAGMA secure_delete=ON;")?;
        if !preserve_schema_16 {
            apply_schema_migrations(&mut connection, now, None)?;
        }
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for_each_policy(&journal, |user, policy| {
            note_privacy::overlay_policy(&transaction, user, policy)
        })?;
        transaction.execute("DELETE FROM note_privacy_commit_witnesses", [])?;
        transaction.commit()?;
        if preserve_schema_16 {
            verify_required_schema_at_version(&connection, 16)?;
            verify_semantic_storage_integrity_at_schema(&connection, 16)?;
        } else {
            verify_required_schema(&connection)?;
            verify_semantic_storage_integrity(&connection)?;
        }
        verify_foreign_keys(&connection)?;
        connection.execute("UPDATE account_note_privacy SET cleanup_pending=0", [])?;
        connection.execute_batch("VACUUM;")?;
        drop(connection);
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(candidate)?
            .sync_all()?;
        Self::verify_existing_backup(candidate, now)
    }
}
