// v2.22.38 - Keep job receipts durable and repeatable across clock changes and retries.

use rand::{rngs::OsRng, RngCore};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const BACKGROUND_JOB_STORE_FILE: &str = "desktop_background_jobs.sqlite3";
const BACKGROUND_JOB_SCHEMA_VERSION: i64 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackgroundJobKind {
    Sync,
    Download,
    Upload,
    MediaSync,
}

impl BackgroundJobKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Sync => "sync",
            Self::Download => "download",
            Self::Upload => "upload",
            Self::MediaSync => "media_sync",
        }
    }

    fn parse(value: &str) -> io::Result<Self> {
        match value {
            "sync" => Ok(Self::Sync),
            "download" => Ok(Self::Download),
            "upload" => Ok(Self::Upload),
            "media_sync" => Ok(Self::MediaSync),
            _ => Err(invalid_data(format!(
                "unknown background job kind: {value}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackgroundJobState {
    Pending,
    Running,
    AwaitingReconciliation,
    Completed,
    Failed,
    Cancelled,
}

impl BackgroundJobState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::AwaitingReconciliation => "awaiting_reconciliation",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    fn parse(value: &str) -> io::Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "awaiting_reconciliation" => Ok(Self::AwaitingReconciliation),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(invalid_data(format!(
                "unknown background job state: {value}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackgroundJobRecord {
    pub id: String,
    pub kind: BackgroundJobKind,
    pub state: BackgroundJobState,
    pub workspace_fingerprint: String,
    pub source_revision: i64,
    pub idempotency_key: String,
    pub created_at_epoch_millis: i64,
    pub updated_at_epoch_millis: i64,
    pub last_error: String,
}

#[derive(Clone, Debug)]
pub struct DesktopBackgroundJobStore {
    path: PathBuf,
}

impl DesktopBackgroundJobStore {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let store = Self {
            path: path.as_ref().to_path_buf(),
        };
        let mut connection = store.connection()?;
        initialize_schema(&mut connection)?;
        Ok(store)
    }

    pub fn stage_job(
        &self,
        kind: BackgroundJobKind,
        workspace_fingerprint: &str,
        source_revision: i64,
        now: i64,
    ) -> io::Result<BackgroundJobRecord> {
        if workspace_fingerprint.trim().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "background job workspace fingerprint is empty",
            ));
        }
        let now = now.max(0);
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        let id = random_identifier("job");
        let idempotency_key = random_identifier("idempotency");
        transaction
            .execute(
                "INSERT INTO desktop_background_jobs (
                    id, kind, state, workspace_fingerprint, source_revision,
                    idempotency_key, created_at_epoch_millis,
                    updated_at_epoch_millis, last_error
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, '')",
                params![
                    id,
                    kind.as_str(),
                    BackgroundJobState::Pending.as_str(),
                    workspace_fingerprint,
                    source_revision,
                    idempotency_key,
                    now,
                ],
            )
            .map_err(sql_error)?;
        transaction.commit().map_err(sql_error)?;
        Ok(BackgroundJobRecord {
            id,
            kind,
            state: BackgroundJobState::Pending,
            workspace_fingerprint: workspace_fingerprint.to_string(),
            source_revision,
            idempotency_key,
            created_at_epoch_millis: now,
            updated_at_epoch_millis: now,
            last_error: String::new(),
        })
    }

    pub fn mark_running(&self, id: &str, now: i64) -> io::Result<()> {
        self.transition(
            id,
            BackgroundJobState::Running,
            now,
            "",
            &[BackgroundJobState::Pending],
        )
    }

    pub fn mark_completed(&self, id: &str, now: i64) -> io::Result<()> {
        self.transition(
            id,
            BackgroundJobState::Completed,
            now,
            "",
            &[
                BackgroundJobState::Pending,
                BackgroundJobState::Running,
                BackgroundJobState::AwaitingReconciliation,
            ],
        )
    }

    pub fn mark_failed(&self, id: &str, now: i64, error: &str) -> io::Result<()> {
        self.transition(
            id,
            BackgroundJobState::Failed,
            now,
            error,
            &[
                BackgroundJobState::Pending,
                BackgroundJobState::Running,
                BackgroundJobState::AwaitingReconciliation,
            ],
        )
    }

    pub fn mark_awaiting_reconciliation(&self, id: &str, now: i64, reason: &str) -> io::Result<()> {
        self.transition(
            id,
            BackgroundJobState::AwaitingReconciliation,
            now,
            reason,
            &[BackgroundJobState::Pending, BackgroundJobState::Running],
        )
    }

    pub fn cancel_pending(&self, id: &str, now: i64, reason: &str) -> io::Result<()> {
        self.transition(
            id,
            BackgroundJobState::Cancelled,
            now,
            reason,
            &[BackgroundJobState::Pending],
        )
    }

    pub fn recover_interrupted(&self, now: i64) -> io::Result<Vec<BackgroundJobRecord>> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        transaction
            .execute(
                "UPDATE desktop_background_jobs
                 SET state = 'awaiting_reconciliation',
                     updated_at_epoch_millis = MAX(updated_at_epoch_millis,
                                                   created_at_epoch_millis, ?1),
                     last_error = CASE
                         WHEN last_error = '' THEN 'client exited before completion was recorded'
                         ELSE last_error
                     END
                 WHERE state IN ('pending', 'running')",
                params![now.max(0)],
            )
            .map_err(sql_error)?;
        let records =
            query_jobs_by_state(&transaction, BackgroundJobState::AwaitingReconciliation)?;
        transaction.commit().map_err(sql_error)?;
        Ok(records)
    }

    pub fn prune_terminal_before(&self, cutoff: i64) -> io::Result<usize> {
        self.connection()?
            .execute(
                "DELETE FROM desktop_background_jobs
                 WHERE state IN ('completed', 'failed', 'cancelled')
                   AND updated_at_epoch_millis < ?1",
                params![cutoff],
            )
            .map_err(sql_error)
    }

    pub fn load(&self, id: &str) -> io::Result<Option<BackgroundJobRecord>> {
        self.connection()?
            .query_row(
                "SELECT id, kind, state, workspace_fingerprint, source_revision,
                        idempotency_key, created_at_epoch_millis,
                        updated_at_epoch_millis, last_error
                 FROM desktop_background_jobs WHERE id = ?1",
                params![id],
                row_to_record,
            )
            .optional()
            .map_err(sql_error)?
            .map(parse_record)
            .transpose()
    }

    fn transition(
        &self,
        id: &str,
        next: BackgroundJobState,
        now: i64,
        error: &str,
        allowed: &[BackgroundJobState],
    ) -> io::Result<()> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        let current = transaction
            .query_row(
                "SELECT id, kind, state, workspace_fingerprint, source_revision,
                        idempotency_key, created_at_epoch_millis,
                        updated_at_epoch_millis, last_error
                 FROM desktop_background_jobs WHERE id = ?1",
                params![id],
                row_to_record,
            )
            .optional()
            .map_err(sql_error)?
            .map(parse_record)
            .transpose()?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "background job not found"))?;
        // A receipt may be delivered again after its durable commit. Confirm
        // that exact terminal state without changing the original evidence.
        // Running remains non-idempotent: accepting it twice could start two
        // workers for one staged operation.
        if current.state == next
            && matches!(
                next,
                BackgroundJobState::Completed
                    | BackgroundJobState::Failed
                    | BackgroundJobState::Cancelled
            )
        {
            transaction.commit().map_err(sql_error)?;
            return Ok(());
        }
        if !allowed.contains(&current.state) {
            return Err(invalid_data(format!(
                "background job {} cannot transition from {} to {}",
                id,
                current.state.as_str(),
                next.as_str()
            )));
        }
        let now = now
            .max(0)
            .max(current.created_at_epoch_millis)
            .max(current.updated_at_epoch_millis);
        let changed = transaction
            .execute(
                "UPDATE desktop_background_jobs
                 SET state = ?1, updated_at_epoch_millis = ?2, last_error = ?3
                 WHERE id = ?4 AND state = ?5",
                params![next.as_str(), now, error, id, current.state.as_str()],
            )
            .map_err(sql_error)?;
        if changed != 1 {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "background job changed concurrently",
            ));
        }
        transaction.commit().map_err(sql_error)?;
        Ok(())
    }

    fn connection(&self) -> io::Result<Connection> {
        let parent = self.path.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "background job database has no parent directory",
            )
        })?;
        std::fs::create_dir_all(parent)?;
        let connection = Connection::open(&self.path).map_err(sql_error)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(sql_error)?;
        // synchronous is a connection setting, so initializing the schema
        // once does not configure the later short-lived write connections.
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = FULL;
                 PRAGMA trusted_schema = OFF;",
            )
            .map_err(sql_error)?;
        let journal_mode: String = connection
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .map_err(sql_error)?;
        let synchronous: i64 = connection
            .pragma_query_value(None, "synchronous", |row| row.get(0))
            .map_err(sql_error)?;
        if !journal_mode.eq_ignore_ascii_case("wal") || synchronous != 2 {
            return Err(invalid_data("background job journal is not durable".into()));
        }
        Ok(connection)
    }
}

fn initialize_schema(connection: &mut Connection) -> io::Result<()> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql_error)?;
    let version: i64 = transaction
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(sql_error)?;
    if version > BACKGROUND_JOB_SCHEMA_VERSION || version < 0 {
        return Err(invalid_data(format!(
            "background job schema {version} is newer than supported {BACKGROUND_JOB_SCHEMA_VERSION}"
        )));
    }
    if version == 0 {
        let legacy_exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master
                 WHERE type = 'table' AND name = 'desktop_background_jobs')",
                [],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        if legacy_exists {
            verify_job_columns(&transaction)?;
            // SQLite cannot alter a CHECK constraint in place. Rename, copy
            // and replace inside one transaction; any row/constraint failure
            // restores the original schema and every original receipt.
            transaction
                .execute_batch(
                    "ALTER TABLE desktop_background_jobs RENAME TO desktop_background_jobs_v0;
                     DROP INDEX IF EXISTS desktop_background_jobs_state_updated;",
                )
                .map_err(sql_error)?;
        }
        create_job_schema(&transaction)?;
        if legacy_exists {
            transaction
                .execute_batch(
                    "INSERT INTO desktop_background_jobs
                 SELECT id, kind, state, workspace_fingerprint, source_revision,
                        idempotency_key, created_at_epoch_millis,
                        updated_at_epoch_millis, last_error
                 FROM desktop_background_jobs_v0;
                 DROP TABLE desktop_background_jobs_v0;",
                )
                .map_err(sql_error)?;
        }
        transaction
            .pragma_update(None, "user_version", BACKGROUND_JOB_SCHEMA_VERSION)
            .map_err(sql_error)?;
    }
    verify_job_columns(&transaction)?;
    transaction.commit().map_err(sql_error)
}

fn verify_job_columns(connection: &Connection) -> io::Result<()> {
    let mut statement = connection
        .prepare("PRAGMA table_info(desktop_background_jobs)")
        .map_err(sql_error)?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_error)?;
    if columns
        != [
            "id",
            "kind",
            "state",
            "workspace_fingerprint",
            "source_revision",
            "idempotency_key",
            "created_at_epoch_millis",
            "updated_at_epoch_millis",
            "last_error",
        ]
    {
        return Err(invalid_data(
            "background job schema has unexpected columns".into(),
        ));
    }
    Ok(())
}

fn create_job_schema(connection: &Connection) -> io::Result<()> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS desktop_background_jobs (
                 id TEXT PRIMARY KEY,
                 kind TEXT NOT NULL CHECK(kind IN ('sync', 'download', 'upload', 'media_sync')),
                 state TEXT NOT NULL CHECK(state IN (
                     'pending', 'running', 'awaiting_reconciliation',
                     'completed', 'failed', 'cancelled'
                 )),
                 workspace_fingerprint TEXT NOT NULL,
                 source_revision INTEGER NOT NULL,
                 idempotency_key TEXT NOT NULL UNIQUE,
                 created_at_epoch_millis INTEGER NOT NULL,
                 updated_at_epoch_millis INTEGER NOT NULL,
                 last_error TEXT NOT NULL DEFAULT ''
             );
             CREATE INDEX IF NOT EXISTS desktop_background_jobs_state_updated
                 ON desktop_background_jobs(state, updated_at_epoch_millis);",
        )
        .map_err(sql_error)
}

fn query_jobs_by_state(
    connection: &Connection,
    state: BackgroundJobState,
) -> io::Result<Vec<BackgroundJobRecord>> {
    let mut statement = connection
        .prepare(
            "SELECT id, kind, state, workspace_fingerprint, source_revision,
                    idempotency_key, created_at_epoch_millis,
                    updated_at_epoch_millis, last_error
             FROM desktop_background_jobs
             WHERE state = ?1
             ORDER BY created_at_epoch_millis, id",
        )
        .map_err(sql_error)?;
    let rows = statement
        .query_map(params![state.as_str()], row_to_record)
        .map_err(sql_error)?;
    rows.map(|row| row.map_err(sql_error).and_then(parse_record))
        .collect()
}

fn row_to_record(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<(
    String,
    String,
    String,
    String,
    i64,
    String,
    i64,
    i64,
    String,
)> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
    ))
}

fn parse_record(
    raw: (
        String,
        String,
        String,
        String,
        i64,
        String,
        i64,
        i64,
        String,
    ),
) -> io::Result<BackgroundJobRecord> {
    Ok(BackgroundJobRecord {
        id: raw.0,
        kind: BackgroundJobKind::parse(&raw.1)?,
        state: BackgroundJobState::parse(&raw.2)?,
        workspace_fingerprint: raw.3,
        source_revision: raw.4,
        idempotency_key: raw.5,
        created_at_epoch_millis: raw.6,
        updated_at_epoch_millis: raw.7,
        last_error: raw.8,
    })
}

fn random_identifier(prefix: &str) -> String {
    let mut bytes = [0_u8; 16];
    OsRng.fill_bytes(&mut bytes);
    let mut encoded = String::with_capacity(prefix.len() + 1 + bytes.len() * 2);
    encoded.push_str(prefix);
    encoded.push('-');
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    encoded
}

fn sql_error(error: rusqlite::Error) -> io::Error {
    io::Error::new(io::ErrorKind::Other, error)
}

fn invalid_data(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_store(label: &str) -> (PathBuf, DesktopBackgroundJobStore) {
        let path = std::env::temp_dir().join(format!(
            "gridtimer-background-jobs-{label}-{}-{}.sqlite3",
            std::process::id(),
            random_identifier("test")
        ));
        let store = DesktopBackgroundJobStore::open(&path).unwrap();
        (path, store)
    }

    fn cleanup(path: &Path) {
        for candidate in [
            path.to_path_buf(),
            PathBuf::from(format!("{}-wal", path.display())),
            PathBuf::from(format!("{}-shm", path.display())),
        ] {
            let _ = std::fs::remove_file(candidate);
        }
    }

    #[test]
    fn job_is_durable_before_running_and_finishes_explicitly() {
        let (path, store) = temporary_store("complete");
        let job = store
            .stage_job(BackgroundJobKind::Sync, "workspace-a", 7, 100)
            .unwrap();
        assert_eq!(
            BackgroundJobState::Pending,
            store.load(&job.id).unwrap().unwrap().state
        );
        store.mark_running(&job.id, 101).unwrap();
        assert_eq!(
            BackgroundJobState::Running,
            store.load(&job.id).unwrap().unwrap().state
        );
        store.mark_completed(&job.id, 102).unwrap();
        assert_eq!(
            BackgroundJobState::Completed,
            store.load(&job.id).unwrap().unwrap().state
        );
        cleanup(&path);
    }

    #[test]
    fn interrupted_running_job_requires_reconciliation_after_restart() {
        let (path, store) = temporary_store("recover");
        let job = store
            .stage_job(BackgroundJobKind::MediaSync, "workspace-b", 9, 200)
            .unwrap();
        store.mark_running(&job.id, 201).unwrap();
        let reopened = DesktopBackgroundJobStore::open(&path).unwrap();
        let recovered = reopened.recover_interrupted(300).unwrap();
        assert_eq!(1, recovered.len());
        assert_eq!(job.id, recovered[0].id);
        assert_eq!(
            BackgroundJobState::AwaitingReconciliation,
            recovered[0].state
        );
        cleanup(&path);
    }

    #[test]
    fn every_write_connection_keeps_full_wal_durability() {
        let (path, store) = temporary_store("durable_connections");
        for _ in 0..3 {
            let connection = store.connection().unwrap();
            let mode: String = connection
                .pragma_query_value(None, "journal_mode", |row| row.get(0))
                .unwrap();
            let synchronous: i64 = connection
                .pragma_query_value(None, "synchronous", |row| row.get(0))
                .unwrap();
            assert_eq!(mode, "wal");
            assert_eq!(synchronous, 2);
        }
        cleanup(&path);
    }

    #[test]
    fn duplicate_terminal_receipts_preserve_evidence_without_restarting_jobs() {
        let (path, store) = temporary_store("duplicate_receipts");
        let completed = store
            .stage_job(BackgroundJobKind::Sync, "workspace-a", 1, 100)
            .unwrap();
        store.mark_running(&completed.id, 110).unwrap();
        assert!(store.mark_running(&completed.id, 111).is_err());
        store.mark_completed(&completed.id, 120).unwrap();
        let first_receipt = store.load(&completed.id).unwrap().unwrap();
        let reopened = DesktopBackgroundJobStore::open(&path).unwrap();
        reopened.mark_completed(&completed.id, 130).unwrap();
        assert_eq!(
            first_receipt,
            reopened.load(&completed.id).unwrap().unwrap()
        );
        assert!(reopened
            .mark_failed(&completed.id, 140, "late error")
            .is_err());

        let failed = store
            .stage_job(BackgroundJobKind::Sync, "workspace-a", 2, 200)
            .unwrap();
        store.mark_failed(&failed.id, 210, "first failure").unwrap();
        let first_failure = store.load(&failed.id).unwrap().unwrap();
        store
            .mark_failed(&failed.id, 220, "duplicate failure")
            .unwrap();
        assert_eq!(first_failure, store.load(&failed.id).unwrap().unwrap());
        assert!(store.mark_completed(&failed.id, 230).is_err());

        let cancelled = store
            .stage_job(BackgroundJobKind::Sync, "workspace-a", 3, 300)
            .unwrap();
        store
            .cancel_pending(&cancelled.id, 310, "not started")
            .unwrap();
        let first_cancellation = store.load(&cancelled.id).unwrap().unwrap();
        store
            .cancel_pending(&cancelled.id, 320, "duplicate cancellation")
            .unwrap();
        assert_eq!(
            first_cancellation,
            store.load(&cancelled.id).unwrap().unwrap()
        );
        assert!(store.mark_running(&cancelled.id, 330).is_err());
        cleanup(&path);
    }

    #[test]
    fn clock_rollback_does_not_age_out_unresolved_or_newly_completed_jobs() {
        let (path, store) = temporary_store("clock_rollback");
        let job = store
            .stage_job(BackgroundJobKind::MediaSync, "workspace-a", 1, 1_000)
            .unwrap();
        store.mark_running(&job.id, 500).unwrap();
        assert_eq!(
            1_000,
            store
                .load(&job.id)
                .unwrap()
                .unwrap()
                .updated_at_epoch_millis
        );
        let recovered = store.recover_interrupted(100).unwrap();
        assert_eq!(1_000, recovered[0].updated_at_epoch_millis);
        assert_eq!(0, store.prune_terminal_before(2_000).unwrap());
        store.mark_completed(&job.id, 200).unwrap();
        assert_eq!(0, store.prune_terminal_before(900).unwrap());
        assert_eq!(
            1_000,
            store
                .load(&job.id)
                .unwrap()
                .unwrap()
                .updated_at_epoch_millis
        );
        assert_eq!(1, store.prune_terminal_before(1_001).unwrap());
        cleanup(&path);
    }

    fn legacy_store(label: &str) -> (PathBuf, DesktopBackgroundJobStore) {
        let (path, store) = temporary_store(label);
        let connection = store.connection().unwrap();
        let schema: String = connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name = 'desktop_background_jobs'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let legacy_schema = schema.replace(
            "'sync', 'download', 'upload', 'media_sync'",
            "'sync', 'media_sync'",
        );
        assert_ne!(schema, legacy_schema);
        connection
            .execute_batch("DROP TABLE desktop_background_jobs;")
            .unwrap();
        connection.execute_batch(&legacy_schema).unwrap();
        connection.pragma_update(None, "user_version", 0).unwrap();
        drop(connection);
        (path, store)
    }

    #[test]
    fn legacy_schema_migration_preserves_receipts_and_distinguishes_sync_directions() {
        let (path, store) = legacy_store("legacy_migration");
        let job = store
            .stage_job(BackgroundJobKind::Sync, "workspace", 7, 100)
            .unwrap();
        store.mark_running(&job.id, 200).unwrap();
        let original = store.recover_interrupted(300).unwrap().remove(0);
        let reopened = DesktopBackgroundJobStore::open(&path).unwrap();
        assert_eq!(Some(original), reopened.load(&job.id).unwrap());
        for kind in [
            BackgroundJobKind::Sync,
            BackgroundJobKind::Upload,
            BackgroundJobKind::Download,
            BackgroundJobKind::MediaSync,
        ] {
            let staged = reopened.stage_job(kind, "workspace", 8, 400).unwrap();
            assert_eq!(kind, reopened.load(&staged.id).unwrap().unwrap().kind);
        }
        let connection = reopened.connection().unwrap();
        assert_eq!(
            BACKGROUND_JOB_SCHEMA_VERSION,
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap()
        );
        drop(connection);
        cleanup(&path);
    }

    #[test]
    fn rejected_legacy_row_rolls_back_schema_replacement_and_preserves_evidence() {
        let (path, store) = legacy_store("migration_rollback");
        let job = store
            .stage_job(BackgroundJobKind::Sync, "workspace", 7, 100)
            .unwrap();
        let connection = store.connection().unwrap();
        let old_schema: String = connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name = 'desktop_background_jobs'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        connection
            .pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        connection
            .execute(
                "UPDATE desktop_background_jobs SET kind = 'future_operation' WHERE id = ?1",
                params![job.id],
            )
            .unwrap();
        drop(connection);
        assert!(DesktopBackgroundJobStore::open(&path).is_err());
        let connection = store.connection().unwrap();
        assert_eq!(
            old_schema,
            connection
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE name = 'desktop_background_jobs'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap()
        );
        assert_eq!(
            "future_operation",
            connection
                .query_row(
                    "SELECT kind FROM desktop_background_jobs WHERE id = ?1",
                    params![job.id],
                    |row| row.get::<_, String>(0)
                )
                .unwrap()
        );
        assert_eq!(
            0,
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap()
        );
        assert_eq!(
            0,
            connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE name = 'desktop_background_jobs_v0'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap()
        );
        drop(connection);
        cleanup(&path);
    }

    #[test]
    fn future_job_schema_is_rejected_without_rewriting_receipts() {
        let (path, store) = temporary_store("future_schema");
        let job = store
            .stage_job(BackgroundJobKind::Upload, "workspace", 7, 100)
            .unwrap();
        let connection = store.connection().unwrap();
        connection
            .pragma_update(None, "user_version", BACKGROUND_JOB_SCHEMA_VERSION + 1)
            .unwrap();
        drop(connection);
        assert!(DesktopBackgroundJobStore::open(&path)
            .unwrap_err()
            .to_string()
            .contains("newer than supported"));
        assert_eq!(Some(job.clone()), store.load(&job.id).unwrap());
        let connection = store.connection().unwrap();
        assert_eq!(
            BACKGROUND_JOB_SCHEMA_VERSION + 1,
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap()
        );
        drop(connection);
        cleanup(&path);
    }
}
