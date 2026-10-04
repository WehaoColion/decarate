// v1.0.3.15 Windows - Recover recently used active sessions after a short offline expiry.
// v0.0.3 - Version private-media storage explicitly and verify format-15 recovery copies.
// v0.0.2 - Commit private declarations with token and restore acknowledgement checks.
// v0.0.1 - Require retained-reference checks for ordinary media DELETE and retention pruning.
// v1.0.2 - Reuse byte-bound content proofs without retaining account data.
// v1.0.1 - Bound repeated archive validation without trusting metadata or disk caches.
// v2.22.56 - Retain note privacy across recovery, replay and imported legacy files.
// v2.22.43 - Exclude deliberately deleted media from historical recovery warnings.
// v2.22.34 - Preserve unchanged legacy attachment references without blocking unrelated sync edits.
//! Transactional persistence for the desktop sync service.
//!
//! This module is deliberately excluded from Android builds. It keeps bearer
//! tokens as SHA-256 fingerprints, uses SQLite WAL transactions, and provides a
//! one-time, backed-up migration path from the legacy `server_store.json` file.

use crate::app_data::APP_DATA_SCHEMA_VERSION;
use crate::sync_identity::account_namespace_identifier;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use rand::{rngs::OsRng, RngCore};
use rusqlite::backup::Backup;
use rusqlite::{
    params, Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[path = "server_media_privacy.rs"]
mod media_privacy;
#[path = "server_private_media_exchange.rs"]
mod private_media_exchange;
#[path = "server_snapshot_media.rs"]
mod snapshot_media;
pub(crate) use media_privacy::MediaDeletionReferenceScope;
pub use media_privacy::MediaPrivacyStatus;

#[path = "server_backup_verification.rs"]
mod backup_verification;
#[cfg(test)]
#[path = "server_backup_verification_tests.rs"]
mod backup_verification_tests;
#[path = "server_content_verification.rs"]
mod content_verification;

#[path = "server_note_privacy.rs"]
mod note_privacy;

#[path = "server_legacy_privacy.rs"]
mod legacy_privacy;
#[path = "server_legal_reports.rs"]
mod legal_reports;
#[path = "server_preschema_privacy.rs"]
mod preschema_privacy;
#[path = "server_privacy_journal.rs"]
mod privacy_journal;
#[path = "server_privacy_notifications.rs"]
mod privacy_notifications;
pub use crate::sync_core::{LegalReportManifestItem, LegalReportTombstone};
#[cfg(test)]
pub(crate) use legacy_privacy::INTERRUPT as LEGACY_PRIVACY_INTERRUPT;
pub use legal_reports::{
    LegalReportDownloadChunk, LegalReportUploadOutcome, MAX_LEGAL_REPORT_CHUNK_BYTES,
};
#[cfg(test)]
pub(crate) use privacy_notifications::{
    privacy_notification_test_key, privacy_notification_test_notify,
};
pub(crate) use privacy_notifications::{BackupPrivacySubscription, BackupPrivacyWake};

const SCHEMA_VERSION: i64 = 17;
const WORKSPACE_CAPABILITY_DOMAIN: &[u8] = b"gridtimer-workspace-capability-v1\0";
const DEFAULT_BUSY_TIMEOUT: Duration = Duration::from_secs(10);
pub const DEFAULT_LEGACY_TOKEN_TTL_MILLIS: i64 = 90 * 24 * 60 * 60 * 1_000;
pub const REQUEST_DEDUP_RETENTION_MILLIS: i64 = 7 * 24 * 60 * 60 * 1_000;
pub const REQUEST_DEDUP_MAX_PER_USER: i64 = 100;
pub const REQUEST_DEDUP_MAX_RESPONSE_BYTES: usize = 24 * 1024 * 1024;
pub const REQUEST_DEDUP_MAX_TOTAL_BYTES_PER_USER: i64 = 64 * 1024 * 1024;
pub const SNAPSHOT_HISTORY_WARNING_BYTES_PER_USER: i64 = 18 * 1024 * 1024;
pub const SNAPSHOT_HISTORY_CRITICAL_BYTES_PER_USER: i64 = 22 * 1024 * 1024;
pub const SNAPSHOT_HISTORY_HARD_LIMIT_BYTES_PER_USER: i64 = 24 * 1024 * 1024;
pub const ACCOUNT_APP_DATA_HARD_LIMIT_BYTES: i64 = 24 * 1024 * 1024;
const SNAPSHOT_HISTORY_ROW_OVERHEAD_BYTES: i64 = 256;
const SNAPSHOT_MEDIA_HISTORY_ROW_OVERHEAD_BYTES: i64 = 256;
pub const SNAPSHOT_WRITE_MIN_FREE_BYTES: u64 = 256 * 1024 * 1024;
const SNAPSHOT_WRITE_OVERHEAD_BYTES: u64 = 8 * 1024 * 1024;
const SCHEMA_MIGRATION_INDEX_ROW_ESTIMATE_BYTES: u64 = 768;
const MEDIA_TOMBSTONE_WRITE_ESTIMATE_BYTES: u64 = 4 * 1024;
const SNAPSHOT_CONTENT_COMPRESSION: &str = "zlib";
pub const MAX_MEDIA_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_MEDIA_ACCOUNT_BYTES: i64 = 512 * 1024 * 1024;
pub const MAX_MEDIA_ITEMS_PER_ACCOUNT: i64 = 2_000;
pub const MAX_MEDIA_ATTACHMENT_ID_BYTES: usize = 128;
pub const MAX_MEDIA_MIME_TYPE_BYTES: usize = 128;
pub const MAX_MEDIA_IDENTITIES_PER_ACCOUNT: i64 = 10_000;
pub const MAX_MEDIA_RETAINED_BYTES_GLOBAL: i64 = 512 * 1024 * 1024;
pub const MAX_MEDIA_IDENTITIES_GLOBAL: i64 = 50_000;
pub const MAX_REGISTERED_ACCOUNTS: i64 = 16;
pub const DELETED_MEDIA_CONTENT_RETENTION_MILLIS: i64 = 90 * 24 * 60 * 60 * 1_000;
const TOKEN_LAST_SEEN_WRITE_INTERVAL_MILLIS: i64 = 5 * 60 * 1_000;
const PENDING_TOKEN_TOMBSTONE_RETENTION_MILLIS: i64 = 30 * 24 * 60 * 60 * 1_000;
const INITIAL_REGISTRATION_ACTIVATION_LEASE_MILLIS: i64 = 10 * 60 * 1_000;
const INITIAL_REGISTRATION_RECOVERY_WINDOW_MILLIS: i64 = 48 * 60 * 60 * 1_000;
const ABANDONED_REGISTRATION_SAFETY_MARGIN_MILLIS: i64 = 10 * 60 * 1_000;
const LEGACY_BACKUP_MAGIC: &[u8; 8] = b"GTLGBK01";
const LEGACY_BACKUP_VERSION: u16 = 1;
const LEGACY_BACKUP_FLAG_DPAPI_CURRENT_USER: u16 = 1;
#[cfg(test)]
#[allow(dead_code)]
const LEGACY_BACKUP_FLAG_RESTRICTED_PLAINTEXT: u16 = 2;
const LEGACY_BACKUP_HEADER_BYTES: usize = 8 + 2 + 2 + 8 + 32 + 8;

#[derive(Debug)]
pub enum StoreError {
    Io(io::Error),
    Sqlite(rusqlite::Error),
    Json(serde_json::Error),
    Integrity(String),
    NotFound(String),
    MediaReferenceConflict {
        opaque: bool,
    },
    RevisionConflict {
        expected_revision: i64,
        actual_revision: i64,
    },
    RestoreGenerationConflict {
        expected_generation: i64,
        actual_generation: i64,
    },
    RestoreReceiptRequired {
        actual_generation: i64,
    },
    ServerGenerationRollback {
        client_generation: i64,
        server_generation: i64,
    },
    SnapshotHistoryQuotaExceeded {
        usage_bytes: i64,
        projected_bytes: i64,
        limit_bytes: i64,
    },
    AppDataQuotaExceeded {
        current_bytes: i64,
        projected_bytes: i64,
        limit_bytes: i64,
    },
    MediaServerRetainedQuotaExceeded {
        usage_bytes: i64,
        projected_bytes: i64,
        limit_bytes: i64,
    },
    MediaServerIdentityQuotaExceeded {
        usage_items: i64,
        projected_items: i64,
        limit_items: i64,
    },
    MediaAccountIdentityQuotaExceeded {
        usage_items: i64,
        projected_items: i64,
        limit_items: i64,
    },
    RegisteredAccountQuotaExceeded {
        current_accounts: i64,
        limit_accounts: i64,
    },
    DiskReserveExceeded {
        available_bytes: u64,
        required_bytes: u64,
    },
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::Sqlite(error) => write!(formatter, "SQLite error: {error}"),
            Self::Json(error) => write!(formatter, "JSON error: {error}"),
            Self::Integrity(message) => write!(formatter, "integrity check failed: {message}"),
            Self::NotFound(message) => write!(formatter, "not found: {message}"),
            Self::MediaReferenceConflict { opaque: true } =>
                formatter.write_str("encrypted attachment references are unresolved"),
            Self::MediaReferenceConflict { opaque: false } =>
                formatter.write_str("attachment is still referenced by a retained note"),
            Self::RevisionConflict {
                expected_revision,
                actual_revision,
            } => write!(
                formatter,
                "account revision conflict: expected {expected_revision}, actual {actual_revision}"
            ),
            Self::RestoreGenerationConflict {
                expected_generation,
                actual_generation,
            } => write!(
                formatter,
                "account restore generation conflict: expected {expected_generation}, actual {actual_generation}"
            ),
            Self::RestoreReceiptRequired { actual_generation } => write!(
                formatter,
                "account restore receipt is required for generation {actual_generation}"
            ),
            Self::ServerGenerationRollback {
                client_generation,
                server_generation,
            } => write!(
                formatter,
                "server restore generation rolled back: client {client_generation}, server {server_generation}"
            ),
            Self::SnapshotHistoryQuotaExceeded {
                usage_bytes,
                projected_bytes,
                limit_bytes,
            } => write!(
                formatter,
                "snapshot history quota exceeded: usage {usage_bytes}, projected {projected_bytes}, limit {limit_bytes}"
            ),
            Self::AppDataQuotaExceeded {
                current_bytes,
                projected_bytes,
                limit_bytes,
            } => write!(
                formatter,
                "account app-data quota exceeded: current {current_bytes}, projected {projected_bytes}, limit {limit_bytes}"
            ),
            Self::MediaServerRetainedQuotaExceeded {
                usage_bytes,
                projected_bytes,
                limit_bytes,
            } => write!(
                formatter,
                "media server retained quota exceeded: usage {usage_bytes}, projected {projected_bytes}, limit {limit_bytes}"
            ),
            Self::MediaServerIdentityQuotaExceeded {
                usage_items,
                projected_items,
                limit_items,
            } => write!(
                formatter,
                "media server identity quota exceeded: usage {usage_items}, projected {projected_items}, limit {limit_items}"
            ),
            Self::MediaAccountIdentityQuotaExceeded {
                usage_items,
                projected_items,
                limit_items,
            } => write!(
                formatter,
                "media account identity quota exceeded: usage {usage_items}, projected {projected_items}, limit {limit_items}"
            ),
            Self::RegisteredAccountQuotaExceeded {
                current_accounts,
                limit_accounts,
            } => write!(
                formatter,
                "registered account quota exceeded: current {current_accounts}, limit {limit_accounts}"
            ),
            Self::DiskReserveExceeded {
                available_bytes,
                required_bytes,
            } => write!(
                formatter,
                "storage disk reserve exceeded: available {available_bytes} bytes, required {required_bytes} bytes"
            ),
        }
    }
}

impl Error for StoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Sqlite(error) => Some(error),
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for StoreError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

pub type StoreResult<T> = Result<T, StoreError>;

#[derive(Clone, Debug)]
pub struct ServerStoreOpenOptions {
    pub database_path: PathBuf,
    pub legacy_json_path: Option<PathBuf>,
    pub now_epoch_millis: i64,
    pub legacy_token_ttl_millis: i64,
}

impl ServerStoreOpenOptions {
    pub fn new(database_path: impl Into<PathBuf>) -> Self {
        Self {
            database_path: database_path.into(),
            legacy_json_path: None,
            now_epoch_millis: system_time_epoch_millis(),
            legacy_token_ttl_millis: DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SqliteServerStore {
    database_path: PathBuf,
}

#[derive(Debug)]
pub struct OpenedServerStore {
    pub store: SqliteServerStore,
    pub legacy_migration: Option<LegacyMigrationReport>,
    pub pre_schema_migration_backup: Option<VerifiedBackupReport>,
    pub schema_migrated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LegacyMigrationReport {
    pub source_path: PathBuf,
    pub backup_path: PathBuf,
    pub content_sha256: String,
    pub users_imported: usize,
    pub tokens_imported: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LegacyBatchMigrationReport {
    pub files: Vec<LegacyMigrationReport>,
    pub users_inserted: usize,
    pub users_merged: usize,
    pub tokens_inserted: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotImportReport {
    pub snapshot: AccountSnapshot,
    pub changed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LegacySnapshotBinding {
    pub binding_key: String,
    pub user_id: String,
    pub content_sha256: String,
    pub source_path: String,
    pub bound_at_epoch_millis: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BoundSnapshotImportOutcome {
    Imported {
        binding: LegacySnapshotBinding,
        snapshot: SnapshotImportReport,
    },
    AlreadyBound(LegacySnapshotBinding),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyServerStore {
    pub users: Vec<LegacyServerUser>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyServerUser {
    pub id: String,
    pub email: String,
    pub password_salt: String,
    pub password_hash: String,
    pub created_at_epoch_millis: i64,
    pub updated_at_epoch_millis: i64,
    #[serde(default)]
    pub app_data_json: String,
    #[serde(default)]
    pub tokens: Vec<LegacyServerToken>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyServerToken {
    pub token: String,
    #[serde(default)]
    pub device_name: String,
    pub created_at_epoch_millis: i64,
    pub last_seen_at_epoch_millis: i64,
}

#[derive(Clone, Debug)]
pub struct NewStoredUser {
    pub id: String,
    pub email: String,
    pub password_salt: String,
    pub password_hash: String,
    pub password_scheme: String,
    pub created_at_epoch_millis: i64,
    pub updated_at_epoch_millis: i64,
    pub app_data_json: String,
    pub account_revision: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountSnapshot {
    pub user_id: String,
    pub app_data_json: String,
    pub revision: i64,
    pub updated_at_epoch_millis: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountSnapshotHistory {
    pub user_id: String,
    pub revision: i64,
    pub app_data_json: String,
    pub created_at_epoch_millis: i64,
    pub sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredUser {
    pub id: String,
    pub email: String,
    pub password_salt: String,
    pub password_hash: String,
    pub password_scheme: String,
    pub created_at_epoch_millis: i64,
    pub updated_at_epoch_millis: i64,
    pub account: AccountSnapshot,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenMetadata {
    pub id: i64,
    pub user_id: String,
    pub token_id: String,
    pub token_hash: String,
    pub device_name: String,
    pub created_at_epoch_millis: i64,
    pub last_seen_at_epoch_millis: i64,
    pub expires_at_epoch_millis: i64,
    pub revoked_at_epoch_millis: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthenticatedToken {
    pub token_id: i64,
    pub token_identifier: String,
    pub user_id: String,
    pub device_name: String,
    pub expires_at_epoch_millis: i64,
    pub last_seen_restore_generation: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingTokenRecoveryCandidate {
    pub token: AuthenticatedToken,
    pub original_expires_at_epoch_millis: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RestoreBarrierState {
    pub current_generation: i64,
    pub token_last_seen_generation: i64,
    pub token_restore_acknowledged: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestoreReceipt {
    pub current_generation: i64,
    pub receipt: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerAccountIdentity {
    pub server_instance_id: String,
    pub account_namespace: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SnapshotStorageStats {
    pub history_rows: i64,
    pub content_objects: i64,
    pub uncompressed_bytes: i64,
    pub compressed_bytes: i64,
    pub prune_audit_rows: i64,
    pub charged_bytes: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotHistoryWarning {
    None,
    ApproachingLimit,
    Critical,
    OverLimit,
}

impl SnapshotHistoryWarning {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "",
            Self::ApproachingLimit => "approaching_limit",
            Self::Critical => "critical",
            Self::OverLimit => "over_limit",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SnapshotHistoryQuotaStatus {
    pub usage_bytes: i64,
    pub projected_bytes: i64,
    pub limit_bytes: i64,
    pub warning: SnapshotHistoryWarning,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TokenAuthentication {
    Active(AuthenticatedToken),
    PendingActivation(AuthenticatedToken),
    Unknown,
    Expired,
    Revoked,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiscoveryTokenKeyLookup {
    Available {
        key: [u8; 32],
        recovered_pending_lease: bool,
    },
    RecoveredActiveLease {
        key: [u8; 32],
    },
    PendingReauthenticationRequired,
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PendingTokenActivation {
    pub token_id: i64,
    pub activated_at_epoch_millis: i64,
    pub active_expires_at_epoch_millis: i64,
    pub cleanup_recovery_original_expiry_epoch_millis: Option<i64>,
    pub cleanup_recovery_original_lease_millis: i64,
    pub cleanup_recovery_window_millis: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyncRequestReceipt {
    pub response_json: String,
    pub account_revision: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SyncRequestOutcome {
    Applied(SyncRequestReceipt),
    Replayed(SyncRequestReceipt),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StoreStats {
    pub users: i64,
    pub tokens: i64,
    pub snapshots: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaMetadata {
    pub user_id: String,
    pub attachment_id: String,
    pub sha256: String,
    pub mime_type: String,
    pub size_bytes: i64,
    pub updated_at_epoch_millis: i64,
    pub deleted_at_epoch_millis: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredMedia {
    pub metadata: MediaMetadata,
    pub content: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MediaUpsertOutcome {
    Stored(MediaMetadata),
    RejectedByTombstone(MediaMetadata),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedBackupReport {
    pub destination: PathBuf,
    pub size_bytes: u64,
    pub sha256: String,
    pub created_at_epoch_millis: i64,
    pub server_instance_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct LegacySnapshotRepairSource {
    user_id: String,
    revision: i64,
    content_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LegacySnapshotRepairBackupIdentity {
    file_name: String,
    size_bytes: i64,
    sha256: String,
    schema_version: i64,
    created_at_epoch_millis: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct VerifiedPreSchemaSnapshotManifest {
    backup: LegacySnapshotRepairBackupIdentity,
    sources: Vec<LegacySnapshotRepairSource>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LegacySnapshotRepairAllowanceSeed {
    source: LegacySnapshotRepairSource,
    seeded_at_epoch_millis: i64,
}

impl SqliteServerStore {
    pub fn open(
        database_path: impl Into<PathBuf>,
        legacy_json_path: Option<PathBuf>,
    ) -> StoreResult<Self> {
        let mut options = ServerStoreOpenOptions::new(database_path);
        options.legacy_json_path = legacy_json_path;
        Ok(Self::open_with_options(options)?.store)
    }

    pub fn open_from_legacy_path(legacy_json_path: impl Into<PathBuf>) -> StoreResult<Self> {
        let legacy_json_path = legacy_json_path.into();
        let database_path = legacy_json_path.with_extension("sqlite3");
        Self::open(database_path, Some(legacy_json_path))
    }

    pub fn open_with_options(options: ServerStoreOpenOptions) -> StoreResult<OpenedServerStore> {
        if options.legacy_token_ttl_millis <= 0 {
            return Err(StoreError::Integrity(
                "legacy token TTL must be positive".to_string(),
            ));
        }
        if let Some(parent) = options.database_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let database_existed = options.database_path.exists();
        if database_existed && options.database_path.metadata()?.len() == 0 {
            return Err(StoreError::Integrity(format!(
                "existing database is empty: {}",
                options.database_path.display()
            )));
        }

        let store = Self {
            database_path: options.database_path,
        };
        let mut connection = store.open_connection(true)?;
        verify_quick_check(&connection)?;
        verify_integrity_check(&connection)?;
        verify_foreign_keys(&connection)?;
        let previous_schema_version = current_schema_version(&connection)?;
        if previous_schema_version > SCHEMA_VERSION {
            return Err(StoreError::Integrity(format!(
                "database schema version {previous_schema_version} is newer than supported {SCHEMA_VERSION}"
            )));
        }
        let pre_schema_migration_backup =
            if database_existed && previous_schema_version < SCHEMA_VERSION {
                ensure_schema_migration_capacity(&connection)?;
                let destination = unique_pre_schema_backup_path(
                    &store.database_path,
                    previous_schema_version,
                    SCHEMA_VERSION,
                    options.now_epoch_millis,
                )?;
                Some(create_verified_sqlite_backup(
                    &connection,
                    &store.database_path,
                    &destination,
                    options.now_epoch_millis,
                    Some(previous_schema_version),
                    Some((previous_schema_version, SCHEMA_VERSION)),
                )?)
            } else {
                None
            };
        let legacy_snapshot_repair_manifest = if matches!(previous_schema_version, 12 | 13) {
            let backup = pre_schema_migration_backup.as_ref().ok_or_else(|| {
                StoreError::Integrity(
                    "schema v12 or v13 migration requires a verified pre-schema backup".to_string(),
                )
            })?;
            Some(read_verified_pre_schema_snapshot_manifest(
                backup,
                previous_schema_version,
            )?)
        } else {
            None
        };
        let schema_migrated = apply_schema_migrations(
            &mut connection,
            options.now_epoch_millis,
            legacy_snapshot_repair_manifest.as_ref(),
        )?;
        verify_required_schema(&connection)?;
        privacy_journal::reconcile(&mut connection)?;
        note_privacy::repair_all(&mut connection)?;
        verify_semantic_storage_integrity(&connection)?;
        verify_foreign_keys(&connection)?;
        let legacy_migration = match options.legacy_json_path.as_deref() {
            Some(path) if path.exists() => migrate_legacy_store(
                &mut connection,
                path,
                options.now_epoch_millis,
                options.legacy_token_ttl_millis,
            )?,
            _ => None,
        };
        cleanup_expired_pending_tokens_in_connection(&mut connection, options.now_epoch_millis)?;
        note_privacy::repair_all(&mut connection)?;
        verify_quick_check(&connection)?;
        verify_integrity_check(&connection)?;
        verify_semantic_storage_integrity(&connection)?;
        verify_foreign_keys(&connection)?;
        drop(connection);
        Ok(OpenedServerStore {
            store,
            legacy_migration,
            pre_schema_migration_backup,
            schema_migrated,
        })
    }

    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    pub fn validate_integrity(&self) -> StoreResult<()> {
        let connection = self.open_connection(false)?;
        verify_quick_check(&connection)?;
        verify_integrity_check(&connection)?;
        verify_required_schema(&connection)?;
        verify_semantic_storage_integrity(&connection)?;
        verify_foreign_keys(&connection)
    }

    pub fn stable_database_sha256(&self) -> StoreResult<String> {
        let connection = self.open_connection(false)?;
        connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        verify_quick_check(&connection)?;
        verify_integrity_check(&connection)?;
        drop(connection);
        sha256_file(&self.database_path)
    }

    pub fn server_account_identity(&self, user_id: &str) -> StoreResult<ServerAccountIdentity> {
        let connection = self.open_connection(false)?;
        let identity = connection
            .query_row(
                "SELECT s.server_instance_id, a.account_namespace
                 FROM server_identity s
                 JOIN account_namespaces a ON a.user_id = ?1
                 WHERE s.singleton = 1",
                params![user_id],
                |row| {
                    Ok(ServerAccountIdentity {
                        server_instance_id: row.get(0)?,
                        account_namespace: row.get(1)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("account namespace for user {user_id}")))?;
        if !valid_sha256_hex(&identity.server_instance_id)
            || identity.server_instance_id != identity.server_instance_id.to_ascii_lowercase()
            || identity.account_namespace
                != account_namespace_identifier(&identity.server_instance_id, user_id)
        {
            return Err(StoreError::Integrity(format!(
                "account namespace integrity check failed for user {user_id}"
            )));
        }
        Ok(identity)
    }

    pub fn server_instance_id(&self) -> StoreResult<String> {
        let connection = self.open_connection(false)?;
        let server_instance_id = connection
            .query_row(
                "SELECT server_instance_id FROM server_identity WHERE singleton = 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::Integrity("server identity is missing".to_string()))?;
        if !valid_sha256_hex(&server_instance_id)
            || server_instance_id != server_instance_id.to_ascii_lowercase()
        {
            return Err(StoreError::Integrity(
                "server instance identifier is malformed".to_string(),
            ));
        }
        Ok(server_instance_id)
    }

    pub fn workspace_capability_proof(
        &self,
        user_id: &str,
        workspace_id: &str,
        generation: i64,
    ) -> StoreResult<String> {
        if !valid_lowercase_opaque_identifier(workspace_id) {
            return Err(StoreError::Integrity(
                "workspace identifier must be 64 lowercase hexadecimal characters".to_string(),
            ));
        }
        if generation < 0 {
            return Err(StoreError::Integrity(
                "workspace capability generation cannot be negative".to_string(),
            ));
        }
        let connection = self.open_connection(false)?;
        let (server_instance_id, capability_secret, account_namespace, current_generation) =
            read_workspace_capability_material(&connection, user_id)?;
        if generation > current_generation {
            return Err(StoreError::ServerGenerationRollback {
                client_generation: generation,
                server_generation: current_generation,
            });
        }
        if generation < current_generation {
            return Err(StoreError::RestoreGenerationConflict {
                expected_generation: generation,
                actual_generation: current_generation,
            });
        }
        workspace_capability_hmac(
            &capability_secret,
            &server_instance_id,
            user_id,
            &account_namespace,
            workspace_id,
            generation,
        )
    }

    pub fn verify_workspace_capability(
        &self,
        user_id: &str,
        workspace_id: &str,
        generation: i64,
        proof: &str,
    ) -> StoreResult<bool> {
        if !valid_lowercase_opaque_identifier(workspace_id)
            || !valid_lowercase_opaque_identifier(proof)
        {
            return Ok(false);
        }
        let expected = self.workspace_capability_proof(user_id, workspace_id, generation)?;
        Ok(constant_time_bytes_eq(
            expected.as_bytes(),
            proof.as_bytes(),
        ))
    }

    /// Finds legacy store generations without touching them. Migration-created
    /// backups are excluded so repeated startup cannot recursively re-import
    /// its own safety copies.
    pub fn discover_legacy_json_files(directory: &Path) -> StoreResult<Vec<PathBuf>> {
        let mut paths = Vec::new();
        if !directory.exists() {
            return Ok(paths);
        }
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let lower = name.to_ascii_lowercase();
            if lower.starts_with("server_store")
                && lower.ends_with(".json")
                && !lower.contains("_pre_sqlite_migration_")
                && !lower.starts_with("server_store_startup_state_")
            {
                paths.push(entry.path());
            }
        }
        paths.sort_by(|left, right| {
            left.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .cmp(&right.file_name().unwrap_or_default().to_string_lossy())
        });
        Ok(paths)
    }

    /// Imports all supplied legacy generations in one SQLite transaction. Each
    /// source is validated and copied byte-for-byte before the transaction.
    /// Users match by id first and normalized email second. The caller owns the
    /// business-level snapshot merge so no history is truncated here.
    pub fn import_legacy_files_with_merge<F>(
        &self,
        paths: &[PathBuf],
        now_epoch_millis: i64,
        token_ttl_millis: i64,
        mut merge_snapshot: F,
    ) -> StoreResult<LegacyBatchMigrationReport>
    where
        F: FnMut(&str, &str, i64) -> StoreResult<String>,
    {
        self.import_legacy_files_with_revision_merge(
            paths,
            now_epoch_millis,
            token_ttl_millis,
            |existing, _existing_updated_at, incoming, _incoming_updated_at, merge_now| {
                merge_snapshot_values(existing, incoming, merge_now, &mut merge_snapshot)
            },
        )
    }

    /// Revision-aware variant used when legacy snapshots predate explicit
    /// scalar revision fields. The incoming revision prefers the user's own
    /// updated timestamp and falls back to the source file modification time.
    pub fn import_legacy_files_with_revision_merge<F>(
        &self,
        paths: &[PathBuf],
        now_epoch_millis: i64,
        token_ttl_millis: i64,
        mut merge_snapshot: F,
    ) -> StoreResult<LegacyBatchMigrationReport>
    where
        F: FnMut(&str, i64, &str, i64, i64) -> StoreResult<String>,
    {
        if token_ttl_millis <= 0 {
            return Err(StoreError::Integrity(
                "legacy token TTL must be positive".to_string(),
            ));
        }
        let mut connection = self.open_connection(false)?;
        let mut content_seen = HashSet::new();
        let mut pending = Vec::new();
        for path in paths {
            let raw = fs::read(path)?;
            let content_sha256 = sha256_hex(&raw);
            if !content_seen.insert(content_sha256.clone()) {
                continue;
            }
            let already_imported = connection
                .query_row(
                    "SELECT 1 FROM legacy_imports WHERE content_sha256 = ?1",
                    params![content_sha256],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if already_imported {
                continue;
            }
            let legacy: LegacyServerStore = serde_json::from_slice(&raw)?;
            validate_legacy_store(&legacy)?;
            let source_updated_at_epoch_millis = fs::metadata(path)?
                .modified()?
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .min(i64::MAX as u128) as i64;
            pending.push(PendingLegacyFile {
                path: path.clone(),
                raw,
                content_sha256,
                legacy,
                source_updated_at_epoch_millis,
                backup_path: None,
            });
        }
        for file in &mut pending {
            file.backup_path = Some(write_timestamped_backup(
                &file.path,
                &file.raw,
                now_epoch_millis,
            )?);
        }

        let mut report = LegacyBatchMigrationReport::default();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for file in &pending {
            let backup_path = file.backup_path.as_ref().ok_or_else(|| {
                StoreError::Integrity("legacy backup was not created".to_string())
            })?;
            let mut file_tokens_inserted = 0_usize;
            for incoming in &file.legacy.users {
                let incoming_snapshot_updated_at = if incoming.updated_at_epoch_millis > 0 {
                    incoming.updated_at_epoch_millis
                } else {
                    file.source_updated_at_epoch_millis
                };
                let matched = find_user_for_legacy(&transaction, incoming)?;
                let canonical_user_id = if let Some(existing) = matched {
                    let merged_json = merge_snapshot(
                        &existing.account.app_data_json,
                        existing.account.updated_at_epoch_millis,
                        &incoming.app_data_json,
                        incoming_snapshot_updated_at,
                        now_epoch_millis,
                    )?;
                    validate_app_data_json(&merged_json)?;
                    let snapshot_changed = merged_json != existing.account.app_data_json;
                    if snapshot_changed {
                        let revision = next_account_revision(existing.account.revision)?;
                        insert_snapshot_history_with_limit(
                            &transaction,
                            &existing.account,
                            now_epoch_millis,
                            i64::MAX,
                            NewUserMediaPolicy::GrandfatherVerifiedLegacyImport,
                        )?;
                        ensure_snapshot_write_capacity(&transaction, merged_json.len() as u64)?;
                        let content_sha256 = sha256_hex(merged_json.as_bytes());
                        let updated_at_epoch_millis = existing
                            .account
                            .updated_at_epoch_millis
                            .max(incoming_snapshot_updated_at);
                        let restore_generation =
                            restore_generation_in_transaction(&transaction, &existing.id)?;
                        let envelope_sha256 = account_snapshot_envelope_sha256(
                            &existing.id,
                            &merged_json,
                            revision,
                            updated_at_epoch_millis,
                            restore_generation,
                        );
                        let changed = transaction.execute(
                            "UPDATE account_snapshots \
                             SET app_data_json = ?1, revision = ?2, \
                                 updated_at_epoch_millis = ?3, content_sha256 = ?4, \
                                 envelope_sha256 = ?5 \
                             WHERE user_id = ?6",
                            params![
                                merged_json,
                                revision,
                                updated_at_epoch_millis,
                                content_sha256,
                                envelope_sha256,
                                existing.id
                            ],
                        )?;
                        if changed != 1 {
                            return Err(StoreError::Integrity(
                                "legacy merge changed an unexpected snapshot row count".to_string(),
                            ));
                        }
                        replace_current_snapshot_media_identities_tolerant(
                            &transaction,
                            &existing.id,
                            &merged_json,
                        )?;
                        note_privacy::enforce(&transaction, &existing.id)?;
                    }
                    if incoming.updated_at_epoch_millis > existing.updated_at_epoch_millis {
                        transaction.execute(
                            "UPDATE users SET email = ?1, password_salt = ?2, password_hash = ?3, \
                                              password_scheme = 'legacy_sha256', \
                                              updated_at_epoch_millis = ?4 \
                             WHERE id = ?5",
                            params![
                                incoming.email.trim(),
                                incoming.password_salt,
                                incoming.password_hash,
                                incoming.updated_at_epoch_millis,
                                existing.id
                            ],
                        )?;
                    }
                    report.users_merged += 1;
                    existing.id
                } else {
                    let prepared_json = merge_snapshot(
                        "",
                        0,
                        &incoming.app_data_json,
                        incoming_snapshot_updated_at,
                        now_epoch_millis,
                    )?;
                    validate_app_data_json(&prepared_json)?;
                    let revision = i64::from(!prepared_json.trim().is_empty());
                    insert_new_user(
                        &transaction,
                        &NewStoredUser {
                            id: incoming.id.clone(),
                            email: incoming.email.clone(),
                            password_salt: incoming.password_salt.clone(),
                            password_hash: incoming.password_hash.clone(),
                            password_scheme: "legacy_sha256".to_string(),
                            created_at_epoch_millis: incoming.created_at_epoch_millis,
                            updated_at_epoch_millis: incoming_snapshot_updated_at,
                            app_data_json: prepared_json,
                            account_revision: revision,
                        },
                        NewUserMediaPolicy::GrandfatherVerifiedLegacyImport,
                    )?;
                    report.users_inserted += 1;
                    incoming.id.clone()
                };

                for token in &incoming.tokens {
                    if insert_legacy_token(
                        &transaction,
                        &canonical_user_id,
                        token,
                        now_epoch_millis,
                        token_ttl_millis,
                    )? {
                        report.tokens_inserted += 1;
                        file_tokens_inserted += 1;
                    }
                }
            }
            transaction.execute(
                "INSERT INTO legacy_imports(
                     content_sha256, source_path, backup_path, imported_at_epoch_millis,
                     users_imported, tokens_imported
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    file.content_sha256,
                    file.path.to_string_lossy(),
                    backup_path.to_string_lossy(),
                    now_epoch_millis,
                    file.legacy.users.len() as i64,
                    file_tokens_inserted as i64,
                ],
            )?;
            report.files.push(LegacyMigrationReport {
                source_path: file.path.clone(),
                backup_path: backup_path.clone(),
                content_sha256: file.content_sha256.clone(),
                users_imported: file.legacy.users.len(),
                tokens_imported: file_tokens_inserted,
            });
        }
        privacy_journal::commit(transaction)?;
        verify_integrity_check(&connection)?;
        verify_foreign_keys(&connection)?;
        Ok(report)
    }

    /// Merges an additional local snapshot, such as the Windows client state,
    /// into one account without bypassing the monotonic revision counter.
    pub fn import_account_snapshot_with_merge<F>(
        &self,
        user_id: &str,
        incoming_app_data_json: &str,
        source_updated_at_epoch_millis: i64,
        now_epoch_millis: i64,
        mut merge_snapshot: F,
    ) -> StoreResult<SnapshotImportReport>
    where
        F: FnMut(&str, &str, i64) -> StoreResult<String>,
    {
        self.import_account_snapshot_with_revision_merge(
            user_id,
            incoming_app_data_json,
            source_updated_at_epoch_millis,
            now_epoch_millis,
            |existing, _existing_updated_at, incoming, _incoming_updated_at, merge_now| {
                merge_snapshot_values(existing, incoming, merge_now, &mut merge_snapshot)
            },
        )
    }

    /// Revision-aware local snapshot import. Unlike online sync, this exposes
    /// source timestamps solely so callers can repair missing legacy scalar
    /// revision fields before applying the normal structural merge.
    pub fn import_account_snapshot_with_revision_merge<F>(
        &self,
        user_id: &str,
        incoming_app_data_json: &str,
        source_updated_at_epoch_millis: i64,
        now_epoch_millis: i64,
        mut merge_snapshot: F,
    ) -> StoreResult<SnapshotImportReport>
    where
        F: FnMut(&str, i64, &str, i64, i64) -> StoreResult<String>,
    {
        validate_app_data_json(incoming_app_data_json)?;
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let report = import_account_snapshot_in_transaction(
            &transaction,
            user_id,
            incoming_app_data_json,
            source_updated_at_epoch_millis,
            now_epoch_millis,
            &mut merge_snapshot,
        )?;
        privacy_journal::commit(transaction)?;
        Ok(report)
    }

    /// Atomically binds a one-time legacy snapshot to its original account and
    /// imports it. Once the binding exists, the same global source is never
    /// imported into any account again, even if the active desktop login changes.
    #[allow(clippy::too_many_arguments)]
    pub fn import_bound_account_snapshot_with_revision_merge<F>(
        &self,
        binding_key: &str,
        user_id: &str,
        content_sha256: &str,
        source_path: &str,
        incoming_app_data_json: &str,
        source_updated_at_epoch_millis: i64,
        now_epoch_millis: i64,
        mut merge_snapshot: F,
    ) -> StoreResult<BoundSnapshotImportOutcome>
    where
        F: FnMut(&str, i64, &str, i64, i64) -> StoreResult<String>,
    {
        let binding_key = binding_key.trim();
        if binding_key.is_empty()
            || binding_key.len() > 128
            || binding_key.chars().any(char::is_control)
        {
            return Err(StoreError::Integrity(
                "legacy snapshot binding key is invalid".to_string(),
            ));
        }
        decode_sha256_hex(content_sha256.trim())?;
        if source_path.trim().is_empty() {
            return Err(StoreError::Integrity(
                "legacy snapshot source path cannot be empty".to_string(),
            ));
        }
        validate_app_data_json(incoming_app_data_json)?;
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = transaction
            .query_row(
                "SELECT binding_key, user_id, content_sha256, source_path, bound_at_epoch_millis \
                 FROM legacy_snapshot_bindings WHERE binding_key = ?1",
                params![binding_key],
                legacy_snapshot_binding_from_row,
            )
            .optional()?;
        if let Some(binding) = existing {
            privacy_journal::commit(transaction)?;
            return Ok(BoundSnapshotImportOutcome::AlreadyBound(binding));
        }

        let snapshot = import_account_snapshot_in_transaction(
            &transaction,
            user_id,
            incoming_app_data_json,
            source_updated_at_epoch_millis,
            now_epoch_millis,
            &mut merge_snapshot,
        )?;
        let binding = LegacySnapshotBinding {
            binding_key: binding_key.to_string(),
            user_id: user_id.to_string(),
            content_sha256: content_sha256.trim().to_ascii_lowercase(),
            source_path: source_path.trim().to_string(),
            bound_at_epoch_millis: now_epoch_millis,
        };
        transaction.execute(
            "INSERT INTO legacy_snapshot_bindings(
                 binding_key, user_id, content_sha256, source_path, bound_at_epoch_millis
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                binding.binding_key,
                binding.user_id,
                binding.content_sha256,
                binding.source_path,
                binding.bound_at_epoch_millis,
            ],
        )?;
        privacy_journal::commit(transaction)?;
        Ok(BoundSnapshotImportOutcome::Imported { binding, snapshot })
    }

    pub fn create_user(&self, user: NewStoredUser) -> StoreResult<StoredUser> {
        self.create_users_atomically(std::slice::from_ref(&user))?;
        self.find_user_by_id(&user.id)?
            .ok_or_else(|| StoreError::Integrity("created user could not be read back".to_string()))
    }

    pub fn create_users_atomically(&self, users: &[NewStoredUser]) -> StoreResult<()> {
        for user in users {
            validate_new_user(user)?;
        }
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for user in users {
            insert_new_user(&transaction, user, NewUserMediaPolicy::EnforceGrowth)?;
        }
        privacy_journal::commit(transaction)?;
        Ok(())
    }

    pub fn find_user_by_id(&self, user_id: &str) -> StoreResult<Option<StoredUser>> {
        self.query_user("u.id = ?1", user_id)
    }

    pub fn find_user_by_email(&self, email: &str) -> StoreResult<Option<StoredUser>> {
        self.query_user("u.email = ?1 COLLATE NOCASE", email.trim())
    }

    fn query_user(&self, predicate: &str, value: &str) -> StoreResult<Option<StoredUser>> {
        let connection = self.open_connection(false)?;
        let sql = format!(
            "SELECT u.id, u.email, u.password_salt, u.password_hash, \
                    u.password_scheme, u.created_at_epoch_millis, u.updated_at_epoch_millis, \
                    s.app_data_json, s.revision, s.updated_at_epoch_millis, s.content_sha256, \
                    s.restore_generation, s.envelope_sha256 \
             FROM users u JOIN account_snapshots s ON s.user_id = u.id WHERE {predicate}"
        );
        let stored = connection
            .query_row(&sql, params![value], |row| {
                Ok((
                    stored_user_from_row(row)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, i64>(11)?,
                    row.get::<_, String>(12)?,
                ))
            })
            .optional()?;
        match stored {
            Some((user, content_sha256, restore_generation, envelope_sha256)) => {
                verify_current_account_snapshot(
                    &user.account,
                    restore_generation,
                    &content_sha256,
                    &envelope_sha256,
                )?;
                Ok(Some(user))
            }
            None => Ok(None),
        }
    }

    pub fn read_account(&self, user_id: &str) -> StoreResult<AccountSnapshot> {
        let connection = self.open_connection(false)?;
        let stored = connection
            .query_row(
                "SELECT user_id, app_data_json, revision, updated_at_epoch_millis, \
                        restore_generation, content_sha256, envelope_sha256 \
                 FROM account_snapshots WHERE user_id = ?1",
                params![user_id],
                |row| {
                    Ok((
                        AccountSnapshot {
                            user_id: row.get(0)?,
                            app_data_json: row.get(1)?,
                            revision: row.get(2)?,
                            updated_at_epoch_millis: row.get(3)?,
                        },
                        row.get::<_, i64>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("account for user {user_id}")))?;
        verify_current_account_snapshot(&stored.0, stored.1, &stored.2, &stored.3)?;
        Ok(stored.0)
    }

    pub fn list_account_snapshots(&self) -> StoreResult<Vec<AccountSnapshot>> {
        let connection = self.open_connection(false)?;
        let mut statement = connection.prepare(
            "SELECT user_id, app_data_json, revision, updated_at_epoch_millis, \
                    restore_generation, content_sha256, envelope_sha256 \
             FROM account_snapshots ORDER BY user_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                AccountSnapshot {
                    user_id: row.get(0)?,
                    app_data_json: row.get(1)?,
                    revision: row.get(2)?,
                    updated_at_epoch_millis: row.get(3)?,
                },
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })?;
        let stored = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)?;
        let mut snapshots = Vec::with_capacity(stored.len());
        for (snapshot, restore_generation, content_sha256, envelope_sha256) in stored {
            verify_current_account_snapshot(
                &snapshot,
                restore_generation,
                &content_sha256,
                &envelope_sha256,
            )?;
            snapshots.push(snapshot);
        }
        Ok(snapshots)
    }

    pub fn list_snapshot_history(
        &self,
        user_id: &str,
        limit: usize,
    ) -> StoreResult<Vec<AccountSnapshotHistory>> {
        let limit = limit.clamp(1, 1_000) as i64;
        let connection = self.open_connection(false)?;
        let mut statement = connection.prepare(
            "SELECT h.user_id, h.revision, h.created_at_epoch_millis,
                    c.sha256, c.compression, c.uncompressed_size_bytes,
                    c.compressed_size_bytes, c.content
             FROM account_snapshot_history h
             JOIN snapshot_contents c ON c.sha256 = h.content_sha256
             WHERE h.user_id = ?1
             ORDER BY h.revision DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![user_id, limit], snapshot_content_row)?;
        let stored = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)?;
        stored.into_iter().map(decode_snapshot_history).collect()
    }

    pub fn snapshot_history_count(&self, user_id: &str) -> StoreResult<i64> {
        let connection = self.open_connection(false)?;
        connection
            .query_row(
                "SELECT COUNT(*) FROM account_snapshot_history WHERE user_id = ?1",
                params![user_id],
                |row| row.get(0),
            )
            .map_err(StoreError::from)
    }

    pub fn snapshot_storage_stats(&self, user_id: &str) -> StoreResult<SnapshotStorageStats> {
        let connection = self.open_connection(false)?;
        let history_rows = connection.query_row(
            "SELECT COUNT(*) FROM account_snapshot_history WHERE user_id = ?1",
            params![user_id],
            |row| row.get::<_, i64>(0),
        )?;
        let (content_objects, uncompressed_bytes, compressed_bytes) = connection.query_row(
            "SELECT COUNT(*), COALESCE(SUM(c.uncompressed_size_bytes), 0),
                    COALESCE(SUM(c.compressed_size_bytes), 0)
             FROM snapshot_contents c
             WHERE c.sha256 IN (
                 SELECT DISTINCT content_sha256
                 FROM account_snapshot_history
                 WHERE user_id = ?1
             )",
            params![user_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )?;
        let prune_audit_rows = connection.query_row(
            "SELECT COUNT(*) FROM snapshot_history_prune_audit WHERE user_id = ?1",
            params![user_id],
            |row| row.get(0),
        )?;
        let charged_bytes = snapshot_history_charged_bytes(&connection, user_id)?;
        Ok(SnapshotStorageStats {
            history_rows,
            content_objects,
            uncompressed_bytes,
            compressed_bytes,
            prune_audit_rows,
            charged_bytes,
        })
    }

    pub fn snapshot_history_quota_status(
        &self,
        user_id: &str,
    ) -> StoreResult<SnapshotHistoryQuotaStatus> {
        let connection = self.open_connection(false)?;
        let exists = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM users WHERE id = ?1)",
            params![user_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !exists {
            return Err(StoreError::NotFound(format!("user {user_id}")));
        }
        let usage_bytes = snapshot_history_charged_bytes(&connection, user_id)?;
        Ok(SnapshotHistoryQuotaStatus {
            usage_bytes,
            projected_bytes: usage_bytes,
            limit_bytes: SNAPSHOT_HISTORY_HARD_LIMIT_BYTES_PER_USER,
            warning: snapshot_history_warning(usage_bytes),
        })
    }

    pub fn snapshot_history_quota_after_archiving(
        &self,
        snapshot: &AccountSnapshot,
    ) -> StoreResult<SnapshotHistoryQuotaStatus> {
        let connection = self.open_connection(false)?;
        let (usage_bytes, projected_bytes) = snapshot_history_projection(&connection, snapshot)?;
        Ok(SnapshotHistoryQuotaStatus {
            usage_bytes,
            projected_bytes,
            limit_bytes: SNAPSHOT_HISTORY_HARD_LIMIT_BYTES_PER_USER,
            warning: snapshot_history_warning(projected_bytes),
        })
    }

    pub fn verify_snapshot_storage_integrity(&self) -> StoreResult<()> {
        let connection = self.open_connection(false)?;
        verify_semantic_storage_integrity(&connection)
    }

    pub fn restore_barrier_state(
        &self,
        user_id: &str,
        token_id: i64,
    ) -> StoreResult<RestoreBarrierState> {
        let connection = self.open_connection(false)?;
        read_restore_barrier(&connection, user_id, token_id)
    }

    pub fn acknowledge_restore_generation(
        &self,
        user_id: &str,
        token_id: i64,
        acknowledged_generation: i64,
        restore_receipt: &str,
    ) -> StoreResult<RestoreBarrierState> {
        if acknowledged_generation < 0 {
            return Err(StoreError::Integrity(
                "acknowledged restore generation cannot be negative".to_string(),
            ));
        }
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        acknowledge_restore_barrier_in_transaction(
            &transaction,
            user_id,
            Some((token_id, acknowledged_generation, restore_receipt, false)),
        )?;
        privacy_journal::commit(transaction)?;
        Ok(RestoreBarrierState {
            current_generation: acknowledged_generation,
            token_last_seen_generation: acknowledged_generation,
            token_restore_acknowledged: true,
        })
    }

    pub fn ensure_restore_receipt(
        &self,
        user_id: &str,
        token_id: i64,
    ) -> StoreResult<RestoreReceipt> {
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let receipt = ensure_restore_receipt_in_transaction(&transaction, user_id, token_id)?;
        privacy_journal::commit(transaction)?;
        Ok(receipt)
    }

    pub fn restored_account_with_receipt(
        &self,
        user_id: &str,
        token_id: i64,
    ) -> StoreResult<(AccountSnapshot, RestoreReceipt)> {
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let account = read_account_in_transaction(&transaction, user_id)?
            .ok_or_else(|| StoreError::NotFound(format!("account for user {user_id}")))?;
        validate_app_data_json(&account.app_data_json)?;
        let receipt = ensure_restore_receipt_in_transaction(&transaction, user_id, token_id)?;
        privacy_journal::commit(transaction)?;
        Ok((account, receipt))
    }

    pub fn restore_account_snapshot(
        &self,
        user_id: &str,
        history_revision: i64,
        expected_current_revision: i64,
        now_epoch_millis: i64,
    ) -> StoreResult<AccountSnapshot> {
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = read_account_in_transaction(&transaction, user_id)?
            .ok_or_else(|| StoreError::NotFound(format!("account for user {user_id}")))?;
        validate_app_data_json(&current.app_data_json)?;
        if current.revision != expected_current_revision {
            return Err(StoreError::RevisionConflict {
                expected_revision: expected_current_revision,
                actual_revision: current.revision,
            });
        }
        let stored_historical = transaction
            .query_row(
                "SELECT h.user_id, h.revision, h.created_at_epoch_millis,
                        c.sha256, c.compression, c.uncompressed_size_bytes,
                        c.compressed_size_bytes, c.content
                 FROM account_snapshot_history h
                 JOIN snapshot_contents c ON c.sha256 = h.content_sha256
                 WHERE h.user_id = ?1 AND h.revision = ?2",
                params![user_id, history_revision],
                snapshot_content_row,
            )
            .optional()?
            .ok_or_else(|| {
                StoreError::NotFound(format!(
                    "snapshot revision {history_revision} for user {user_id}"
                ))
            })?;
        let historical = decode_snapshot_history(stored_historical)?;
        verify_snapshot_history_entry(&historical)?;
        let historical_media = load_complete_media_snapshot(
            &transaction,
            user_id,
            history_revision,
            &historical.app_data_json,
        )?;
        // Historical bytes remain recoverable, but restoring a head cannot
        // erase durable per-note attachment detachments accepted afterward.
        let policy = note_privacy::read_policy(&transaction, user_id)?;
        let restored_app_data = note_privacy::project_current(&policy, &historical.app_data_json)?
            .unwrap_or_else(|| historical.app_data_json.clone());
        let revision = next_account_revision(current.revision)?;
        if !snapshot_has_complete_recovery_copy(&transaction, &current)? {
            insert_snapshot_history(&transaction, &current, now_epoch_millis)?;
            if !snapshot_has_complete_recovery_copy(&transaction, &current)? {
                return Err(StoreError::Integrity(
                    "current account snapshot could not be archived completely; restore was not applied"
                        .to_string(),
                ));
            }
        }
        validate_app_data_media_identity_quota(&transaction, user_id, &restored_app_data)?;
        ensure_snapshot_write_capacity(&transaction, restored_app_data.len() as u64)?;
        let restore_generation = restore_generation_in_transaction(&transaction, user_id)?
            .checked_add(1)
            .ok_or_else(|| {
                StoreError::Integrity(
                    "restore generation is exhausted; refusing a restore without capability invalidation"
                        .to_string(),
                )
            })?;
        let content_sha256 = sha256_hex(restored_app_data.as_bytes());
        let envelope_sha256 = account_snapshot_envelope_sha256(
            user_id,
            &restored_app_data,
            revision,
            now_epoch_millis,
            restore_generation,
        );
        let changed = transaction.execute(
            "UPDATE account_snapshots \
             SET app_data_json = ?1, revision = ?2, updated_at_epoch_millis = ?3,
                 restore_generation = ?4, content_sha256 = ?5, envelope_sha256 = ?6 \
             WHERE user_id = ?7 AND revision = ?8",
            params![
                &restored_app_data,
                revision,
                now_epoch_millis,
                restore_generation,
                content_sha256,
                envelope_sha256,
                user_id,
                current.revision
            ],
        )?;
        if changed != 1 {
            return Err(StoreError::Integrity(
                "snapshot restore changed an unexpected row count".to_string(),
            ));
        }
        replace_current_snapshot_media_identities(&transaction, user_id, &restored_app_data)?;
        restore_media_snapshot(&transaction, user_id, &historical_media, now_epoch_millis)?;
        note_privacy::enforce(&transaction, user_id)?;
        transaction.execute(
            "UPDATE tokens
             SET restore_acknowledged = 0,
                 pending_restore_generation = NULL,
                 pending_restore_receipt = ''
             WHERE user_id = ?1",
            params![user_id],
        )?;
        transaction.execute(
            "UPDATE users SET updated_at_epoch_millis = ?1 WHERE id = ?2",
            params![now_epoch_millis, user_id],
        )?;
        privacy_journal::commit(transaction)?;
        Ok(AccountSnapshot {
            user_id: user_id.to_string(),
            app_data_json: restored_app_data,
            revision,
            updated_at_epoch_millis: now_epoch_millis,
        })
    }

    pub fn update_password_hash(
        &self,
        user_id: &str,
        password_salt: &str,
        password_hash: &str,
        password_scheme: &str,
        now_epoch_millis: i64,
    ) -> StoreResult<()> {
        if password_hash.trim().is_empty() || password_scheme.trim().is_empty() {
            return Err(StoreError::Integrity(
                "password hash and scheme cannot be empty".to_string(),
            ));
        }
        if password_scheme == "legacy_sha256" && password_salt.is_empty() {
            return Err(StoreError::Integrity(
                "legacy SHA-256 password requires a salt".to_string(),
            ));
        }
        let connection = self.open_connection(false)?;
        let changed = connection.execute(
            "UPDATE users SET password_salt = ?1, password_hash = ?2, password_scheme = ?3, \
                              updated_at_epoch_millis = ?4 \
             WHERE id = ?5",
            params![
                password_salt,
                password_hash,
                password_scheme,
                now_epoch_millis,
                user_id
            ],
        )?;
        if changed == 1 {
            Ok(())
        } else {
            Err(StoreError::NotFound(format!("user {user_id}")))
        }
    }

    pub fn compare_and_swap_account(
        &self,
        user_id: &str,
        expected_revision: i64,
        app_data_json: &str,
        now_epoch_millis: i64,
    ) -> StoreResult<AccountSnapshot> {
        if expected_revision < 0 {
            return Err(StoreError::Integrity(
                "expected revision cannot be negative".to_string(),
            ));
        }
        validate_app_data_json(app_data_json)?;
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = read_account_in_transaction(&transaction, user_id)?
            .ok_or_else(|| StoreError::NotFound(format!("account for user {user_id}")))?;
        validate_app_data_json(&current.app_data_json)?;
        if current.revision != expected_revision {
            return Err(StoreError::RevisionConflict {
                expected_revision,
                actual_revision: current.revision,
            });
        }
        validate_app_data_growth_quota(&current.app_data_json, app_data_json)?;
        validate_app_data_media_transition(&current.app_data_json, app_data_json)?;
        let revision = next_account_revision(expected_revision)?;
        insert_snapshot_history_for_account_mutation(
            &transaction,
            &self.database_path,
            &current,
            app_data_json,
            now_epoch_millis,
        )?;
        validate_app_data_media_identity_quota(&transaction, user_id, app_data_json)?;
        validate_prospective_current_archivability(
            &transaction,
            user_id,
            revision,
            app_data_json,
            now_epoch_millis,
        )?;
        ensure_snapshot_write_capacity(&transaction, app_data_json.len() as u64)?;
        let restore_generation = restore_generation_in_transaction(&transaction, user_id)?;
        let content_sha256 = sha256_hex(app_data_json.as_bytes());
        let envelope_sha256 = account_snapshot_envelope_sha256(
            user_id,
            app_data_json,
            revision,
            now_epoch_millis,
            restore_generation,
        );
        let changed = transaction.execute(
            "UPDATE account_snapshots \
             SET app_data_json = ?1, revision = ?2, updated_at_epoch_millis = ?3, \
                 content_sha256 = ?4, envelope_sha256 = ?5 \
             WHERE user_id = ?6 AND revision = ?7",
            params![
                app_data_json,
                revision,
                now_epoch_millis,
                content_sha256,
                envelope_sha256,
                user_id,
                expected_revision
            ],
        )?;
        if changed != 1 {
            return Err(StoreError::Integrity(
                "account CAS changed an unexpected row count".to_string(),
            ));
        }
        replace_current_snapshot_media_identities(&transaction, user_id, app_data_json)?;
        note_privacy::enforce(&transaction, user_id)?;
        transaction.execute(
            "UPDATE users SET updated_at_epoch_millis = ?1 WHERE id = ?2",
            params![now_epoch_millis, user_id],
        )?;
        let snapshot = transaction.query_row(
            "SELECT user_id, app_data_json, revision, updated_at_epoch_millis \
             FROM account_snapshots WHERE user_id = ?1",
            params![user_id],
            |row| {
                Ok(AccountSnapshot {
                    user_id: row.get(0)?,
                    app_data_json: row.get(1)?,
                    revision: row.get(2)?,
                    updated_at_epoch_millis: row.get(3)?,
                })
            },
        )?;
        privacy_journal::commit(transaction)?;
        Ok(snapshot)
    }

    pub fn issue_token(
        &self,
        user_id: &str,
        raw_token: &str,
        device_name: &str,
        created_at_epoch_millis: i64,
        expires_at_epoch_millis: i64,
    ) -> StoreResult<TokenMetadata> {
        if raw_token.trim().is_empty() {
            return Err(StoreError::Integrity("token cannot be empty".to_string()));
        }
        if expires_at_epoch_millis <= created_at_epoch_millis {
            return Err(StoreError::Integrity(
                "token expiry must be after creation".to_string(),
            ));
        }
        let hash = token_fingerprint(raw_token);
        let token_id = token_id_from_hash(&hash)?;
        let connection = self.open_connection(false)?;
        connection.execute(
            "INSERT INTO tokens( \
                 user_id, token_id, token_hash, device_name, created_at_epoch_millis, \
                 last_seen_at_epoch_millis, expires_at_epoch_millis, revoked_at_epoch_millis, \
                 last_seen_restore_generation, restore_acknowledged \
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, NULL, 0, 0)",
            params![
                user_id,
                token_id,
                hash,
                device_name.trim(),
                created_at_epoch_millis,
                expires_at_epoch_millis
            ],
        )?;
        let id = connection.last_insert_rowid();
        Ok(TokenMetadata {
            id,
            user_id: user_id.to_string(),
            token_id,
            token_hash: hash,
            device_name: device_name.trim().to_string(),
            created_at_epoch_millis,
            last_seen_at_epoch_millis: created_at_epoch_millis,
            expires_at_epoch_millis,
            revoked_at_epoch_millis: None,
        })
    }

    /// Issues a short-lived token that is unusable outside the normal sync
    /// activation path.  The caller supplies only the lease deadline here;
    /// the formal token expiry is written later by a committed normal sync.
    pub fn issue_pending_token(
        &self,
        user_id: &str,
        raw_token: &str,
        device_name: &str,
        created_at_epoch_millis: i64,
        activation_deadline_epoch_millis: i64,
    ) -> StoreResult<TokenMetadata> {
        if raw_token.trim().is_empty() {
            return Err(StoreError::Integrity("token cannot be empty".to_string()));
        }
        if activation_deadline_epoch_millis <= created_at_epoch_millis {
            return Err(StoreError::Integrity(
                "token activation deadline must be after creation".to_string(),
            ));
        }
        let hash = token_fingerprint(raw_token);
        let token_id = token_id_from_hash(&hash)?;
        let connection = self.open_connection(false)?;
        connection.execute(
            "INSERT INTO tokens( \
                 user_id, token_id, token_hash, device_name, created_at_epoch_millis, \
                 last_seen_at_epoch_millis, expires_at_epoch_millis, revoked_at_epoch_millis, \
                 last_seen_restore_generation, restore_acknowledged, activation_state, \
                 activated_at_epoch_millis \
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, NULL, 0, 0, 0, 0)",
            params![
                user_id,
                token_id,
                hash,
                device_name.trim(),
                created_at_epoch_millis,
                activation_deadline_epoch_millis
            ],
        )?;
        let id = connection.last_insert_rowid();
        Ok(TokenMetadata {
            id,
            user_id: user_id.to_string(),
            token_id,
            token_hash: hash,
            device_name: device_name.trim().to_string(),
            created_at_epoch_millis,
            last_seen_at_epoch_millis: created_at_epoch_millis,
            expires_at_epoch_millis: activation_deadline_epoch_millis,
            revoked_at_epoch_millis: None,
        })
    }

    /// Creates the empty account and its first, already-baselined token in one
    /// IMMEDIATE transaction.  Registration must not expose the crash window
    /// where the unique email is committed but no usable token exists.
    pub fn create_user_with_initial_token(
        &self,
        user: NewStoredUser,
        raw_token: &str,
        device_name: &str,
        created_at_epoch_millis: i64,
        expires_at_epoch_millis: i64,
    ) -> StoreResult<TokenMetadata> {
        validate_new_user(&user)?;
        if user.account_revision != 0 || !user.app_data_json.trim().is_empty() {
            return Err(StoreError::Integrity(
                "initial registration token requires a new empty account".to_string(),
            ));
        }
        if raw_token.trim().is_empty() {
            return Err(StoreError::Integrity("token cannot be empty".to_string()));
        }
        if expires_at_epoch_millis <= created_at_epoch_millis {
            return Err(StoreError::Integrity(
                "token expiry must be after creation".to_string(),
            ));
        }
        let hash = token_fingerprint(raw_token);
        let token_id = token_id_from_hash(&hash)?;
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_registered_account_capacity(&transaction, created_at_epoch_millis)?;
        insert_new_user(&transaction, &user, NewUserMediaPolicy::EnforceGrowth)?;
        transaction.execute(
            "INSERT INTO tokens(
                 user_id, token_id, token_hash, device_name, created_at_epoch_millis,
                 last_seen_at_epoch_millis, expires_at_epoch_millis, revoked_at_epoch_millis,
                 last_seen_restore_generation, restore_acknowledged
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, NULL, 0, 1)",
            params![
                user.id,
                token_id,
                hash,
                device_name.trim(),
                created_at_epoch_millis,
                expires_at_epoch_millis
            ],
        )?;
        let id = transaction.last_insert_rowid();
        privacy_journal::commit(transaction)?;
        Ok(TokenMetadata {
            id,
            user_id: user.id,
            token_id,
            token_hash: hash,
            device_name: device_name.trim().to_string(),
            created_at_epoch_millis,
            last_seen_at_epoch_millis: created_at_epoch_millis,
            expires_at_epoch_millis,
            revoked_at_epoch_millis: None,
        })
    }

    /// Creates an empty account and its short-lived unactivated token in one
    /// transaction.  Existing active-token constructors stay unchanged for
    /// migrations, administrative recovery, and backwards-compatible tests.
    pub fn create_user_with_initial_pending_token(
        &self,
        user: NewStoredUser,
        raw_token: &str,
        device_name: &str,
        created_at_epoch_millis: i64,
        activation_deadline_epoch_millis: i64,
    ) -> StoreResult<TokenMetadata> {
        validate_new_user(&user)?;
        if user.account_revision != 0 || !user.app_data_json.trim().is_empty() {
            return Err(StoreError::Integrity(
                "initial registration token requires a new empty account".to_string(),
            ));
        }
        if raw_token.trim().is_empty() {
            return Err(StoreError::Integrity("token cannot be empty".to_string()));
        }
        if activation_deadline_epoch_millis <= created_at_epoch_millis {
            return Err(StoreError::Integrity(
                "token activation deadline must be after creation".to_string(),
            ));
        }
        let hash = token_fingerprint(raw_token);
        let token_id = token_id_from_hash(&hash)?;
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_registered_account_capacity(&transaction, created_at_epoch_millis)?;
        insert_new_user(&transaction, &user, NewUserMediaPolicy::EnforceGrowth)?;
        transaction.execute(
            "INSERT INTO tokens(
                 user_id, token_id, token_hash, device_name, created_at_epoch_millis,
                 last_seen_at_epoch_millis, expires_at_epoch_millis, revoked_at_epoch_millis,
                 last_seen_restore_generation, restore_acknowledged, activation_state,
                 activated_at_epoch_millis
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, NULL, 0, 1, 0, 0)",
            params![
                user.id,
                token_id,
                hash,
                device_name.trim(),
                created_at_epoch_millis,
                activation_deadline_epoch_millis
            ],
        )?;
        let id = transaction.last_insert_rowid();
        privacy_journal::commit(transaction)?;
        Ok(TokenMetadata {
            id,
            user_id: user.id,
            token_id,
            token_hash: hash,
            device_name: device_name.trim().to_string(),
            created_at_epoch_millis,
            last_seen_at_epoch_millis: created_at_epoch_millis,
            expires_at_epoch_millis: activation_deadline_epoch_millis,
            revoked_at_epoch_millis: None,
        })
    }

    pub fn authenticate_token(
        &self,
        raw_token: &str,
        now_epoch_millis: i64,
    ) -> StoreResult<TokenAuthentication> {
        if raw_token.is_empty() {
            return Ok(TokenAuthentication::Unknown);
        }
        let hash = token_fingerprint(raw_token);
        let connection = self.open_connection(false)?;
        let token = connection
            .query_row(
                "SELECT id, token_id, user_id, device_name, last_seen_at_epoch_millis, \
                        expires_at_epoch_millis, revoked_at_epoch_millis,
                        last_seen_restore_generation, activation_state \
                 FROM tokens WHERE token_hash = ?1",
                params![hash],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            token_id,
            token_identifier,
            user_id,
            device_name,
            last_seen_at,
            expires_at,
            revoked_at,
            last_seen_restore_generation,
            activation_state,
        )) = token
        else {
            return Ok(TokenAuthentication::Unknown);
        };
        if revoked_at.is_some() {
            return Ok(TokenAuthentication::Revoked);
        }
        if expires_at <= now_epoch_millis {
            return Ok(TokenAuthentication::Expired);
        }
        let authenticated = AuthenticatedToken {
            token_id,
            token_identifier,
            user_id,
            device_name,
            expires_at_epoch_millis: expires_at,
            last_seen_restore_generation,
        };
        if activation_state == 0 {
            return Ok(TokenAuthentication::PendingActivation(authenticated));
        }
        if activation_state != 1 {
            return Err(StoreError::Integrity(format!(
                "token {token_id} has an invalid activation state"
            )));
        }
        if last_seen_at < now_epoch_millis.saturating_sub(TOKEN_LAST_SEEN_WRITE_INTERVAL_MILLIS) {
            let changed = connection.execute(
                "UPDATE tokens SET last_seen_at_epoch_millis = ?1 \
                 WHERE id = ?2 \
                   AND last_seen_at_epoch_millis < ?3 \
                   AND revoked_at_epoch_millis IS NULL \
                   AND expires_at_epoch_millis > ?1",
                params![
                    now_epoch_millis,
                    token_id,
                    now_epoch_millis.saturating_sub(TOKEN_LAST_SEEN_WRITE_INTERVAL_MILLIS)
                ],
            )?;
            if changed == 0 {
                return self.authenticate_token(raw_token, now_epoch_millis);
            }
        }
        Ok(TokenAuthentication::Active(authenticated))
    }

    /// Extends an authenticated active session only while its current lease is
    /// valid. Callers should use this near expiry to avoid a write on each
    /// request; the update predicate also rejects a concurrent revocation.
    pub fn renew_active_token_lease(
        &self,
        token_id: i64,
        now_epoch_millis: i64,
        renewed_expiry_epoch_millis: i64,
    ) -> StoreResult<bool> {
        if renewed_expiry_epoch_millis <= now_epoch_millis {
            return Err(StoreError::Integrity(
                "renewed token expiry must be in the future".to_string(),
            ));
        }
        let connection = self.open_connection(false)?;
        let changed = connection.execute(
            "UPDATE tokens
             SET expires_at_epoch_millis = MAX(expires_at_epoch_millis, ?1),
                 last_seen_at_epoch_millis = MAX(last_seen_at_epoch_millis, ?2)
             WHERE id = ?3
               AND activation_state = 1
               AND revoked_at_epoch_millis IS NULL
               AND expires_at_epoch_millis > ?2",
            params![renewed_expiry_epoch_millis, now_epoch_millis, token_id],
        )?;
        Ok(changed == 1)
    }

    /// A raw bearer token can bridge a short expiry gap only when its active
    /// session was used recently and was never explicitly revoked. The lookup
    /// and renewal share one immediate transaction so another request cannot
    /// revoke or replace the lease between the two steps.
    pub fn recover_recent_active_token_lease(
        &self,
        raw_token: &str,
        now_epoch_millis: i64,
        recovery_grace_millis: i64,
        recent_activity_window_millis: i64,
        renewed_lease_millis: i64,
    ) -> StoreResult<Option<AuthenticatedToken>> {
        if raw_token.trim().is_empty() {
            return Ok(None);
        }
        if recovery_grace_millis <= 0
            || recent_activity_window_millis <= 0
            || renewed_lease_millis <= 0
        {
            return Err(StoreError::Integrity(
                "active token recovery windows must be positive".to_string(),
            ));
        }
        let hash = token_fingerprint(raw_token);
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let token = transaction
            .query_row(
                "SELECT id, token_id, user_id, device_name, created_at_epoch_millis,
                        last_seen_at_epoch_millis, expires_at_epoch_millis,
                        revoked_at_epoch_millis, last_seen_restore_generation, activation_state
                 FROM tokens WHERE token_hash = ?1",
                params![hash],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, Option<i64>>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, i64>(9)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            id,
            token_identifier,
            user_id,
            device_name,
            created_at,
            last_seen_at,
            expires_at,
            revoked_at,
            last_seen_restore_generation,
            activation_state,
        )) = token
        else {
            privacy_journal::commit(transaction)?;
            return Ok(None);
        };
        let eligible = activation_state == 1
            && revoked_at.is_none()
            && created_at <= last_seen_at
            && last_seen_at <= expires_at
            && last_seen_at >= expires_at.saturating_sub(recent_activity_window_millis)
            && expires_at <= now_epoch_millis
            && now_epoch_millis <= expires_at.saturating_add(recovery_grace_millis);
        if !eligible {
            privacy_journal::commit(transaction)?;
            return Ok(None);
        }
        let renewed_expiry = now_epoch_millis.saturating_add(renewed_lease_millis);
        let changed = transaction.execute(
            "UPDATE tokens
             SET expires_at_epoch_millis = ?1,
                 last_seen_at_epoch_millis = ?2
             WHERE id = ?3
               AND token_hash = ?4
               AND activation_state = 1
               AND revoked_at_epoch_millis IS NULL
               AND expires_at_epoch_millis = ?5",
            params![renewed_expiry, now_epoch_millis, id, hash, expires_at],
        )?;
        if changed != 1 {
            return Err(StoreError::Integrity(
                "active token recovery changed an unexpected row count".to_string(),
            ));
        }
        privacy_journal::commit(transaction)?;
        Ok(Some(AuthenticatedToken {
            token_id: id,
            token_identifier,
            user_id,
            device_name,
            expires_at_epoch_millis: renewed_expiry,
            last_seen_restore_generation,
        }))
    }

    /// Authenticates possession of a naturally expired, never-activated login
    /// token for one normal-sync recovery attempt. This lookup is read-only:
    /// the token remains expired/revoked until the normal sync and activation
    /// commit atomically in the same database transaction.
    pub fn pending_token_recovery_candidate_for_normal_sync(
        &self,
        raw_token: &str,
        now_epoch_millis: i64,
        original_lease_millis: i64,
        recovery_window_millis: i64,
    ) -> StoreResult<Option<PendingTokenRecoveryCandidate>> {
        if raw_token.is_empty() {
            return Ok(None);
        }
        if original_lease_millis <= 0 || recovery_window_millis <= 0 {
            return Err(StoreError::Integrity(
                "pending token recovery windows must be positive".to_string(),
            ));
        }
        let hash = token_fingerprint(raw_token);
        let connection = self.open_connection(false)?;
        let token = connection
            .query_row(
                "SELECT id, token_id, user_id, device_name, created_at_epoch_millis,
                        expires_at_epoch_millis, revoked_at_epoch_millis,
                        last_seen_restore_generation, activation_state
                 FROM tokens WHERE token_hash = ?1",
                params![hash],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            token_id,
            token_identifier,
            user_id,
            device_name,
            created_at,
            expires_at,
            revoked_at,
            last_seen_restore_generation,
            activation_state,
        )) = token
        else {
            return Ok(None);
        };
        let eligible = activation_state == 0
            && expires_at <= now_epoch_millis
            && (revoked_at.is_none() || revoked_at == Some(expires_at))
            && expires_at == created_at.saturating_add(original_lease_millis)
            && now_epoch_millis <= expires_at.saturating_add(recovery_window_millis);
        if !eligible {
            return Ok(None);
        }
        Ok(Some(PendingTokenRecoveryCandidate {
            token: AuthenticatedToken {
                token_id,
                token_identifier,
                user_id,
                device_name,
                expires_at_epoch_millis: expires_at,
                last_seen_restore_generation,
            },
            original_expires_at_epoch_millis: expires_at,
        }))
    }

    /// Test-only primitive for exercising token-state transitions directly.
    /// Production activation is part of the normal sync commit transaction.
    #[cfg(test)]
    pub fn activate_pending_token(
        &self,
        raw_token: &str,
        now_epoch_millis: i64,
        active_expires_at_epoch_millis: i64,
    ) -> StoreResult<TokenAuthentication> {
        if raw_token.is_empty() {
            return Ok(TokenAuthentication::Unknown);
        }
        if active_expires_at_epoch_millis <= now_epoch_millis {
            return Err(StoreError::Integrity(
                "activated token expiry must be after activation".to_string(),
            ));
        }
        let hash = token_fingerprint(raw_token);
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let token = transaction
            .query_row(
                "SELECT id, token_id, user_id, device_name, created_at_epoch_millis,
                        expires_at_epoch_millis, revoked_at_epoch_millis,
                        last_seen_restore_generation, activation_state
                 FROM tokens WHERE token_hash = ?1",
                params![hash],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            id,
            token_identifier,
            user_id,
            device_name,
            created_at_epoch_millis,
            current_expiry,
            revoked_at,
            last_seen_restore_generation,
            activation_state,
        )) = token
        else {
            privacy_journal::commit(transaction)?;
            return Ok(TokenAuthentication::Unknown);
        };
        if revoked_at.is_some() {
            privacy_journal::commit(transaction)?;
            return Ok(TokenAuthentication::Revoked);
        }
        if current_expiry <= now_epoch_millis {
            privacy_journal::commit(transaction)?;
            return Ok(TokenAuthentication::Expired);
        }
        if activation_state == 1 {
            privacy_journal::commit(transaction)?;
            return Ok(TokenAuthentication::Active(AuthenticatedToken {
                token_id: id,
                token_identifier,
                user_id,
                device_name,
                expires_at_epoch_millis: current_expiry,
                last_seen_restore_generation,
            }));
        }
        if activation_state != 0 {
            return Err(StoreError::Integrity(format!(
                "token {id} has an invalid activation state"
            )));
        }
        if active_expires_at_epoch_millis <= created_at_epoch_millis {
            return Err(StoreError::Integrity(
                "activated token expiry must be after token creation".to_string(),
            ));
        }
        let changed = transaction.execute(
            "UPDATE tokens
             SET activation_state = 1,
                 activated_at_epoch_millis = ?1,
                 expires_at_epoch_millis = ?2,
                 last_seen_at_epoch_millis = ?1
             WHERE id = ?3
               AND activation_state = 0
               AND revoked_at_epoch_millis IS NULL
               AND expires_at_epoch_millis > ?1",
            params![now_epoch_millis, active_expires_at_epoch_millis, id],
        )?;
        if changed != 1 {
            return Err(StoreError::Integrity(
                "pending token activation changed an unexpected row count".to_string(),
            ));
        }
        privacy_journal::commit(transaction)?;
        Ok(TokenAuthentication::Active(AuthenticatedToken {
            token_id: id,
            token_identifier,
            user_id,
            device_name,
            expires_at_epoch_millis: active_expires_at_epoch_millis,
            last_seen_restore_generation,
        }))
    }

    pub fn revoke_token(&self, raw_token: &str, revoked_at_epoch_millis: i64) -> StoreResult<bool> {
        if raw_token.is_empty() {
            return Ok(false);
        }
        let connection = self.open_connection(false)?;
        Ok(connection.execute(
            "UPDATE tokens SET revoked_at_epoch_millis = ?1 \
             WHERE token_hash = ?2 AND revoked_at_epoch_millis IS NULL",
            params![revoked_at_epoch_millis, token_fingerprint(raw_token)],
        )? == 1)
    }

    /// Revokes a known bearer token and returns its owner. Repeating logout is
    /// idempotent. If cleanup created an expiry tombstone for a pending token,
    /// the first explicit logout replaces that marker so discovery recovery
    /// can never mistake a user-revoked session for an abandoned first sync.
    pub fn revoke_token_for_logout(
        &self,
        raw_token: &str,
        revoked_at_epoch_millis: i64,
    ) -> StoreResult<Option<String>> {
        if raw_token.is_empty() {
            return Ok(None);
        }
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let user_id = transaction
            .query_row(
                "SELECT user_id FROM tokens WHERE token_hash = ?1",
                params![token_fingerprint(raw_token)],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(user_id) = user_id else {
            privacy_journal::commit(transaction)?;
            return Ok(None);
        };
        let changed = transaction.execute(
            "UPDATE tokens
             SET revoked_at_epoch_millis = CASE
                     WHEN activation_state = 0
                      AND revoked_at_epoch_millis = expires_at_epoch_millis
                     THEN ?1
                     ELSE COALESCE(revoked_at_epoch_millis, ?1)
                 END
             WHERE token_hash = ?2",
            params![revoked_at_epoch_millis, token_fingerprint(raw_token)],
        )?;
        if changed != 1 {
            return Err(StoreError::Integrity(
                "logout token revocation changed an unexpected row count".to_string(),
            ));
        }
        privacy_journal::commit(transaction)?;
        Ok(Some(user_id))
    }

    /// Converts expired pending leases into revocation tombstones, then
    /// removes only tombstones older than the retention window.  This bounds
    /// abandoned login rows while keeping recent logout retries idempotent.
    pub fn cleanup_expired_pending_tokens(&self, now_epoch_millis: i64) -> StoreResult<usize> {
        let mut connection = self.open_connection(false)?;
        cleanup_expired_pending_tokens_in_connection(&mut connection, now_epoch_millis)
    }

    pub fn revoke_all_user_tokens(
        &self,
        user_id: &str,
        revoked_at_epoch_millis: i64,
    ) -> StoreResult<usize> {
        let connection = self.open_connection(false)?;
        connection
            .execute(
                "UPDATE tokens SET revoked_at_epoch_millis = ?1 \
                 WHERE user_id = ?2 AND revoked_at_epoch_millis IS NULL",
                params![revoked_at_epoch_millis, user_id],
            )
            .map_err(StoreError::from)
    }

    /// Returns a deterministic key derived from the bearer token hash for an
    /// active token id. Raw bearer tokens are never returned or stored.
    pub fn active_token_key_by_id(
        &self,
        user_id: &str,
        token_id: &str,
        now_epoch_millis: i64,
    ) -> StoreResult<Option<[u8; 32]>> {
        let connection = self.open_connection(false)?;
        let token_hash = connection
            .query_row(
                "SELECT token_hash FROM tokens \
                 WHERE user_id = ?1 AND token_id = ?2 \
                   AND revoked_at_epoch_millis IS NULL \
                   AND activation_state = 1 \
                   AND expires_at_epoch_millis > ?3",
                params![user_id, token_id, now_epoch_millis],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        token_hash.map(|hash| decode_sha256_hex(&hash)).transpose()
    }

    /// Returns the token-derived key used only to prove discovery ownership.
    /// A freshly authenticated token may still be pending its first durable
    /// normal sync, so both pending and active states are eligible while the
    /// token's current lease is live. This lookup does not activate the token.
    pub fn discovery_token_key_by_id(
        &self,
        user_id: &str,
        token_id: &str,
        now_epoch_millis: i64,
    ) -> StoreResult<Option<[u8; 32]>> {
        let connection = self.open_connection(false)?;
        let token_hash = connection
            .query_row(
                "SELECT token_hash FROM tokens \
                 WHERE user_id = ?1 AND token_id = ?2 \
                   AND revoked_at_epoch_millis IS NULL \
                   AND activation_state IN (0, 1) \
                   AND expires_at_epoch_millis > ?3",
                params![user_id, token_id, now_epoch_millis],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        token_hash.map(|hash| decode_sha256_hex(&hash)).transpose()
    }

    /// Resolves the key for discovery proof. A URL-bound token proof can renew
    /// a recently used active session during a short expiry grace period. The
    /// same proof also recovers a naturally expired first-sync lease once;
    /// its replacement cannot satisfy the original lease predicate again.
    #[allow(clippy::too_many_arguments)]
    pub fn discovery_token_key_by_id_with_recovery(
        &self,
        user_id: &str,
        token_id: &str,
        nonce: &str,
        public_server_url: &str,
        client_proof: &str,
        now_epoch_millis: i64,
        original_lease_millis: i64,
        recovery_window_millis: i64,
        active_token_ttl_millis: i64,
        active_expiry_grace_millis: i64,
        recent_activity_window_millis: i64,
    ) -> StoreResult<DiscoveryTokenKeyLookup> {
        if original_lease_millis <= 0
            || recovery_window_millis <= 0
            || active_token_ttl_millis <= 0
            || active_expiry_grace_millis <= 0
            || recent_activity_window_millis <= 0
        {
            return Err(StoreError::Integrity(
                "discovery recovery windows must be positive".to_string(),
            ));
        }
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let token = transaction
            .query_row(
                "SELECT id, token_hash, created_at_epoch_millis,
                        last_seen_at_epoch_millis,
                        expires_at_epoch_millis, revoked_at_epoch_millis, activation_state
                 FROM tokens WHERE user_id = ?1 AND token_id = ?2",
                params![user_id, token_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                        row.get::<_, i64>(6)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            id,
            token_hash,
            created_at,
            last_seen_at,
            expires_at,
            revoked_at,
            activation_state,
        )) = token
        else {
            privacy_journal::commit(transaction)?;
            return Ok(DiscoveryTokenKeyLookup::Invalid);
        };
        let key = decode_sha256_hex(&token_hash)?;
        if revoked_at.is_none()
            && matches!(activation_state, 0 | 1)
            && expires_at > now_epoch_millis
        {
            privacy_journal::commit(transaction)?;
            return Ok(DiscoveryTokenKeyLookup::Available {
                key,
                recovered_pending_lease: false,
            });
        }

        // A recently used active device may bridge a short offline gap after
        // the fixed legacy lease expires. The URL-bound MAC proves possession
        // of its original token; an old, revoked or never-used token cannot
        // be revived through discovery.
        let recent_active_expiry = activation_state == 1
            && revoked_at.is_none()
            && expires_at <= now_epoch_millis
            && now_epoch_millis <= expires_at.saturating_add(active_expiry_grace_millis)
            && last_seen_at >= expires_at.saturating_sub(recent_activity_window_millis)
            && last_seen_at <= expires_at;
        if recent_active_expiry {
            let recovery_message = format!(
                "gridtimer.discovery.pending-recovery.v1\n{user_id}\n{token_id}\n{nonce}\n{public_server_url}"
            );
            let expected_proof = hmac_sha256_hex(&key, recovery_message.as_bytes());
            if !valid_sha256_hex(client_proof)
                || !constant_time_bytes_eq(expected_proof.as_bytes(), client_proof.as_bytes())
            {
                privacy_journal::commit(transaction)?;
                return Ok(DiscoveryTokenKeyLookup::Invalid);
            }
            let replacement_expiry = now_epoch_millis.saturating_add(active_token_ttl_millis);
            let changed = transaction.execute(
                "UPDATE tokens
                 SET expires_at_epoch_millis = ?1,
                     last_seen_at_epoch_millis = ?2
                 WHERE id = ?3
                   AND activation_state = 1
                   AND revoked_at_epoch_millis IS NULL
                   AND expires_at_epoch_millis = ?4",
                params![replacement_expiry, now_epoch_millis, id, expires_at],
            )?;
            if changed != 1 {
                return Err(StoreError::Integrity(
                    "active discovery recovery changed an unexpected row count".to_string(),
                ));
            }
            privacy_journal::commit(transaction)?;
            return Ok(DiscoveryTokenKeyLookup::RecoveredActiveLease { key });
        }

        let automatically_expired_pending = activation_state == 0
            && expires_at <= now_epoch_millis
            && (revoked_at.is_none() || revoked_at == Some(expires_at))
            && expires_at == created_at.saturating_add(original_lease_millis);
        if !automatically_expired_pending {
            privacy_journal::commit(transaction)?;
            return Ok(DiscoveryTokenKeyLookup::Invalid);
        }
        if now_epoch_millis > expires_at.saturating_add(recovery_window_millis) {
            privacy_journal::commit(transaction)?;
            return Ok(DiscoveryTokenKeyLookup::PendingReauthenticationRequired);
        }
        // Compatibility for clients released before token-bound recovery was
        // added: disclose only the usual nonce-bound discovery MAC. The row is
        // not renewed here. A later normal sync must still prove possession of
        // the full raw Bearer token and commits activation atomically.
        if client_proof.is_empty() {
            privacy_journal::commit(transaction)?;
            return Ok(DiscoveryTokenKeyLookup::Available {
                key,
                recovered_pending_lease: false,
            });
        }
        if !valid_sha256_hex(client_proof) {
            privacy_journal::commit(transaction)?;
            return Ok(DiscoveryTokenKeyLookup::Invalid);
        }
        let recovery_message = format!(
            "gridtimer.discovery.pending-recovery.v1\n{user_id}\n{token_id}\n{nonce}\n{public_server_url}"
        );
        let expected_proof = hmac_sha256_hex(&key, recovery_message.as_bytes());
        if !constant_time_bytes_eq(expected_proof.as_bytes(), client_proof.as_bytes()) {
            privacy_journal::commit(transaction)?;
            return Ok(DiscoveryTokenKeyLookup::Invalid);
        }
        let replacement_expiry = now_epoch_millis.saturating_add(original_lease_millis);
        let changed = transaction.execute(
            "UPDATE tokens
             SET revoked_at_epoch_millis = NULL,
                 expires_at_epoch_millis = ?1
             WHERE id = ?2
               AND activation_state = 0
               AND created_at_epoch_millis = ?3
               AND expires_at_epoch_millis = ?4
               AND (revoked_at_epoch_millis IS NULL OR revoked_at_epoch_millis = ?4)",
            params![replacement_expiry, id, created_at, expires_at],
        )?;
        if changed != 1 {
            return Err(StoreError::Integrity(
                "pending discovery recovery changed an unexpected row count".to_string(),
            ));
        }
        privacy_journal::commit(transaction)?;
        Ok(DiscoveryTokenKeyLookup::Available {
            key,
            recovered_pending_lease: true,
        })
    }

    pub fn list_token_metadata(&self, user_id: &str) -> StoreResult<Vec<TokenMetadata>> {
        let connection = self.open_connection(false)?;
        let mut statement = connection.prepare(
            "SELECT id, user_id, token_id, token_hash, device_name, created_at_epoch_millis, \
                    last_seen_at_epoch_millis, expires_at_epoch_millis, revoked_at_epoch_millis \
             FROM tokens WHERE user_id = ?1 ORDER BY id",
        )?;
        let rows = statement.query_map(params![user_id], |row| {
            Ok(TokenMetadata {
                id: row.get(0)?,
                user_id: row.get(1)?,
                token_id: row.get(2)?,
                token_hash: row.get(3)?,
                device_name: row.get(4)?,
                created_at_epoch_millis: row.get(5)?,
                last_seen_at_epoch_millis: row.get(6)?,
                expires_at_epoch_millis: row.get(7)?,
                revoked_at_epoch_millis: row.get(8)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Applies a snapshot CAS and stores the exact response under `request_id`
    /// in the same transaction. Repeated delivery returns the stored response
    /// without changing the account revision a second time.
    pub(crate) fn note_media_intent_policy(
        &self,
        user_id: &str,
    ) -> StoreResult<crate::desktop_state_store::DesktopPrivacyPolicy> {
        note_privacy::read_policy(&self.open_connection(false)?, user_id)
    }

    pub fn apply_sync_request(
        &self,
        user_id: &str,
        request_id: &str,
        operation_payload_json: &str,
        expected_revision: i64,
        app_data_json: &str,
        response_json: &str,
        now_epoch_millis: i64,
    ) -> StoreResult<SyncRequestOutcome> {
        self.apply_sync_request_with_limits(
            user_id,
            request_id,
            operation_payload_json,
            expected_revision,
            app_data_json,
            response_json,
            now_epoch_millis,
            REQUEST_DEDUP_MAX_PER_USER,
            REQUEST_DEDUP_MAX_TOTAL_BYTES_PER_USER,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_sync_request_for_generation(
        &self,
        user_id: &str,
        token_id: i64,
        acknowledged_generation: i64,
        restore_receipt: &str,
        request_id: &str,
        operation_payload_json: &str,
        expected_revision: i64,
        app_data_json: &str,
        response_json: &str,
        now_epoch_millis: i64,
    ) -> StoreResult<SyncRequestOutcome> {
        self.apply_sync_request_for_generation_with_pending_activation(
            user_id,
            token_id,
            acknowledged_generation,
            restore_receipt,
            request_id,
            operation_payload_json,
            expected_revision,
            app_data_json,
            response_json,
            now_epoch_millis,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_sync_request_for_generation_with_pending_activation(
        &self,
        user_id: &str,
        token_id: i64,
        acknowledged_generation: i64,
        restore_receipt: &str,
        request_id: &str,
        operation_payload_json: &str,
        expected_revision: i64,
        app_data_json: &str,
        response_json: &str,
        now_epoch_millis: i64,
        pending_activation: Option<PendingTokenActivation>,
    ) -> StoreResult<SyncRequestOutcome> {
        self.apply_sync_request_with_barrier_and_limits(
            user_id,
            request_id,
            operation_payload_json,
            expected_revision,
            app_data_json,
            response_json,
            now_epoch_millis,
            REQUEST_DEDUP_MAX_PER_USER,
            REQUEST_DEDUP_MAX_TOTAL_BYTES_PER_USER,
            Some((token_id, acknowledged_generation, restore_receipt, false)),
            pending_activation,
            None,
        )
    }

    /// Applies a normal sync from a client that proved the stable server/account
    /// workspace binding. The proof itself is checked by the protocol layer;
    /// this flag only permits a receiptless, current-generation token baseline
    /// to be acknowledged in the same transaction as the account CAS write.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_sync_request_for_generation_with_pending_activation_and_media_intents(
        &self,
        user_id: &str,
        token_id: i64,
        acknowledged_generation: i64,
        restore_receipt: &str,
        request_id: &str,
        operation_payload_json: &str,
        expected_revision: i64,
        app_data_json: &str,
        response_json: &str,
        now_epoch_millis: i64,
        pending_activation: Option<PendingTokenActivation>,
        incoming_raw_app_data_json: &str,
    ) -> StoreResult<SyncRequestOutcome> {
        self.apply_sync_request_with_barrier_and_limits(
            user_id,
            request_id,
            operation_payload_json,
            expected_revision,
            app_data_json,
            response_json,
            now_epoch_millis,
            REQUEST_DEDUP_MAX_PER_USER,
            REQUEST_DEDUP_MAX_TOTAL_BYTES_PER_USER,
            Some((token_id, acknowledged_generation, restore_receipt, false)),
            pending_activation,
            Some(incoming_raw_app_data_json),
        )
    }

    /// Applies a normal sync from a client that proved the stable server/account
    /// workspace binding. The proof itself is checked by the protocol layer;
    /// this flag only permits a receiptless, current-generation token baseline
    /// to be acknowledged in the same transaction as the account CAS write.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_workspace_bound_sync_request_for_generation(
        &self,
        user_id: &str,
        token_id: i64,
        acknowledged_generation: i64,
        request_id: &str,
        operation_payload_json: &str,
        expected_revision: i64,
        app_data_json: &str,
        response_json: &str,
        now_epoch_millis: i64,
    ) -> StoreResult<SyncRequestOutcome> {
        self.apply_workspace_bound_sync_request_for_generation_with_pending_activation(
            user_id,
            token_id,
            acknowledged_generation,
            request_id,
            operation_payload_json,
            expected_revision,
            app_data_json,
            response_json,
            now_epoch_millis,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_workspace_bound_sync_request_for_generation_with_pending_activation(
        &self,
        user_id: &str,
        token_id: i64,
        acknowledged_generation: i64,
        request_id: &str,
        operation_payload_json: &str,
        expected_revision: i64,
        app_data_json: &str,
        response_json: &str,
        now_epoch_millis: i64,
        pending_activation: Option<PendingTokenActivation>,
    ) -> StoreResult<SyncRequestOutcome> {
        self.apply_sync_request_with_barrier_and_limits(
            user_id,
            request_id,
            operation_payload_json,
            expected_revision,
            app_data_json,
            response_json,
            now_epoch_millis,
            REQUEST_DEDUP_MAX_PER_USER,
            REQUEST_DEDUP_MAX_TOTAL_BYTES_PER_USER,
            Some((token_id, acknowledged_generation, "", true)),
            pending_activation,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_workspace_bound_sync_request_for_generation_with_pending_activation_and_media_intents(
        &self,
        user_id: &str,
        token_id: i64,
        acknowledged_generation: i64,
        request_id: &str,
        operation_payload_json: &str,
        expected_revision: i64,
        app_data_json: &str,
        response_json: &str,
        now_epoch_millis: i64,
        pending_activation: Option<PendingTokenActivation>,
        incoming_raw_app_data_json: &str,
    ) -> StoreResult<SyncRequestOutcome> {
        self.apply_sync_request_with_barrier_and_limits(
            user_id,
            request_id,
            operation_payload_json,
            expected_revision,
            app_data_json,
            response_json,
            now_epoch_millis,
            REQUEST_DEDUP_MAX_PER_USER,
            REQUEST_DEDUP_MAX_TOTAL_BYTES_PER_USER,
            Some((token_id, acknowledged_generation, "", true)),
            pending_activation,
            Some(incoming_raw_app_data_json),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_sync_request_with_limits(
        &self,
        user_id: &str,
        request_id: &str,
        operation_payload_json: &str,
        expected_revision: i64,
        app_data_json: &str,
        response_json: &str,
        now_epoch_millis: i64,
        max_receipts_per_user: i64,
        max_response_bytes_per_user: i64,
    ) -> StoreResult<SyncRequestOutcome> {
        self.apply_sync_request_with_barrier_and_limits(
            user_id,
            request_id,
            operation_payload_json,
            expected_revision,
            app_data_json,
            response_json,
            now_epoch_millis,
            max_receipts_per_user,
            max_response_bytes_per_user,
            None,
            None,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_sync_request_with_barrier_and_limits(
        &self,
        user_id: &str,
        request_id: &str,
        operation_payload_json: &str,
        expected_revision: i64,
        app_data_json: &str,
        response_json: &str,
        now_epoch_millis: i64,
        max_receipts_per_user: i64,
        max_response_bytes_per_user: i64,
        restore_barrier: Option<(i64, i64, &str, bool)>,
        pending_activation: Option<PendingTokenActivation>,
        incoming_raw_app_data_json: Option<&str>,
    ) -> StoreResult<SyncRequestOutcome> {
        if let Some(activation) = pending_activation {
            if activation.activated_at_epoch_millis != now_epoch_millis {
                return Err(StoreError::Integrity(
                    "pending token activation time must match the sync commit time".to_string(),
                ));
            }
            if restore_barrier.map(|barrier| barrier.0) != Some(activation.token_id) {
                return Err(StoreError::Integrity(
                    "pending token activation must match the sync restore barrier token"
                        .to_string(),
                ));
            }
        }
        if request_id.trim().is_empty() {
            return Err(StoreError::Integrity(
                "request id cannot be empty".to_string(),
            ));
        }
        if request_id.len() > 256 || request_id.chars().any(char::is_control) {
            return Err(StoreError::Integrity(
                "request id is too long or contains control characters".to_string(),
            ));
        }
        if response_json.len() > REQUEST_DEDUP_MAX_RESPONSE_BYTES {
            return Err(StoreError::Integrity(format!(
                "sync response exceeds the {} byte deduplication limit",
                REQUEST_DEDUP_MAX_RESPONSE_BYTES
            )));
        }
        if expected_revision < 0 {
            return Err(StoreError::Integrity(
                "expected revision cannot be negative".to_string(),
            ));
        }
        if max_receipts_per_user <= 0 || max_response_bytes_per_user <= 0 {
            return Err(StoreError::Integrity(
                "request deduplication limits must be positive".to_string(),
            ));
        }
        validate_app_data_json(app_data_json)?;
        validate_json_document(operation_payload_json, "sync operation payload")?;
        validate_json_document(response_json, "sync response")?;
        let legacy_request_fingerprint = canonical_json_sha256(operation_payload_json)?;
        let request_fingerprint =
            if let Some(raw) = incoming_raw_app_data_json {
                validate_app_data_json(raw)?;
                let incoming: serde_json::Value = if raw.trim().is_empty() {
                    serde_json::json!({})
                } else {
                    serde_json::from_str(raw)?
                };
                canonical_json_sha256(&serde_json::json!({
                "domain":"windows-note-media-intent-v1",
                "operation":serde_json::from_str::<serde_json::Value>(operation_payload_json)?,
                "incoming":incoming,
            }).to_string())?
            } else {
                legacy_request_fingerprint.clone()
            };
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = read_account_in_transaction(&transaction, user_id)?
            .ok_or_else(|| StoreError::NotFound(format!("account for user {user_id}")))?;
        validate_app_data_json(&current.app_data_json)?;
        acknowledge_restore_barrier_in_transaction(&transaction, user_id, restore_barrier)?;
        transaction.execute(
            "DELETE FROM request_dedup WHERE created_at_epoch_millis < ?1",
            params![now_epoch_millis.saturating_sub(REQUEST_DEDUP_RETENTION_MILLIS)],
        )?;
        let replay = transaction
            .query_row(
                "SELECT user_id, request_fingerprint, response_json, account_revision \
                 FROM request_dedup WHERE request_id = ?1",
                params![request_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()?;
        if let Some((stored_user_id, stored_fingerprint, response_json, account_revision)) = replay
        {
            if stored_user_id != user_id {
                return Err(StoreError::Integrity(
                    "request id is already owned by another user".to_string(),
                ));
            }
            // Receipts written by the previous version remain spent. A legacy
            // match only replays the retained response; it never accepts new intent.
            if stored_fingerprint != request_fingerprint
                && !(incoming_raw_app_data_json.is_some()
                    && stored_fingerprint == legacy_request_fingerprint)
            {
                return Err(StoreError::Integrity(
                    "request id was reused with a different operation payload".to_string(),
                ));
            }
            let policy = note_privacy::read_policy(&transaction, user_id)?;
            let response_json =
                note_privacy::redact_response(&policy, &response_json)?.unwrap_or(response_json);
            activate_pending_token_in_transaction(&transaction, user_id, pending_activation)?;
            privacy_journal::commit(transaction)?;
            return Ok(SyncRequestOutcome::Replayed(SyncRequestReceipt {
                response_json,
                account_revision,
            }));
        }

        if current.revision != expected_revision {
            return Err(StoreError::RevisionConflict {
                expected_revision,
                actual_revision: current.revision,
            });
        }
        // Re-read policy and raw evidence inside this transaction. Reference
        // declarations can change without advancing the account revision.
        let policy = note_privacy::read_policy(&transaction, user_id)?;
        let mut detachments = policy.note_attachment_detachments().clone();
        let mut media_candidates = media_privacy::Candidates::new();
        let original_proposed: serde_json::Value = if app_data_json.trim().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(app_data_json)?
        };
        let mut proposed = original_proposed.clone();
        if let Some(raw) = incoming_raw_app_data_json {
            let before: serde_json::Value = if current.app_data_json.trim().is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str(&current.app_data_json)?
            };
            let incoming: serde_json::Value = if raw.trim().is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str(raw)?
            };
            let prior_intents = crate::note_media_intent::infer_snapshot_detachments(&before)
                .map_err(StoreError::Integrity)?;
            crate::note_media_intent::merge_detachments(&mut detachments, &prior_intents)
                .map_err(StoreError::Integrity)?;
            let inferred =
                crate::note_media_intent::infer_transition_detachments(&before, &incoming)
                    .map_err(StoreError::Integrity)?;
            crate::note_media_intent::merge_detachments(&mut detachments, &inferred)
                .map_err(StoreError::Integrity)?;
            let replayed =
                crate::note_media_intent::infer_transition_detachments(&incoming, &before)
                    .map_err(StoreError::Integrity)?;
            crate::note_media_intent::merge_detachments(&mut detachments, &replayed)
                .map_err(StoreError::Integrity)?;
            let normalized = crate::note_media_intent::normalize_legacy_media_tombstones(
                &before,
                &incoming,
                &mut proposed,
                &detachments,
                policy.media_deletions(),
            )
            .map_err(StoreError::Integrity)?;
            if !normalized.unresolved_ids.is_empty() {
                return Err(StoreError::Integrity(
                    "attachment deletion ownership is unresolved; no changes committed".into(),
                ));
            }
            media_candidates = media_privacy::Candidates::from_ids(&normalized.cleanup_candidates)?;
        }
        let policy = policy
            .including_detachments(&detachments)
            .map_err(|error| StoreError::Integrity(error.to_string()))?;
        let proposed_json = serde_json::to_string(&proposed)?;
        if note_privacy::project(&policy, &proposed_json)?.is_some() {
            return Err(StoreError::Integrity(
                "incoming account crosses a retained note privacy barrier".into(),
            ));
        }
        let projected = note_privacy::project_current(&policy, &proposed_json)?;
        let committed_json = projected.as_deref().unwrap_or(&proposed_json);
        // Preserve exact bytes on unchanged legacy calls to avoid spurious account revisions.
        let app_data_json = if proposed == original_proposed && projected.is_none() {
            app_data_json
        } else {
            committed_json
        };
        validate_app_data_growth_quota(&current.app_data_json, app_data_json)?;
        let account_revision = if current.app_data_json == app_data_json {
            current.revision
        } else {
            validate_app_data_media_transition(&current.app_data_json, app_data_json)?;
            let revision = next_account_revision(expected_revision)?;
            insert_snapshot_history_for_account_mutation(
                &transaction,
                &self.database_path,
                &current,
                app_data_json,
                now_epoch_millis,
            )?;
            validate_app_data_media_identity_quota(&transaction, user_id, app_data_json)?;
            validate_prospective_current_archivability(
                &transaction,
                user_id,
                revision,
                app_data_json,
                now_epoch_millis,
            )?;
            ensure_snapshot_write_capacity(&transaction, app_data_json.len() as u64)?;
            let restore_generation = restore_generation_in_transaction(&transaction, user_id)?;
            let content_sha256 = sha256_hex(app_data_json.as_bytes());
            let envelope_sha256 = account_snapshot_envelope_sha256(
                user_id,
                app_data_json,
                revision,
                now_epoch_millis,
                restore_generation,
            );
            let changed = transaction.execute(
                "UPDATE account_snapshots \
                 SET app_data_json = ?1, revision = ?2, updated_at_epoch_millis = ?3, \
                     content_sha256 = ?4, envelope_sha256 = ?5 \
                 WHERE user_id = ?6 AND revision = ?7",
                params![
                    app_data_json,
                    revision,
                    now_epoch_millis,
                    content_sha256,
                    envelope_sha256,
                    user_id,
                    expected_revision
                ],
            )?;
            if changed != 1 {
                return Err(StoreError::Integrity(
                    "sync CAS changed an unexpected row count".to_string(),
                ));
            }
            replace_current_snapshot_media_identities(&transaction, user_id, app_data_json)?;
            transaction.execute(
                "UPDATE users SET updated_at_epoch_millis = ?1 WHERE id = ?2",
                params![now_epoch_millis, user_id],
            )?;
            revision
        };
        let privacy_policy = note_privacy::enforce_with_media_intents(
            &transaction,
            user_id,
            &detachments,
            media_candidates,
        )?;
        let mut committed_response: serde_json::Value = serde_json::from_str(response_json)?;
        if incoming_raw_app_data_json.is_some() && committed_response["appDataJson"].is_string() {
            committed_response["appDataJson"] = serde_json::Value::String(app_data_json.to_owned());
            if committed_response.get("appDataProjectedBytes").is_some() {
                committed_response["appDataProjectedBytes"] =
                    serde_json::json!(app_data_json.len());
            }
        }
        let committed_response = if incoming_raw_app_data_json.is_some() {
            serde_json::to_string(&committed_response)?
        } else {
            response_json.to_owned()
        };
        let sanitized_response =
            note_privacy::redact_response(&privacy_policy, &committed_response)?;
        let response_json = sanitized_response.as_deref().unwrap_or(&committed_response);
        let refreshed_response = if incoming_raw_app_data_json.is_some() {
            crate::sync_core::refresh_committed_sync_response(response_json, app_data_json)
                .map_err(StoreError::Integrity)?
        } else {
            response_json.to_owned()
        };
        let response_json = refreshed_response.as_str();
        if response_json.len() > REQUEST_DEDUP_MAX_RESPONSE_BYTES {
            return Err(StoreError::Integrity(
                "committed sync response exceeds deduplication limit".into(),
            ));
        }
        transaction.execute(
            "INSERT INTO request_dedup(
                 request_id, user_id, request_fingerprint, response_json, account_revision,
                 created_at_epoch_millis
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                request_id,
                user_id,
                request_fingerprint,
                response_json,
                account_revision,
                now_epoch_millis
            ],
        )?;
        transaction.execute(
            "DELETE FROM request_dedup \
             WHERE user_id = ?1 \
               AND request_id NOT IN ( \
                   SELECT request_id FROM request_dedup \
                   WHERE user_id = ?1 \
                   ORDER BY created_at_epoch_millis DESC, rowid DESC \
                   LIMIT ?2 \
               )",
            params![user_id, max_receipts_per_user],
        )?;
        prune_request_dedup_bytes_for_user(&transaction, user_id, max_response_bytes_per_user)?;
        activate_pending_token_in_transaction(&transaction, user_id, pending_activation)?;
        privacy_journal::commit(transaction)?;
        Ok(SyncRequestOutcome::Applied(SyncRequestReceipt {
            response_json: response_json.to_string(),
            account_revision,
        }))
    }

    pub fn prune_request_dedup(&self, now_epoch_millis: i64) -> StoreResult<usize> {
        let connection = self.open_connection(false)?;
        connection
            .execute(
                "DELETE FROM request_dedup WHERE created_at_epoch_millis < ?1",
                params![now_epoch_millis.saturating_sub(REQUEST_DEDUP_RETENTION_MILLIS)],
            )
            .map_err(StoreError::from)
    }

    pub fn upsert_media(
        &self,
        user_id: &str,
        attachment_id: &str,
        declared_sha256: &str,
        mime_type: &str,
        declared_size_bytes: i64,
        content: &[u8],
        upload_revision_epoch_millis: i64,
    ) -> StoreResult<MediaUpsertOutcome> {
        self.upsert_media_with_restore(
            user_id,
            attachment_id,
            declared_sha256,
            mime_type,
            declared_size_bytes,
            content,
            upload_revision_epoch_millis,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_media_with_restore(
        &self,
        user_id: &str,
        attachment_id: &str,
        declared_sha256: &str,
        mime_type: &str,
        declared_size_bytes: i64,
        content: &[u8],
        upload_revision_epoch_millis: i64,
        restore_deleted: bool,
    ) -> StoreResult<MediaUpsertOutcome> {
        self.upsert_media_with_restore_and_barrier(
            user_id,
            attachment_id,
            declared_sha256,
            mime_type,
            declared_size_bytes,
            content,
            upload_revision_epoch_millis,
            restore_deleted,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_media_with_restore_for_generation(
        &self,
        user_id: &str,
        token_id: i64,
        acknowledged_generation: i64,
        restore_receipt: &str,
        attachment_id: &str,
        declared_sha256: &str,
        mime_type: &str,
        declared_size_bytes: i64,
        content: &[u8],
        upload_revision_epoch_millis: i64,
        restore_deleted: bool,
    ) -> StoreResult<MediaUpsertOutcome> {
        self.upsert_media_with_restore_and_barrier(
            user_id,
            attachment_id,
            declared_sha256,
            mime_type,
            declared_size_bytes,
            content,
            upload_revision_epoch_millis,
            restore_deleted,
            Some((token_id, acknowledged_generation, restore_receipt, false)),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn upsert_media_with_restore_and_barrier(
        &self,
        user_id: &str,
        attachment_id: &str,
        declared_sha256: &str,
        mime_type: &str,
        declared_size_bytes: i64,
        content: &[u8],
        upload_revision_epoch_millis: i64,
        restore_deleted: bool,
        restore_barrier: Option<(i64, i64, &str, bool)>,
    ) -> StoreResult<MediaUpsertOutcome> {
        if !valid_media_attachment_id(attachment_id) {
            return Err(StoreError::Integrity(
                "media attachment id is invalid".to_string(),
            ));
        }
        if !valid_media_mime_type(mime_type) {
            return Err(StoreError::Integrity(
                "media MIME type is invalid".to_string(),
            ));
        }
        if content.len() > MAX_MEDIA_BYTES {
            return Err(StoreError::Integrity(format!(
                "attachment exceeds the {MAX_MEDIA_BYTES} byte limit"
            )));
        }
        if declared_size_bytes != content.len() as i64 {
            return Err(StoreError::Integrity(
                "attachment size does not match its content".to_string(),
            ));
        }
        let computed_sha256 = sha256_hex(content);
        if !computed_sha256.eq_ignore_ascii_case(declared_sha256.trim()) {
            return Err(StoreError::Integrity(
                "attachment SHA-256 does not match its content".to_string(),
            ));
        }
        if upload_revision_epoch_millis <= 0 {
            return Err(StoreError::Integrity(
                "media upload revision must be positive".to_string(),
            ));
        }
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        acknowledge_restore_barrier_in_transaction(&transaction, user_id, restore_barrier)?;
        validate_media_identity_quota(&transaction, user_id, attachment_id)?;
        let existing = transaction
            .query_row(
                "SELECT user_id, attachment_id, sha256, mime_type, size_bytes,
                        updated_at_epoch_millis, deleted_at_epoch_millis
                 FROM note_media \
                 WHERE user_id = ?1 AND attachment_id = ?2",
                params![user_id, attachment_id],
                media_metadata_from_row,
            )
            .optional()?;
        let table_tombstone_revision =
            media_tombstone_revision(&transaction, user_id, attachment_id)?;
        let tombstone_revision = max_optional_revision(
            existing
                .as_ref()
                .and_then(|metadata| metadata.deleted_at_epoch_millis),
            table_tombstone_revision,
        );
        let existing = existing
            .map(|metadata| media_metadata_with_effective_tombstone(metadata, tombstone_revision));
        if let Some(metadata) = existing.as_ref() {
            if metadata.deleted_at_epoch_millis.is_none()
                && upload_revision_epoch_millis <= metadata.updated_at_epoch_millis
            {
                if !metadata.sha256.eq_ignore_ascii_case(&computed_sha256)
                    || metadata.size_bytes != declared_size_bytes
                {
                    return Err(StoreError::Integrity(
                        "attachment id already belongs to different content".to_string(),
                    ));
                }
                let stored_content = transaction.query_row(
                    "SELECT content FROM note_media
                     WHERE user_id = ?1 AND attachment_id = ?2",
                    params![user_id, attachment_id],
                    |row| row.get::<_, Vec<u8>>(0),
                )?;
                ensure_media_write_capacity(&transaction, content.len() as u64)?;
                if !stored_blob_matches_sha256(
                    &metadata.sha256,
                    metadata.size_bytes,
                    &stored_content,
                ) {
                    let changed = transaction.execute(
                        "UPDATE note_media SET content = ?1
                         WHERE user_id = ?2 AND attachment_id = ?3
                           AND lower(sha256) = lower(?4) AND size_bytes = ?5",
                        params![
                            content,
                            user_id,
                            attachment_id,
                            computed_sha256,
                            declared_size_bytes
                        ],
                    )?;
                    if changed != 1 {
                        return Err(StoreError::Integrity(
                            "corrupt attachment could not be repaired atomically".to_string(),
                        ));
                    }
                }
                repair_missing_media_history_entries(
                    &transaction,
                    user_id,
                    attachment_id,
                    &computed_sha256,
                    mime_type,
                    declared_size_bytes,
                    content,
                    system_time_epoch_millis(),
                )?;
                privacy_journal::commit(transaction)?;
                return Ok(MediaUpsertOutcome::Stored(metadata.clone()));
            }
        }
        let deletion_is_authoritative = existing
            .as_ref()
            .map(|metadata| metadata.deleted_at_epoch_millis.is_some())
            .unwrap_or_else(|| tombstone_revision.is_some());
        if deletion_is_authoritative
            && (!restore_deleted
                || tombstone_revision.is_some_and(|deleted_revision| {
                    upload_revision_epoch_millis <= deleted_revision
                }))
        {
            let metadata = existing.unwrap_or_else(|| {
                media_tombstone_only_metadata(
                    user_id,
                    attachment_id,
                    tombstone_revision.expect("checked tombstone"),
                )
            });
            privacy_journal::commit(transaction)?;
            return Ok(MediaUpsertOutcome::RejectedByTombstone(metadata));
        }
        if existing.as_ref().is_some_and(|metadata| {
            !metadata.sha256.eq_ignore_ascii_case(&computed_sha256)
                || metadata.size_bytes != declared_size_bytes
        }) {
            return Err(StoreError::Integrity(
                "attachment id already belongs to different content".to_string(),
            ));
        }
        let (item_count, total_bytes) = transaction.query_row(
            "SELECT COUNT(*), COALESCE(SUM(m.size_bytes), 0)
             FROM note_media m
             LEFT JOIN note_media_tombstones t
               ON t.user_id = m.user_id AND t.attachment_id = m.attachment_id
             WHERE m.user_id = ?1
               AND m.deleted_at_epoch_millis IS NULL
               AND (t.deleted_revision_epoch_millis IS NULL
                    OR m.updated_at_epoch_millis > t.deleted_revision_epoch_millis)",
            params![user_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )?;
        let existing_active_size = existing
            .as_ref()
            .filter(|metadata| metadata.deleted_at_epoch_millis.is_none())
            .map(|metadata| metadata.size_bytes);
        let retained_bytes = transaction.query_row(
            "SELECT COALESCE(SUM(size_bytes), 0) FROM note_media WHERE user_id = ?1",
            params![user_id],
            |row| row.get::<_, i64>(0),
        )?;
        validate_media_retained_quota(
            retained_bytes,
            existing.as_ref().map(|metadata| metadata.size_bytes),
            declared_size_bytes,
        )?;
        validate_media_server_retained_growth(
            &transaction,
            existing.as_ref().map(|metadata| metadata.size_bytes),
            declared_size_bytes,
        )?;
        validate_media_quota(
            item_count,
            total_bytes,
            existing_active_size,
            declared_size_bytes,
        )?;
        ensure_media_write_capacity(&transaction, content.len() as u64)?;
        transaction.execute(
            "INSERT INTO note_media(
                 user_id, attachment_id, sha256, mime_type, size_bytes, content,
                 updated_at_epoch_millis, deleted_at_epoch_millis
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)
             ON CONFLICT(user_id, attachment_id) DO UPDATE SET
                 sha256 = excluded.sha256,
                 mime_type = excluded.mime_type,
                 size_bytes = excluded.size_bytes,
                 content = excluded.content,
                 updated_at_epoch_millis = excluded.updated_at_epoch_millis,
                 deleted_at_epoch_millis = NULL",
            params![
                user_id,
                attachment_id,
                computed_sha256,
                mime_type,
                declared_size_bytes,
                content,
                upload_revision_epoch_millis
            ],
        )?;
        repair_missing_media_history_entries(
            &transaction,
            user_id,
            attachment_id,
            &computed_sha256,
            mime_type,
            declared_size_bytes,
            content,
            system_time_epoch_millis(),
        )?;
        privacy_journal::commit(transaction)?;
        Ok(MediaUpsertOutcome::Stored(MediaMetadata {
            user_id: user_id.to_string(),
            attachment_id: attachment_id.to_string(),
            sha256: computed_sha256,
            mime_type: mime_type.to_string(),
            size_bytes: declared_size_bytes,
            updated_at_epoch_millis: upload_revision_epoch_millis,
            deleted_at_epoch_millis: None,
        }))
    }

    pub fn read_media(
        &self,
        user_id: &str,
        attachment_id: &str,
    ) -> StoreResult<Option<StoredMedia>> {
        let connection = self.open_connection(false)?;
        read_media_from_connection(&connection, user_id, attachment_id)
    }

    /// Acknowledges the caller's restore generation and reads the BLOB from the
    /// same IMMEDIATE transaction. This gives restores and media downloads a
    /// single linearization order instead of leaving a check/read TOCTOU gap.
    pub fn read_media_for_generation(
        &self,
        user_id: &str,
        token_id: i64,
        acknowledged_generation: i64,
        restore_receipt: &str,
        attachment_id: &str,
    ) -> StoreResult<Option<StoredMedia>> {
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        acknowledge_restore_barrier_in_transaction(
            &transaction,
            user_id,
            Some((token_id, acknowledged_generation, restore_receipt, false)),
        )?;
        let stored = read_media_from_connection(&transaction, user_id, attachment_id)?;
        privacy_journal::commit(transaction)?;
        Ok(stored)
    }

    pub fn list_media_metadata(
        &self,
        user_id: &str,
        include_deleted: bool,
    ) -> StoreResult<Vec<MediaMetadata>> {
        let connection = self.open_connection(false)?;
        list_media_metadata_from_connection(&connection, user_id, include_deleted)
    }

    /// Acknowledges the caller's restore generation and builds the complete
    /// live/tombstone manifest in the same IMMEDIATE transaction. Besides the
    /// restore barrier, this keeps the manifest's two underlying queries on one
    /// consistent SQLite snapshot.
    pub fn list_media_metadata_for_generation(
        &self,
        user_id: &str,
        token_id: i64,
        acknowledged_generation: i64,
        restore_receipt: &str,
        include_deleted: bool,
    ) -> StoreResult<Vec<MediaMetadata>> {
        self.media_manifest_for_generation(
            user_id,
            token_id,
            acknowledged_generation,
            restore_receipt,
            include_deleted,
        )
        .map(|(items, _)| items)
    }

    /// Legacy gaps are attested from this account's verified current snapshot
    /// under the same restore barrier and SQLite transaction as the manifest.
    pub fn media_manifest_for_generation(
        &self,
        user_id: &str,
        token_id: i64,
        acknowledged_generation: i64,
        restore_receipt: &str,
        include_deleted: bool,
    ) -> StoreResult<(
        Vec<MediaMetadata>,
        Vec<crate::sync_core::LegacyMediaReference>,
    )> {
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        acknowledge_restore_barrier_in_transaction(
            &transaction,
            user_id,
            Some((token_id, acknowledged_generation, restore_receipt, false)),
        )?;
        let items = list_media_metadata_from_connection(&transaction, user_id, include_deleted)?;
        let account = read_account_in_transaction(&transaction, user_id)?
            .ok_or_else(|| StoreError::NotFound(format!("account for user {user_id}")))?;
        let legacy = legacy_unhashed_media_references(&account.app_data_json)?;
        privacy_journal::commit(transaction)?;
        Ok((items, legacy))
    }

    pub fn delete_media(
        &self,
        user_id: &str,
        attachment_id: &str,
        deleted_revision_epoch_millis: i64,
        recorded_at_epoch_millis: i64,
    ) -> StoreResult<MediaMetadata> {
        self.delete_media_with_barrier(
            user_id,
            attachment_id,
            deleted_revision_epoch_millis,
            recorded_at_epoch_millis,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn delete_media_for_generation(
        &self,
        user_id: &str,
        token_id: i64,
        acknowledged_generation: i64,
        restore_receipt: &str,
        attachment_id: &str,
        deleted_revision_epoch_millis: i64,
        recorded_at_epoch_millis: i64,
    ) -> StoreResult<MediaMetadata> {
        self.delete_media_with_barrier(
            user_id,
            attachment_id,
            deleted_revision_epoch_millis,
            recorded_at_epoch_millis,
            Some((token_id, acknowledged_generation, restore_receipt, false)),
        )
    }

    fn delete_media_with_barrier(
        &self,
        user_id: &str,
        attachment_id: &str,
        deleted_revision_epoch_millis: i64,
        recorded_at_epoch_millis: i64,
        restore_barrier: Option<(i64, i64, &str, bool)>,
    ) -> StoreResult<MediaMetadata> {
        if !valid_media_attachment_id(attachment_id) || deleted_revision_epoch_millis <= 0 {
            return Err(StoreError::Integrity(
                "valid attachment id and positive delete revision are required".to_string(),
            ));
        }
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        acknowledge_restore_barrier_in_transaction(&transaction, user_id, restore_barrier)?;
        validate_media_identity_quota(&transaction, user_id, attachment_id)?;
        let existing = transaction
            .query_row(
                "SELECT user_id, attachment_id, sha256, mime_type, size_bytes,
                        updated_at_epoch_millis, deleted_at_epoch_millis
                 FROM note_media WHERE user_id = ?1 AND attachment_id = ?2",
                params![user_id, attachment_id],
                media_metadata_from_row,
            )
            .optional()?;
        let table_tombstone = media_tombstone_revision(&transaction, user_id, attachment_id)?;
        let creates_unknown_tombstone = existing.is_none() && table_tombstone.is_none();
        let current_tombstone = max_optional_revision(
            existing
                .as_ref()
                .and_then(|metadata| metadata.deleted_at_epoch_millis),
            table_tombstone,
        );
        let current = existing
            .map(|metadata| media_metadata_with_effective_tombstone(metadata, current_tombstone));
        if current.as_ref().is_some_and(|metadata| {
            metadata.deleted_at_epoch_millis.is_none()
                && deleted_revision_epoch_millis < metadata.updated_at_epoch_millis
        }) {
            privacy_journal::commit(transaction)?;
            return Ok(current.expect("checked current media"));
        }
        // Replays may follow an unsafe tombstone written by an older binary.
        // Keep the reference check and the mutation in this IMMEDIATE lock.
        media_privacy::ordinary_deletion_scope(&transaction, user_id, false)?
            .allow_delete(attachment_id)?;
        if current_tombstone.is_some_and(|revision| revision >= deleted_revision_epoch_millis) {
            let metadata = current.unwrap_or_else(|| {
                media_tombstone_only_metadata(
                    user_id,
                    attachment_id,
                    current_tombstone.expect("checked tombstone"),
                )
            });
            privacy_journal::commit(transaction)?;
            return Ok(metadata);
        }
        if creates_unknown_tombstone {
            ensure_media_write_capacity(&transaction, MEDIA_TOMBSTONE_WRITE_ESTIMATE_BYTES)?;
        }
        transaction.execute(
            "INSERT INTO note_media_tombstones(
                 user_id, attachment_id, deleted_revision_epoch_millis,
                 recorded_at_epoch_millis
             ) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(user_id, attachment_id) DO UPDATE SET
                 deleted_revision_epoch_millis = excluded.deleted_revision_epoch_millis,
                 recorded_at_epoch_millis = excluded.recorded_at_epoch_millis
             WHERE excluded.deleted_revision_epoch_millis
                   > note_media_tombstones.deleted_revision_epoch_millis",
            params![
                user_id,
                attachment_id,
                deleted_revision_epoch_millis,
                recorded_at_epoch_millis
            ],
        )?;
        transaction.execute(
            "UPDATE note_media
             SET deleted_at_epoch_millis = ?1
             WHERE user_id = ?2 AND attachment_id = ?3
               AND updated_at_epoch_millis <= ?1",
            params![deleted_revision_epoch_millis, user_id, attachment_id],
        )?;
        let metadata = current
            .map(|mut metadata| {
                metadata.deleted_at_epoch_millis = Some(deleted_revision_epoch_millis);
                metadata
            })
            .unwrap_or_else(|| {
                media_tombstone_only_metadata(user_id, attachment_id, deleted_revision_epoch_millis)
            });
        privacy_journal::commit(transaction)?;
        Ok(metadata)
    }

    pub fn media_usage_bytes(&self, user_id: &str, include_deleted: bool) -> StoreResult<i64> {
        let connection = self.open_connection(false)?;
        let query = if include_deleted {
            "SELECT COALESCE(SUM(size_bytes), 0) FROM note_media WHERE user_id = ?1"
        } else {
            "SELECT COALESCE(SUM(m.size_bytes), 0)
             FROM note_media m
             LEFT JOIN note_media_tombstones t
               ON t.user_id = m.user_id AND t.attachment_id = m.attachment_id
             WHERE m.user_id = ?1
               AND m.deleted_at_epoch_millis IS NULL
               AND (t.deleted_revision_epoch_millis IS NULL
                    OR m.updated_at_epoch_millis > t.deleted_revision_epoch_millis)"
        };
        connection
            .query_row(query, params![user_id], |row| row.get(0))
            .map_err(StoreError::from)
    }

    /// Deletes retained BLOBs after the undo window while preserving their tiny
    /// revision tombstones so an offline stale device still cannot resurrect them.
    pub fn prune_deleted_media_content(&self, now_epoch_millis: i64) -> StoreResult<usize> {
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let cutoff = now_epoch_millis.saturating_sub(DELETED_MEDIA_CONTENT_RETENTION_MILLIS);
        let deleted = media_privacy::prune_retained_media(&transaction, cutoff)?;
        privacy_journal::commit(transaction)?;
        Ok(deleted)
    }

    /// Exports the old JSON shape without bearer tokens. Password hashes and
    /// account snapshots remain compatible, so users can sign in again; active
    /// sessions intentionally require reauthentication because raw tokens are
    /// never recoverable from their fingerprints.
    pub fn export_compatible_legacy_json(&self) -> StoreResult<String> {
        let connection = self.open_connection(false)?;
        let mut statement = connection.prepare(
            "SELECT u.id, u.email, u.password_salt, u.password_hash, \
                    u.created_at_epoch_millis, u.updated_at_epoch_millis, s.app_data_json \
             FROM users u JOIN account_snapshots s ON s.user_id = u.id \
             ORDER BY u.created_at_epoch_millis, u.id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(LegacyServerUser {
                id: row.get(0)?,
                email: row.get(1)?,
                password_salt: row.get(2)?,
                password_hash: row.get(3)?,
                created_at_epoch_millis: row.get(4)?,
                updated_at_epoch_millis: row.get(5)?,
                app_data_json: row.get(6)?,
                tokens: Vec::new(),
            })
        })?;
        let users = rows.collect::<Result<Vec<_>, _>>()?;
        serde_json::to_string_pretty(&LegacyServerStore { users }).map_err(StoreError::from)
    }

    pub fn stats(&self) -> StoreResult<StoreStats> {
        let connection = self.open_connection(false)?;
        Ok(StoreStats {
            users: connection.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))?,
            tokens: connection.query_row("SELECT COUNT(*) FROM tokens", [], |row| row.get(0))?,
            snapshots: connection.query_row(
                "SELECT COUNT(*) FROM account_snapshots",
                [],
                |row| row.get(0),
            )?,
        })
    }

    /// Creates a consistent online backup through SQLite's backup API. The
    /// destination is published only after full integrity verification.
    pub fn create_verified_backup(
        &self,
        destination: impl Into<PathBuf>,
        now_epoch_millis: i64,
    ) -> StoreResult<VerifiedBackupReport> {
        let destination = destination.into();
        self.finish_note_privacy_cleanup()?;
        let source = self.open_connection(false)?;
        create_verified_sqlite_backup(
            &source,
            &self.database_path,
            &destination,
            now_epoch_millis,
            Some(SCHEMA_VERSION),
            None,
        )
    }

    pub fn verify_existing_backup(
        backup_path: impl Into<PathBuf>,
        created_at_epoch_millis: i64,
    ) -> StoreResult<VerifiedBackupReport> {
        let backup_path = backup_path.into();
        backup_verification::verify(
            &backup_path,
            created_at_epoch_millis,
            backup_verification::Domain::Recovery,
            || {
                let connection = Connection::open_with_flags(
                    &backup_path,
                    OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
                )?;
                connection.busy_timeout(DEFAULT_BUSY_TIMEOUT)?;
                connection.pragma_update(None, "foreign_keys", "ON")?;
                connection.pragma_update(None, "trusted_schema", "OFF")?;
                verify_quick_check(&connection)?;
                verify_integrity_check(&connection)?;
                let version = current_schema_version(&connection)?;
                if !matches!(version, 14..=17) {
                    return Err(StoreError::Integrity(format!(
                        "unsupported recovery backup schema version {version}"
                    )));
                }
                // The immediately previous release has the same base layout. Verify
                // it without changing its bytes so manifest hashes remain usable; the
                // normal open path performs migration only after a verified copy.
                verify_required_schema_at_version(&connection, version)?;
                verify_semantic_storage_integrity_with_privacy(&connection, version >= 15)?;
                verify_foreign_keys(&connection)?;
                let server_instance_id = connection.query_row(
                    "SELECT server_instance_id FROM server_identity WHERE singleton = 1",
                    [],
                    |row| row.get::<_, String>(0),
                )?;
                drop(connection);
                Ok(VerifiedBackupReport {
                    size_bytes: fs::metadata(&backup_path)?.len(),
                    sha256: sha256_file(&backup_path)?,
                    destination: backup_path.clone(),
                    created_at_epoch_millis,
                    server_instance_id,
                })
            },
        )
    }

    fn open_connection(&self, create: bool) -> StoreResult<Connection> {
        let mut flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_FULL_MUTEX;
        if create {
            flags |= OpenFlags::SQLITE_OPEN_CREATE;
        }
        let connection = Connection::open_with_flags(&self.database_path, flags)?;
        configure_connection(&connection)?;
        Ok(connection)
    }
}

pub fn token_fingerprint(raw_token: &str) -> String {
    sha256_hex(raw_token.as_bytes())
}

pub fn token_identifier(raw_token: &str) -> String {
    let hash = token_fingerprint(raw_token);
    hash[..32].to_string()
}

fn token_id_from_hash(hash: &str) -> StoreResult<String> {
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(StoreError::Integrity(
            "invalid token hash encoding".to_string(),
        ));
    }
    Ok(hash[..32].to_ascii_lowercase())
}

fn decode_sha256_hex(hash: &str) -> StoreResult<[u8; 32]> {
    if hash.len() != 64 {
        return Err(StoreError::Integrity(
            "invalid token hash length".to_string(),
        ));
    }
    let mut decoded = [0_u8; 32];
    for (index, byte) in decoded.iter_mut().enumerate() {
        let offset = index * 2;
        *byte = u8::from_str_radix(&hash[offset..offset + 2], 16)
            .map_err(|_| StoreError::Integrity("invalid token hash encoding".to_string()))?;
    }
    Ok(decoded)
}

fn configure_connection(connection: &Connection) -> StoreResult<()> {
    connection.busy_timeout(DEFAULT_BUSY_TIMEOUT)?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.pragma_update(None, "wal_autocheckpoint", 1_000)?;
    connection.pragma_update(None, "trusted_schema", "OFF")?;
    connection.pragma_update(None, "secure_delete", "ON")?;
    verify_connection_durability_settings(connection)?;
    Ok(())
}

fn verify_connection_durability_settings(connection: &Connection) -> StoreResult<()> {
    let secure_delete =
        connection.pragma_query_value(None, "secure_delete", |row| row.get::<_, i64>(0))?;
    if secure_delete != 1 {
        return Err(StoreError::Integrity(
            "SQLite secure_delete was not enabled".to_string(),
        ));
    }
    let foreign_keys =
        connection.pragma_query_value(None, "foreign_keys", |row| row.get::<_, i64>(0))?;
    if foreign_keys != 1 {
        return Err(StoreError::Integrity(format!(
            "SQLite foreign_keys pragma was not enabled (actual {foreign_keys})"
        )));
    }

    let journal_mode =
        connection.pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(StoreError::Integrity(format!(
            "SQLite journal_mode WAL was not applied (actual {journal_mode})"
        )));
    }

    let synchronous =
        connection.pragma_query_value(None, "synchronous", |row| row.get::<_, i64>(0))?;
    if synchronous != 2 {
        return Err(StoreError::Integrity(format!(
            "SQLite synchronous FULL was not applied (actual {synchronous})"
        )));
    }
    Ok(())
}

fn current_schema_version(connection: &Connection) -> StoreResult<i64> {
    let header_version =
        connection.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))?;
    let has_migration_table = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !has_migration_table {
        if header_version != 0 {
            return Err(StoreError::Integrity(
                "database schema header has no matching migration history".to_string(),
            ));
        }
        return Ok(0);
    }
    let migration_version: i64 = connection
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )
        .map_err(StoreError::from)?;
    if migration_version != header_version {
        return Err(StoreError::Integrity(format!(
            "database schema header {header_version} disagrees with migration history {migration_version}"
        )));
    }
    Ok(migration_version)
}

fn table_has_column(
    connection: &Connection,
    table_name: &str,
    column_name: &str,
) -> StoreResult<bool> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table_name})"))?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        if row.get::<_, String>(1)?.eq_ignore_ascii_case(column_name) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn cleanup_expired_pending_tokens_in_connection(
    connection: &mut Connection,
    now_epoch_millis: i64,
) -> StoreResult<usize> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let revoked = transaction.execute(
        "UPDATE tokens
         SET revoked_at_epoch_millis = expires_at_epoch_millis
         WHERE activation_state = 0
           AND revoked_at_epoch_millis IS NULL
           AND expires_at_epoch_millis <= ?1",
        params![now_epoch_millis],
    )?;
    let deleted = transaction.execute(
        "DELETE FROM tokens
         WHERE activation_state = 0
           AND revoked_at_epoch_millis IS NOT NULL
           AND expires_at_epoch_millis <= ?1
           AND NOT (
               activated_at_epoch_millis = 0
               AND restore_acknowledged = 1
               AND last_seen_restore_generation = 0
               AND pending_restore_generation IS NULL
               AND pending_restore_receipt = ''
               AND last_seen_at_epoch_millis = created_at_epoch_millis
               AND expires_at_epoch_millis = created_at_epoch_millis + ?2
               AND EXISTS(
                   SELECT 1 FROM users u
                   JOIN account_snapshots s ON s.user_id = u.id
                   WHERE u.id = tokens.user_id
                     AND u.created_at_epoch_millis = tokens.created_at_epoch_millis
                     AND u.updated_at_epoch_millis = u.created_at_epoch_millis
                     AND u.password_scheme = 'argon2id_phc'
                     AND u.password_salt = ''
                     AND s.revision = 0
                     AND trim(s.app_data_json) = ''
                     AND s.restore_generation = 0
               )
           )",
        params![
            now_epoch_millis.saturating_sub(PENDING_TOKEN_TOMBSTONE_RETENTION_MILLIS),
            INITIAL_REGISTRATION_ACTIVATION_LEASE_MILLIS,
        ],
    )?;
    transaction.commit()?;
    Ok(revoked.saturating_add(deleted))
}

fn activate_pending_token_in_transaction(
    transaction: &Transaction<'_>,
    expected_user_id: &str,
    activation: Option<PendingTokenActivation>,
) -> StoreResult<()> {
    let Some(activation) = activation else {
        return Ok(());
    };
    if activation.active_expires_at_epoch_millis <= activation.activated_at_epoch_millis {
        return Err(StoreError::Integrity(
            "activated token expiry must be after activation".to_string(),
        ));
    }
    let token = transaction
        .query_row(
            "SELECT user_id, created_at_epoch_millis, expires_at_epoch_millis,
                    revoked_at_epoch_millis, activation_state
             FROM tokens WHERE id = ?1",
            params![activation.token_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((user_id, created_at, current_expiry, revoked_at, activation_state)) = token else {
        return Err(StoreError::Integrity(
            "pending sync token disappeared before commit".to_string(),
        ));
    };
    if user_id != expected_user_id {
        return Err(StoreError::Integrity(
            "pending sync token belongs to a different user".to_string(),
        ));
    }
    if activation_state == 1 {
        if revoked_at.is_some() {
            return Err(StoreError::Integrity(
                "activated sync token was revoked before replay".to_string(),
            ));
        }
        if current_expiry <= activation.activated_at_epoch_millis {
            return Err(StoreError::Integrity(
                "activated sync token expired before replay".to_string(),
            ));
        }
        // A replay can observe the token already activated by the original
        // committed request. Do not rewrite or extend its formal expiry.
        return Ok(());
    }
    if activation_state != 0 {
        return Err(StoreError::Integrity(format!(
            "token {} has an invalid activation state",
            activation.token_id
        )));
    }
    if activation.active_expires_at_epoch_millis <= created_at {
        return Err(StoreError::Integrity(
            "activated token expiry must be after token creation".to_string(),
        ));
    }
    let changed =
        if let Some(original_expiry) = activation.cleanup_recovery_original_expiry_epoch_millis {
            if activation.cleanup_recovery_original_lease_millis <= 0
                || activation.cleanup_recovery_window_millis <= 0
            {
                return Err(StoreError::Integrity(
                    "cleanup recovery activation windows must be positive".to_string(),
                ));
            }
            let cleanup_marker_valid = current_expiry == original_expiry
                && current_expiry <= activation.activated_at_epoch_millis
                && current_expiry
                    == created_at.saturating_add(activation.cleanup_recovery_original_lease_millis)
                && (revoked_at.is_none() || revoked_at == Some(current_expiry))
                && activation.activated_at_epoch_millis
                    <= current_expiry.saturating_add(activation.cleanup_recovery_window_millis);
            if !cleanup_marker_valid {
                return Err(StoreError::Integrity(
                    "expired pending sync token is not eligible for cleanup recovery".to_string(),
                ));
            }
            transaction.execute(
                "UPDATE tokens
             SET activation_state = 1,
                 activated_at_epoch_millis = ?1,
                 expires_at_epoch_millis = ?2,
                 last_seen_at_epoch_millis = ?1,
                 revoked_at_epoch_millis = NULL
             WHERE id = ?3
               AND user_id = ?4
               AND activation_state = 0
               AND created_at_epoch_millis = ?5
               AND expires_at_epoch_millis = ?6
               AND (revoked_at_epoch_millis IS NULL OR revoked_at_epoch_millis = ?6)",
                params![
                    activation.activated_at_epoch_millis,
                    activation.active_expires_at_epoch_millis,
                    activation.token_id,
                    expected_user_id,
                    created_at,
                    original_expiry
                ],
            )?
        } else {
            if activation.cleanup_recovery_original_lease_millis != 0
                || activation.cleanup_recovery_window_millis != 0
            {
                return Err(StoreError::Integrity(
                    "live pending activation carried cleanup recovery metadata".to_string(),
                ));
            }
            if revoked_at.is_some() {
                return Err(StoreError::Integrity(
                    "pending sync token was revoked before commit".to_string(),
                ));
            }
            if current_expiry <= activation.activated_at_epoch_millis {
                return Err(StoreError::Integrity(
                    "pending sync token expired before commit".to_string(),
                ));
            }
            transaction.execute(
                "UPDATE tokens
             SET activation_state = 1,
                 activated_at_epoch_millis = ?1,
                 expires_at_epoch_millis = ?2,
                 last_seen_at_epoch_millis = ?1
             WHERE id = ?3
               AND user_id = ?4
               AND activation_state = 0
               AND revoked_at_epoch_millis IS NULL
               AND expires_at_epoch_millis > ?1",
                params![
                    activation.activated_at_epoch_millis,
                    activation.active_expires_at_epoch_millis,
                    activation.token_id,
                    expected_user_id
                ],
            )?
        };
    if changed != 1 {
        return Err(StoreError::Integrity(
            "pending sync token activation changed an unexpected row count".to_string(),
        ));
    }
    Ok(())
}

fn apply_schema_migrations(
    connection: &mut Connection,
    now_epoch_millis: i64,
    legacy_snapshot_repair_manifest: Option<&VerifiedPreSchemaSnapshotManifest>,
) -> StoreResult<bool> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             version INTEGER PRIMARY KEY,
             description TEXT NOT NULL,
             applied_at_epoch_millis INTEGER NOT NULL
         ) STRICT;",
    )?;
    let current_version = current_schema_version(connection)?;
    if current_version > SCHEMA_VERSION {
        return Err(StoreError::Integrity(format!(
            "database schema version {current_version} is newer than supported {SCHEMA_VERSION}"
        )));
    }
    let legacy_snapshot_repair_manifest = match (current_version, legacy_snapshot_repair_manifest) {
        (12 | 13, Some(manifest)) if manifest.backup.schema_version == current_version => {
            Some(manifest)
        }
        (12 | 13, Some(manifest)) => {
            return Err(StoreError::Integrity(format!(
                "snapshot repair backup schema v{} does not match live schema v{current_version}",
                manifest.backup.schema_version
            )));
        }
        (12 | 13, None) => {
            return Err(StoreError::Integrity(format!(
                "schema v{current_version} migration requires a verified snapshot repair manifest"
            )));
        }
        (_, Some(_)) => {
            return Err(StoreError::Integrity(format!(
                "snapshot repair manifest is only valid for a direct schema v12 or v13 to v14 migration, found schema v{current_version}"
            )));
        }
        (_, None) => None,
    };
    if current_version < 1 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(
            "CREATE TABLE users (
                 id TEXT PRIMARY KEY NOT NULL,
                 email TEXT NOT NULL COLLATE NOCASE UNIQUE,
                 password_salt TEXT NOT NULL,
                 password_hash TEXT NOT NULL,
                 password_scheme TEXT NOT NULL,
                 created_at_epoch_millis INTEGER NOT NULL,
                 updated_at_epoch_millis INTEGER NOT NULL
             ) STRICT;

             CREATE TABLE account_snapshots (
                 user_id TEXT PRIMARY KEY NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                 app_data_json TEXT NOT NULL DEFAULT '',
                 revision INTEGER NOT NULL DEFAULT 0 CHECK(revision >= 0),
                 updated_at_epoch_millis INTEGER NOT NULL,
                 CHECK(length(trim(app_data_json)) = 0 OR json_valid(app_data_json))
             ) STRICT;

             CREATE TABLE account_snapshot_history (
                 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                 revision INTEGER NOT NULL CHECK(revision >= 0),
                 app_data_json TEXT NOT NULL,
                 created_at_epoch_millis INTEGER NOT NULL,
                 sha256 TEXT NOT NULL CHECK(length(sha256) = 64),
                 PRIMARY KEY(user_id, revision),
                 CHECK(length(trim(app_data_json)) = 0 OR json_valid(app_data_json))
             ) STRICT;

             CREATE INDEX account_snapshot_history_created_index
                 ON account_snapshot_history(user_id, created_at_epoch_millis DESC);

             CREATE TABLE tokens (
                 id INTEGER PRIMARY KEY,
                 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                 token_id TEXT NOT NULL UNIQUE CHECK(length(token_id) = 32),
                 token_hash TEXT NOT NULL UNIQUE CHECK(length(token_hash) = 64),
                 device_name TEXT NOT NULL,
                 created_at_epoch_millis INTEGER NOT NULL,
                 last_seen_at_epoch_millis INTEGER NOT NULL,
                 expires_at_epoch_millis INTEGER NOT NULL,
                 revoked_at_epoch_millis INTEGER,
                 CHECK(expires_at_epoch_millis > created_at_epoch_millis)
             ) STRICT;

             CREATE INDEX tokens_user_id_index ON tokens(user_id);
             CREATE INDEX tokens_expiry_index ON tokens(expires_at_epoch_millis)
                 WHERE revoked_at_epoch_millis IS NULL;

             CREATE TABLE legacy_imports (
                 content_sha256 TEXT PRIMARY KEY NOT NULL CHECK(length(content_sha256) = 64),
                 source_path TEXT NOT NULL,
                 backup_path TEXT NOT NULL,
                 imported_at_epoch_millis INTEGER NOT NULL,
                 users_imported INTEGER NOT NULL CHECK(users_imported >= 0),
                 tokens_imported INTEGER NOT NULL CHECK(tokens_imported >= 0)
             ) STRICT;

             CREATE TABLE request_dedup (
                 request_id TEXT PRIMARY KEY NOT NULL,
                 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                 response_json TEXT NOT NULL CHECK(json_valid(response_json)),
                 account_revision INTEGER NOT NULL CHECK(account_revision >= 0),
                 created_at_epoch_millis INTEGER NOT NULL
             ) STRICT;

             CREATE INDEX request_dedup_created_index
                 ON request_dedup(created_at_epoch_millis);

             CREATE TABLE note_media (
                 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                 attachment_id TEXT NOT NULL,
                 sha256 TEXT NOT NULL CHECK(length(sha256) = 64),
                 mime_type TEXT NOT NULL,
                 size_bytes INTEGER NOT NULL CHECK(size_bytes >= 0 AND size_bytes <= 16777216),
                 content BLOB NOT NULL,
                 updated_at_epoch_millis INTEGER NOT NULL,
                 deleted_at_epoch_millis INTEGER,
                 PRIMARY KEY(user_id, attachment_id),
                 CHECK(length(content) = size_bytes)
             ) STRICT;

             CREATE INDEX note_media_active_index
                 ON note_media(user_id, updated_at_epoch_millis DESC)
                 WHERE deleted_at_epoch_millis IS NULL;",
        )?;
        transaction.execute(
            "INSERT INTO schema_migrations(version, description, applied_at_epoch_millis)
             VALUES (1, 'initial transactional sync store', ?1)",
            params![now_epoch_millis],
        )?;
        transaction.pragma_update(None, "user_version", 1)?;
        transaction.commit()?;
    }
    if current_version < 2 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(
            "ALTER TABLE request_dedup
                 ADD COLUMN request_fingerprint TEXT NOT NULL
                 DEFAULT '0000000000000000000000000000000000000000000000000000000000000000'
                 CHECK(length(request_fingerprint) = 64);

             DELETE FROM request_dedup;

             CREATE TABLE legacy_snapshot_bindings (
                 binding_key TEXT PRIMARY KEY NOT NULL,
                 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
                 content_sha256 TEXT NOT NULL CHECK(length(content_sha256) = 64),
                 source_path TEXT NOT NULL,
                 bound_at_epoch_millis INTEGER NOT NULL
             ) STRICT;

             CREATE INDEX legacy_snapshot_bindings_user_index
                 ON legacy_snapshot_bindings(user_id);",
        )?;
        transaction.execute(
            "INSERT INTO schema_migrations(version, description, applied_at_epoch_millis)
             VALUES (2, 'bind legacy snapshots and fingerprint request replay', ?1)",
            params![now_epoch_millis],
        )?;
        transaction.pragma_update(None, "user_version", 2)?;
        transaction.commit()?;
    }
    if current_version < 3 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(
            "CREATE TABLE note_media_tombstones (
                 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                 attachment_id TEXT NOT NULL,
                 deleted_revision_epoch_millis INTEGER NOT NULL
                     CHECK(deleted_revision_epoch_millis > 0),
                 recorded_at_epoch_millis INTEGER NOT NULL,
                 PRIMARY KEY(user_id, attachment_id)
             ) STRICT;

             CREATE INDEX note_media_tombstones_recorded_index
                 ON note_media_tombstones(recorded_at_epoch_millis);",
        )?;
        transaction.execute(
            "INSERT INTO note_media_tombstones(
                 user_id, attachment_id, deleted_revision_epoch_millis,
                 recorded_at_epoch_millis
             )
             SELECT user_id, attachment_id, deleted_at_epoch_millis, ?1
             FROM note_media
             WHERE deleted_at_epoch_millis IS NOT NULL",
            params![now_epoch_millis],
        )?;
        transaction.execute(
            "INSERT INTO schema_migrations(version, description, applied_at_epoch_millis)
             VALUES (3, 'durable revisioned media deletion tombstones', ?1)",
            params![now_epoch_millis],
        )?;
        transaction.pragma_update(None, "user_version", 3)?;
        transaction.commit()?;
    }
    if current_version < 4 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(
            "CREATE TABLE snapshot_contents (
                 sha256 TEXT PRIMARY KEY NOT NULL CHECK(length(sha256) = 64),
                 compression TEXT NOT NULL CHECK(compression = 'zlib'),
                 uncompressed_size_bytes INTEGER NOT NULL CHECK(uncompressed_size_bytes >= 0),
                 compressed_size_bytes INTEGER NOT NULL CHECK(compressed_size_bytes >= 0),
                 content BLOB NOT NULL,
                 reference_count INTEGER NOT NULL DEFAULT 0 CHECK(reference_count >= 0),
                 created_at_epoch_millis INTEGER NOT NULL,
                 CHECK(length(content) = compressed_size_bytes)
             ) STRICT;

             CREATE TABLE account_snapshot_history_v4 (
                 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                 revision INTEGER NOT NULL CHECK(revision >= 0),
                 content_sha256 TEXT NOT NULL REFERENCES snapshot_contents(sha256) ON DELETE RESTRICT,
                 created_at_epoch_millis INTEGER NOT NULL,
                 PRIMARY KEY(user_id, revision)
             ) STRICT;

             CREATE TABLE snapshot_history_prune_audit (
                 id INTEGER PRIMARY KEY,
                 user_id TEXT NOT NULL,
                 revision INTEGER NOT NULL CHECK(revision >= 0),
                 content_sha256 TEXT NOT NULL CHECK(length(content_sha256) = 64),
                 history_created_at_epoch_millis INTEGER NOT NULL,
                 pruned_at_epoch_millis INTEGER NOT NULL,
                 compressed_size_bytes INTEGER NOT NULL CHECK(compressed_size_bytes >= 0),
                 reason TEXT NOT NULL,
                 details_json TEXT NOT NULL CHECK(json_valid(details_json))
             ) STRICT;

             CREATE INDEX snapshot_history_prune_audit_user_index
                 ON snapshot_history_prune_audit(user_id, pruned_at_epoch_millis DESC);",
        )?;
        migrate_snapshot_history_to_content_store(&transaction)?;
        transaction.execute_batch(
            "DROP TABLE account_snapshot_history;
             ALTER TABLE account_snapshot_history_v4 RENAME TO account_snapshot_history;

             CREATE INDEX account_snapshot_history_created_index
                 ON account_snapshot_history(user_id, created_at_epoch_millis DESC);
             CREATE INDEX account_snapshot_history_content_index
                 ON account_snapshot_history(content_sha256);

             CREATE TRIGGER account_snapshot_history_insert_ref
             AFTER INSERT ON account_snapshot_history
             BEGIN
                 UPDATE snapshot_contents
                    SET reference_count = reference_count + 1
                  WHERE sha256 = NEW.content_sha256;
             END;

             CREATE TRIGGER account_snapshot_history_delete_ref
             AFTER DELETE ON account_snapshot_history
             BEGIN
                 UPDATE snapshot_contents
                    SET reference_count = reference_count - 1
                  WHERE sha256 = OLD.content_sha256;
                 DELETE FROM snapshot_contents
                  WHERE sha256 = OLD.content_sha256 AND reference_count = 0;
             END;

             CREATE TRIGGER account_snapshot_history_update_ref
             AFTER UPDATE OF content_sha256 ON account_snapshot_history
             WHEN OLD.content_sha256 <> NEW.content_sha256
             BEGIN
                 UPDATE snapshot_contents
                    SET reference_count = reference_count - 1
                  WHERE sha256 = OLD.content_sha256;
                 UPDATE snapshot_contents
                    SET reference_count = reference_count + 1
                  WHERE sha256 = NEW.content_sha256;
                 DELETE FROM snapshot_contents
                  WHERE sha256 = OLD.content_sha256 AND reference_count = 0;
             END;",
        )?;
        transaction.execute(
            "INSERT INTO schema_migrations(version, description, applied_at_epoch_millis)
             VALUES (4, 'content addressed compressed snapshot history with audited pruning', ?1)",
            params![now_epoch_millis],
        )?;
        transaction.pragma_update(None, "user_version", 4)?;
        verify_snapshot_content_index(&transaction)?;
        transaction.commit()?;
    }
    if current_version < 5 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !table_has_column(&transaction, "account_snapshots", "restore_generation")? {
            transaction.execute_batch(
                "ALTER TABLE account_snapshots
                     ADD COLUMN restore_generation INTEGER NOT NULL DEFAULT 0
                     CHECK(restore_generation >= 0);",
            )?;
        }
        if !table_has_column(&transaction, "tokens", "last_seen_restore_generation")? {
            transaction.execute_batch(
                "ALTER TABLE tokens
                     ADD COLUMN last_seen_restore_generation INTEGER NOT NULL DEFAULT 0
                     CHECK(last_seen_restore_generation >= 0);",
            )?;
        }
        transaction.execute(
            "INSERT INTO schema_migrations(version, description, applied_at_epoch_millis)
             VALUES (5, 'account restore generation barrier for stale devices', ?1)",
            params![now_epoch_millis],
        )?;
        transaction.pragma_update(None, "user_version", 5)?;
        transaction.commit()?;
    }
    if current_version < 6 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !table_has_column(
            &transaction,
            "account_snapshot_history",
            "media_snapshot_complete",
        )? {
            transaction.execute_batch(
                "ALTER TABLE account_snapshot_history
                     ADD COLUMN media_snapshot_complete INTEGER NOT NULL DEFAULT 0
                     CHECK(media_snapshot_complete IN (0, 1));",
            )?;
        }
        if !table_has_column(&transaction, "tokens", "restore_acknowledged")? {
            transaction.execute_batch(
                "ALTER TABLE tokens
                     ADD COLUMN restore_acknowledged INTEGER NOT NULL DEFAULT 0
                     CHECK(restore_acknowledged IN (0, 1));
                 ALTER TABLE tokens
                     ADD COLUMN pending_restore_generation INTEGER;
                 ALTER TABLE tokens
                     ADD COLUMN pending_restore_receipt TEXT NOT NULL DEFAULT '';",
            )?;
        }
        transaction.execute_batch(
            "CREATE TABLE IF NOT EXISTS media_snapshot_contents (
                 sha256 TEXT PRIMARY KEY NOT NULL CHECK(length(sha256) = 64),
                 size_bytes INTEGER NOT NULL CHECK(size_bytes >= 0 AND size_bytes <= 16777216),
                 content BLOB NOT NULL,
                 reference_count INTEGER NOT NULL DEFAULT 0 CHECK(reference_count >= 0),
                 created_at_epoch_millis INTEGER NOT NULL,
                 CHECK(length(content) = size_bytes)
             ) STRICT;

             CREATE TABLE IF NOT EXISTS account_snapshot_media_history (
                 user_id TEXT NOT NULL,
                 account_revision INTEGER NOT NULL CHECK(account_revision >= 0),
                 attachment_id TEXT NOT NULL,
                 content_sha256 TEXT REFERENCES media_snapshot_contents(sha256) ON DELETE RESTRICT,
                 declared_sha256 TEXT NOT NULL,
                 mime_type TEXT NOT NULL,
                 size_bytes INTEGER NOT NULL CHECK(size_bytes >= 0 AND size_bytes <= 16777216),
                 updated_at_epoch_millis INTEGER NOT NULL,
                 missing_reason TEXT NOT NULL DEFAULT '',
                 PRIMARY KEY(user_id, account_revision, attachment_id),
                 FOREIGN KEY(user_id, account_revision)
                     REFERENCES account_snapshot_history(user_id, revision) ON DELETE CASCADE,
                 CHECK(content_sha256 IS NOT NULL OR length(missing_reason) > 0)
             ) STRICT;

             CREATE INDEX IF NOT EXISTS account_snapshot_media_history_content_index
                 ON account_snapshot_media_history(content_sha256);

             CREATE TRIGGER IF NOT EXISTS account_snapshot_media_history_insert_ref
             AFTER INSERT ON account_snapshot_media_history
             WHEN NEW.content_sha256 IS NOT NULL
             BEGIN
                 UPDATE media_snapshot_contents
                    SET reference_count = reference_count + 1
                  WHERE sha256 = NEW.content_sha256;
             END;

             CREATE TRIGGER IF NOT EXISTS account_snapshot_media_history_delete_ref
             AFTER DELETE ON account_snapshot_media_history
             WHEN OLD.content_sha256 IS NOT NULL
             BEGIN
                 UPDATE media_snapshot_contents
                    SET reference_count = reference_count - 1
                  WHERE sha256 = OLD.content_sha256;
                 DELETE FROM media_snapshot_contents
                  WHERE sha256 = OLD.content_sha256 AND reference_count = 0;
             END;

             CREATE TRIGGER IF NOT EXISTS account_snapshot_media_history_update_ref
             AFTER UPDATE OF content_sha256 ON account_snapshot_media_history
             WHEN OLD.content_sha256 IS NOT NEW.content_sha256
             BEGIN
                 UPDATE media_snapshot_contents
                    SET reference_count = reference_count - 1
                  WHERE sha256 = OLD.content_sha256;
                 UPDATE media_snapshot_contents
                    SET reference_count = reference_count + 1
                  WHERE sha256 = NEW.content_sha256;
                 DELETE FROM media_snapshot_contents
                  WHERE sha256 = OLD.content_sha256 AND reference_count = 0;
             END;",
        )?;
        mark_legacy_media_free_histories_complete(&transaction)?;
        transaction.execute(
            "UPDATE media_snapshot_contents
             SET reference_count = (
                 SELECT COUNT(*) FROM account_snapshot_media_history h
                 WHERE h.content_sha256 = media_snapshot_contents.sha256
             )",
            [],
        )?;
        transaction.execute(
            "DELETE FROM media_snapshot_contents WHERE reference_count = 0",
            [],
        )?;
        transaction.execute(
            "INSERT INTO schema_migrations(version, description, applied_at_epoch_millis)
             VALUES (6, 'media-complete account history and token-bound restore receipts', ?1)",
            params![now_epoch_millis],
        )?;
        transaction.pragma_update(None, "user_version", 6)?;
        transaction.commit()?;
    }
    if current_version < 7 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !table_has_column(&transaction, "account_snapshots", "content_sha256")? {
            transaction.execute_batch(
                "ALTER TABLE account_snapshots
                     ADD COLUMN content_sha256 TEXT NOT NULL
                     DEFAULT '0000000000000000000000000000000000000000000000000000000000000000'
                     CHECK(length(content_sha256) = 64);",
            )?;
        }
        let snapshots = {
            let mut statement = transaction
                .prepare("SELECT user_id, app_data_json FROM account_snapshots ORDER BY user_id")?;
            let snapshots = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            snapshots
        };
        for (user_id, app_data_json) in snapshots {
            validate_app_data_json_structure(&app_data_json)?;
            transaction.execute(
                "UPDATE account_snapshots SET content_sha256 = ?1 WHERE user_id = ?2",
                params![sha256_hex(app_data_json.as_bytes()), user_id],
            )?;
        }
        transaction.execute(
            "INSERT INTO schema_migrations(version, description, applied_at_epoch_millis)
             VALUES (7, 'hashed current account snapshots for semantic corruption detection', ?1)",
            params![now_epoch_millis],
        )?;
        transaction.pragma_update(None, "user_version", 7)?;
        transaction.commit()?;
    }
    if current_version < 8 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(
            "CREATE TABLE IF NOT EXISTS server_identity (
                 singleton INTEGER PRIMARY KEY NOT NULL CHECK(singleton = 1),
                 server_instance_id TEXT NOT NULL UNIQUE CHECK(length(server_instance_id) = 64),
                 created_at_epoch_millis INTEGER NOT NULL
             ) STRICT;

             CREATE TABLE IF NOT EXISTS account_namespaces (
                 user_id TEXT PRIMARY KEY NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                 account_namespace TEXT NOT NULL UNIQUE CHECK(length(account_namespace) = 64),
                 created_at_epoch_millis INTEGER NOT NULL
             ) STRICT;",
        )?;
        let server_instance_id = transaction
            .query_row(
                "SELECT server_instance_id FROM server_identity WHERE singleton = 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .unwrap_or_else(random_opaque_identifier);
        transaction.execute(
            "INSERT OR IGNORE INTO server_identity(
                 singleton, server_instance_id, created_at_epoch_millis
             ) VALUES (1, ?1, ?2)",
            params![server_instance_id, now_epoch_millis],
        )?;
        let users = {
            let mut statement =
                transaction.prepare("SELECT id, created_at_epoch_millis FROM users ORDER BY id")?;
            let users = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            users
        };
        for (user_id, created_at_epoch_millis) in users {
            transaction.execute(
                "INSERT INTO account_namespaces(
                     user_id, account_namespace, created_at_epoch_millis
                 ) VALUES (?1, ?2, ?3)
                 ON CONFLICT(user_id) DO UPDATE SET
                     account_namespace = excluded.account_namespace",
                params![
                    user_id,
                    account_namespace_identifier(&server_instance_id, &user_id),
                    created_at_epoch_millis
                ],
            )?;
        }
        transaction.execute(
            "INSERT INTO schema_migrations(version, description, applied_at_epoch_millis)
             VALUES (8, 'stable server and account namespaces for client workspace isolation', ?1)",
            params![now_epoch_millis],
        )?;
        transaction.pragma_update(None, "user_version", 8)?;
        transaction.commit()?;
    }
    if current_version < 9 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !table_has_column(&transaction, "account_snapshots", "envelope_sha256")? {
            transaction.execute_batch(
                "ALTER TABLE account_snapshots
                     ADD COLUMN envelope_sha256 TEXT NOT NULL
                     DEFAULT '0000000000000000000000000000000000000000000000000000000000000000'
                     CHECK(length(envelope_sha256) = 64);",
            )?;
        }
        let snapshots = {
            let mut statement = transaction.prepare(
                "SELECT user_id, app_data_json, revision, updated_at_epoch_millis,
                        restore_generation, content_sha256
                 FROM account_snapshots ORDER BY user_id",
            )?;
            let snapshots = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            snapshots
        };
        for (
            user_id,
            app_data_json,
            revision,
            updated_at_epoch_millis,
            restore_generation,
            content_sha256,
        ) in snapshots
        {
            validate_app_data_json_structure(&app_data_json)?;
            if !valid_sha256_hex(&content_sha256)
                || !content_sha256.eq_ignore_ascii_case(&sha256_hex(app_data_json.as_bytes()))
            {
                return Err(StoreError::Integrity(format!(
                    "current account snapshot SHA-256 mismatch while adding envelope for user {user_id}"
                )));
            }
            transaction.execute(
                "UPDATE account_snapshots SET envelope_sha256 = ?1 WHERE user_id = ?2",
                params![
                    account_snapshot_envelope_sha256(
                        &user_id,
                        &app_data_json,
                        revision,
                        updated_at_epoch_millis,
                        restore_generation,
                    ),
                    user_id
                ],
            )?;
        }
        transaction.execute(
            "INSERT INTO schema_migrations(version, description, applied_at_epoch_millis)
             VALUES (9, 'account snapshot envelope hashes bind owner revision time and restore generation', ?1)",
            params![now_epoch_millis],
        )?;
        transaction.pragma_update(None, "user_version", 9)?;
        transaction.commit()?;
    }
    if current_version < 10 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !table_has_column(
            &transaction,
            "server_identity",
            "workspace_capability_secret",
        )? {
            transaction.execute_batch(
                "ALTER TABLE server_identity
                     ADD COLUMN workspace_capability_secret TEXT NOT NULL
                     DEFAULT '0000000000000000000000000000000000000000000000000000000000000000'
                     CHECK(length(workspace_capability_secret) = 64);",
            )?;
        }
        let existing_secret = transaction.query_row(
            "SELECT workspace_capability_secret FROM server_identity WHERE singleton = 1",
            [],
            |row| row.get::<_, String>(0),
        )?;
        if existing_secret == "0000000000000000000000000000000000000000000000000000000000000000" {
            transaction.execute(
                "UPDATE server_identity SET workspace_capability_secret = ?1 WHERE singleton = 1",
                params![random_opaque_identifier()],
            )?;
        } else if !valid_workspace_capability_secret(&existing_secret) {
            return Err(StoreError::Integrity(
                "workspace capability secret is malformed during schema migration".to_string(),
            ));
        }
        transaction.execute(
            "INSERT INTO schema_migrations(version, description, applied_at_epoch_millis)
             VALUES (10, 'persistent HMAC secret for cross-token workspace capabilities', ?1)",
            params![now_epoch_millis],
        )?;
        transaction.pragma_update(None, "user_version", 10)?;
        transaction.commit()?;
    }
    if current_version < 11 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !table_has_column(&transaction, "tokens", "activation_state")? {
            transaction.execute_batch(
                "ALTER TABLE tokens
                     ADD COLUMN activation_state INTEGER NOT NULL DEFAULT 1
                     CHECK(activation_state IN (0, 1));",
            )?;
        }
        if !table_has_column(&transaction, "tokens", "activated_at_epoch_millis")? {
            transaction.execute_batch(
                "ALTER TABLE tokens
                     ADD COLUMN activated_at_epoch_millis INTEGER NOT NULL DEFAULT 0
                     CHECK(activated_at_epoch_millis >= 0);",
            )?;
        }
        transaction.execute_batch(
            "CREATE INDEX IF NOT EXISTS tokens_pending_activation_index
                     ON tokens(expires_at_epoch_millis)
                     WHERE activation_state = 0 AND revoked_at_epoch_millis IS NULL;",
        )?;
        transaction.execute(
            "INSERT INTO schema_migrations(version, description, applied_at_epoch_millis)
             VALUES (11, 'short-lived token leases with crash-safe sync activation', ?1)",
            params![now_epoch_millis],
        )?;
        transaction.pragma_update(None, "user_version", 11)?;
        transaction.commit()?;
    }
    if current_version < 12 {
        ensure_schema_v12_reconciliation_capacity(connection)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        drop_media_metadata_validation_triggers(&transaction)?;
        reconcile_media_histories_for_schema_v12(&transaction, now_epoch_millis, false)?;
        verify_snapshot_content_index(&transaction)?;
        transaction.execute(
            "INSERT INTO schema_migrations(version, description, applied_at_epoch_millis)
             VALUES (12, 'reconcile note-version media manifests before exact verification', ?1)",
            params![now_epoch_millis],
        )?;
        transaction.pragma_update(None, "user_version", 12)?;
        transaction.commit()?;
    }
    if current_version < 13 {
        ensure_schema_v13_media_index_capacity(connection)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(manifest) = legacy_snapshot_repair_manifest {
            verify_legacy_snapshot_repair_manifest_matches_live(&transaction, &manifest.sources)?;
        }
        drop_media_metadata_validation_triggers(&transaction)?;
        install_current_snapshot_media_identity_table(&transaction)?;
        install_legacy_snapshot_repair_allowance_table(&transaction)?;
        rebuild_current_snapshot_media_identities(&transaction)?;
        if let Some(manifest) = legacy_snapshot_repair_manifest {
            let seeds = manifest
                .sources
                .iter()
                .cloned()
                .map(|source| LegacySnapshotRepairAllowanceSeed {
                    source,
                    seeded_at_epoch_millis: now_epoch_millis,
                })
                .collect::<Vec<_>>();
            seed_legacy_snapshot_repair_allowances(&transaction, &seeds, &manifest.backup)?;
        }
        normalize_legacy_media_history_declared_sha256(&transaction)?;
        verify_current_snapshot_media_identity_index(&transaction)?;
        let audit = verify_media_storage_invariants(&transaction)?;
        install_media_metadata_validation_triggers(&transaction)?;
        transaction.execute(
            "INSERT INTO schema_migrations(version, description, applied_at_epoch_millis)
             VALUES (13, 'fail-closed media metadata audit and global storage growth guards', ?1)",
            params![now_epoch_millis],
        )?;
        transaction.pragma_update(None, "user_version", 13)?;
        transaction.commit()?;
        if audit.has_grandfathered_overage() {
            eprintln!(
                "schema v13 retained existing media quota overage in no-growth mode; reads and cleanup remain available"
            );
        }
    }
    if current_version < 14 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if current_version == 13 {
            let manifest = legacy_snapshot_repair_manifest.ok_or_else(|| {
                StoreError::Integrity(
                    "schema v13 migration requires verified backup evidence".to_string(),
                )
            })?;
            verify_legacy_snapshot_repair_manifest_matches_live(&transaction, &manifest.sources)?;
            let mut retained =
                read_legacy_snapshot_repair_allowance_seeds_for_v14_migration(&transaction)?;
            retained.retain(|allowance| manifest.sources.contains(&allowance.source));
            install_legacy_snapshot_repair_allowance_table(&transaction)?;
            seed_legacy_snapshot_repair_allowances(&transaction, &retained, &manifest.backup)?;
        }
        transaction.execute(
            "INSERT INTO schema_migrations(version, description, applied_at_epoch_millis)
             VALUES (14, 'bind legacy snapshot repair allowances to verified recovery backups', ?1)",
            params![now_epoch_millis],
        )?;
        transaction.pragma_update(None, "user_version", 14)?;
        transaction.commit()?;
    }
    #[cfg(test)]
    if preschema_privacy::MIGRATE_ONLY_THROUGH.with(|value| value.get() == 14) {
        return Ok(current_version < 14);
    }
    if current_version < 15 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(note_privacy::schema_sql())?;
        transaction.execute_batch(privacy_journal::witness_schema_sql())?;
        transaction.execute(
            "INSERT INTO schema_migrations(version, description, applied_at_epoch_millis)
             VALUES (15, 'retain note privacy barriers across recovery and request replay', ?1)",
            params![now_epoch_millis],
        )?;
        transaction.pragma_update(None, "user_version", 15)?;
        transaction.commit()?;
    }
    #[cfg(test)]
    if preschema_privacy::MIGRATE_ONLY_THROUGH.with(|value| value.get() == 15) {
        return Ok(current_version < 15);
    }
    if current_version < 16 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        verify_required_schema_at_version(&transaction, 15)?;
        verify_semantic_storage_integrity(&transaction)?;
        transaction.execute(
            "INSERT INTO schema_migrations(version, description, applied_at_epoch_millis)
             VALUES (16, 'require readers that preserve private attachment declarations and deletion fences', ?1)",
            params![now_epoch_millis],
        )?;
        transaction.pragma_update(None, "user_version", 16)?;
        // Format 15 readers did not index valid typed note conflicts. Preserve
        // their verification contract above, then rebuild under format 16 in
        // this same transaction. Missing old bytes remain explicitly incomplete.
        rebuild_current_snapshot_media_identities(&transaction)?;
        reconcile_media_histories_for_schema_v12(&transaction, now_epoch_millis, true)?;
        verify_semantic_storage_integrity(&transaction)?;
        transaction.commit()?;
    }
    if current_version < 17 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(legal_reports::schema_sql())?;
        transaction.execute(
            "INSERT INTO schema_migrations(version, description, applied_at_epoch_millis)
             VALUES (17, 'account workspace legal reports, staged chunks and permanent tombstones', ?1)",
            params![now_epoch_millis],
        )?;
        transaction.pragma_update(None, "user_version", 17)?;
        transaction.commit()?;
    }
    Ok(current_version < SCHEMA_VERSION)
}

struct PendingLegacyFile {
    path: PathBuf,
    raw: Vec<u8>,
    content_sha256: String,
    legacy: LegacyServerStore,
    source_updated_at_epoch_millis: i64,
    backup_path: Option<PathBuf>,
}

fn find_user_for_legacy(
    transaction: &Transaction<'_>,
    incoming: &LegacyServerUser,
) -> StoreResult<Option<StoredUser>> {
    const SELECT_USER: &str =
        "SELECT u.id, u.email, u.password_salt, u.password_hash, u.password_scheme, \
                u.created_at_epoch_millis, u.updated_at_epoch_millis, \
                s.app_data_json, s.revision, s.updated_at_epoch_millis, s.content_sha256, \
                s.restore_generation, s.envelope_sha256 \
         FROM users u JOIN account_snapshots s ON s.user_id = u.id";
    let by_id = transaction
        .query_row(
            &format!("{SELECT_USER} WHERE u.id = ?1"),
            params![incoming.id],
            |row| {
                Ok((
                    stored_user_from_row(row)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, i64>(11)?,
                    row.get::<_, String>(12)?,
                ))
            },
        )
        .optional()?;
    let by_email = transaction
        .query_row(
            &format!("{SELECT_USER} WHERE u.email = ?1 COLLATE NOCASE"),
            params![incoming.email.trim()],
            |row| {
                Ok((
                    stored_user_from_row(row)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, i64>(11)?,
                    row.get::<_, String>(12)?,
                ))
            },
        )
        .optional()?;
    let by_id = by_id
        .map(
            |(user, content_sha256, restore_generation, envelope_sha256)| {
                verify_current_account_snapshot(
                    &user.account,
                    restore_generation,
                    &content_sha256,
                    &envelope_sha256,
                )?;
                Ok::<_, StoreError>(user)
            },
        )
        .transpose()?;
    let by_email = by_email
        .map(
            |(user, content_sha256, restore_generation, envelope_sha256)| {
                verify_current_account_snapshot(
                    &user.account,
                    restore_generation,
                    &content_sha256,
                    &envelope_sha256,
                )?;
                Ok::<_, StoreError>(user)
            },
        )
        .transpose()?;
    match (by_id, by_email) {
        (None, None) => Ok(None),
        (Some(by_id), Some(by_email)) if by_id.id == by_email.id => {
            if by_id.id != incoming.id
                || !by_id.email.trim().eq_ignore_ascii_case(incoming.email.trim())
            {
                return Err(StoreError::Integrity(format!(
                    "legacy identity fields do not both match account {}",
                    by_id.id
                )));
            }
            Ok(Some(by_id))
        }
        (Some(by_id), Some(by_email)) => Err(StoreError::Integrity(format!(
            "legacy identity collision: id {} belongs to account {}, but email {} belongs to account {}",
            incoming.id, by_id.id, incoming.email, by_email.id
        ))),
        (Some(by_id), None) => Err(StoreError::Integrity(format!(
            "legacy identity collision: id {} belongs to account {}, but email {} does not match that account",
            incoming.id, by_id.id, incoming.email
        ))),
        (None, Some(by_email)) => Err(StoreError::Integrity(format!(
            "legacy identity collision: email {} belongs to account {}, but incoming id {} does not match that account",
            incoming.email, by_email.id, incoming.id
        ))),
    }
}

fn merge_snapshot_values<F>(
    existing: &str,
    incoming: &str,
    now_epoch_millis: i64,
    merge_snapshot: &mut F,
) -> StoreResult<String>
where
    F: FnMut(&str, &str, i64) -> StoreResult<String>,
{
    match (existing.trim().is_empty(), incoming.trim().is_empty()) {
        (true, true) => Ok(String::new()),
        (true, false) => Ok(incoming.to_string()),
        (false, true) => Ok(existing.to_string()),
        (false, false) if existing == incoming => Ok(existing.to_string()),
        (false, false) => merge_snapshot(existing, incoming, now_epoch_millis),
    }
}

fn insert_legacy_token(
    transaction: &Transaction<'_>,
    user_id: &str,
    token: &LegacyServerToken,
    now_epoch_millis: i64,
    token_ttl_millis: i64,
) -> StoreResult<bool> {
    let expires_at = legacy_token_expiry(token, now_epoch_millis, token_ttl_millis)?;
    let token_hash = token_fingerprint(&token.token);
    let token_id = token_id_from_hash(&token_hash)?;
    let existing = transaction
        .query_row(
            "SELECT user_id, token_hash FROM tokens WHERE token_id = ?1 OR token_hash = ?2",
            params![token_id, token_hash],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    if let Some((existing_user_id, existing_hash)) = existing {
        if existing_user_id != user_id || existing_hash != token_hash {
            return Err(StoreError::Integrity(
                "legacy token id/hash collision across users".to_string(),
            ));
        }
        transaction.execute(
            "UPDATE tokens \
             SET last_seen_at_epoch_millis = MAX(last_seen_at_epoch_millis, ?1), \
                 expires_at_epoch_millis = MAX(expires_at_epoch_millis, ?2) \
             WHERE token_hash = ?3",
            params![token.last_seen_at_epoch_millis, expires_at, token_hash],
        )?;
        return Ok(false);
    }
    transaction.execute(
        "INSERT INTO tokens(
             user_id, token_id, token_hash, device_name, created_at_epoch_millis,
             last_seen_at_epoch_millis, expires_at_epoch_millis, revoked_at_epoch_millis
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
        params![
            user_id,
            token_id,
            token_hash,
            token.device_name,
            token.created_at_epoch_millis,
            token.last_seen_at_epoch_millis,
            expires_at,
        ],
    )?;
    Ok(true)
}

fn legacy_token_expiry(
    token: &LegacyServerToken,
    now_epoch_millis: i64,
    token_ttl_millis: i64,
) -> StoreResult<i64> {
    if token_ttl_millis <= 0 {
        return Err(StoreError::Integrity(
            "legacy token TTL must be positive".to_string(),
        ));
    }
    let anchor = token
        .created_at_epoch_millis
        .max(token.last_seen_at_epoch_millis);
    let historical_expiry = anchor.saturating_add(token_ttl_millis);
    let maximum_expiry = now_epoch_millis.saturating_add(token_ttl_millis);
    let expires_at = historical_expiry
        .min(maximum_expiry)
        .max(token.created_at_epoch_millis.saturating_add(1));
    if expires_at <= token.created_at_epoch_millis {
        return Err(StoreError::Integrity(
            "legacy token timestamps cannot produce a valid expiry".to_string(),
        ));
    }
    Ok(expires_at)
}

fn migrate_legacy_store(
    connection: &mut Connection,
    source_path: &Path,
    now_epoch_millis: i64,
    token_ttl_millis: i64,
) -> StoreResult<Option<LegacyMigrationReport>> {
    let raw = fs::read(source_path)?;
    let content_sha256 = sha256_hex(&raw);
    let already_imported = connection
        .query_row(
            "SELECT 1 FROM legacy_imports WHERE content_sha256 = ?1",
            params![content_sha256],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if already_imported {
        return Ok(None);
    }

    let legacy: LegacyServerStore = serde_json::from_slice(&raw)?;
    validate_legacy_store(&legacy)?;
    let backup_path = write_timestamped_backup(source_path, &raw, now_epoch_millis)?;
    let tokens_imported = legacy
        .users
        .iter()
        .map(|user| user.tokens.len())
        .sum::<usize>();
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for user in &legacy.users {
        let revision = i64::from(!user.app_data_json.trim().is_empty());
        insert_new_user(
            &transaction,
            &NewStoredUser {
                id: user.id.clone(),
                email: user.email.clone(),
                password_salt: user.password_salt.clone(),
                password_hash: user.password_hash.clone(),
                password_scheme: "legacy_sha256".to_string(),
                created_at_epoch_millis: user.created_at_epoch_millis,
                updated_at_epoch_millis: user.updated_at_epoch_millis,
                app_data_json: user.app_data_json.clone(),
                account_revision: revision,
            },
            NewUserMediaPolicy::GrandfatherVerifiedLegacyImport,
        )?;
        for token in &user.tokens {
            let expires_at_epoch_millis =
                legacy_token_expiry(token, now_epoch_millis, token_ttl_millis)?;
            transaction.execute(
                "INSERT INTO tokens(
                     user_id, token_id, token_hash, device_name, created_at_epoch_millis,
                     last_seen_at_epoch_millis, expires_at_epoch_millis, revoked_at_epoch_millis
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
                params![
                    user.id,
                    token_identifier(&token.token),
                    token_fingerprint(&token.token),
                    token.device_name,
                    token.created_at_epoch_millis,
                    token.last_seen_at_epoch_millis,
                    expires_at_epoch_millis,
                ],
            )?;
        }
    }
    transaction.execute(
        "INSERT INTO legacy_imports(
             content_sha256, source_path, backup_path, imported_at_epoch_millis,
             users_imported, tokens_imported
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            content_sha256,
            source_path.to_string_lossy(),
            backup_path.to_string_lossy(),
            now_epoch_millis,
            legacy.users.len() as i64,
            tokens_imported as i64,
        ],
    )?;
    privacy_journal::commit(transaction)?;

    Ok(Some(LegacyMigrationReport {
        source_path: source_path.to_path_buf(),
        backup_path,
        content_sha256,
        users_imported: legacy.users.len(),
        tokens_imported,
    }))
}

fn validate_registered_account_capacity(
    transaction: &Transaction<'_>,
    now_epoch_millis: i64,
) -> StoreResult<()> {
    let mut current_accounts =
        transaction.query_row("SELECT COUNT(*) FROM users", [], |row| row.get::<_, i64>(0))?;
    if current_accounts >= MAX_REGISTERED_ACCOUNTS {
        let required_reclaims = current_accounts
            .checked_sub(MAX_REGISTERED_ACCOUNTS)
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| {
                StoreError::Integrity("registered account reclaim projection overflow".to_string())
            })?;
        reclaim_abandoned_initial_registrations(transaction, now_epoch_millis, required_reclaims)?;
        current_accounts =
            transaction.query_row("SELECT COUNT(*) FROM users", [], |row| row.get::<_, i64>(0))?;
    }
    if current_accounts >= MAX_REGISTERED_ACCOUNTS {
        return Err(StoreError::RegisteredAccountQuotaExceeded {
            current_accounts,
            limit_accounts: MAX_REGISTERED_ACCOUNTS,
        });
    }
    Ok(())
}

fn reclaim_abandoned_initial_registrations(
    transaction: &Transaction<'_>,
    now_epoch_millis: i64,
    required_reclaims: i64,
) -> StoreResult<usize> {
    if required_reclaims <= 0 {
        return Ok(0);
    }
    let recovery_cutoff = now_epoch_millis.saturating_sub(
        INITIAL_REGISTRATION_RECOVERY_WINDOW_MILLIS
            .saturating_add(ABANDONED_REGISTRATION_SAFETY_MARGIN_MILLIS),
    );
    let candidates = {
        let mut statement = transaction.prepare(
            "SELECT u.id, u.created_at_epoch_millis,
                    s.content_sha256, s.envelope_sha256,
                    n.account_namespace,
                    t.created_at_epoch_millis, t.last_seen_at_epoch_millis,
                    t.expires_at_epoch_millis
             FROM users u
             JOIN account_snapshots s ON s.user_id = u.id
             JOIN account_namespaces n ON n.user_id = u.id
             JOIN tokens t ON t.user_id = u.id
             WHERE u.password_scheme = 'argon2id_phc'
               AND u.password_salt = ''
               AND u.updated_at_epoch_millis = u.created_at_epoch_millis
               AND s.revision = 0
               AND trim(s.app_data_json) = ''
               AND s.restore_generation = 0
               AND s.updated_at_epoch_millis = u.created_at_epoch_millis
               AND n.created_at_epoch_millis = u.created_at_epoch_millis
               AND t.activation_state = 0
               AND t.activated_at_epoch_millis = 0
               AND t.restore_acknowledged = 1
               AND t.last_seen_restore_generation = 0
               AND t.pending_restore_generation IS NULL
               AND t.pending_restore_receipt = ''
               AND t.created_at_epoch_millis = u.created_at_epoch_millis
               AND t.last_seen_at_epoch_millis = t.created_at_epoch_millis
               AND t.expires_at_epoch_millis <= ?1
               AND (t.revoked_at_epoch_millis IS NULL
                    OR t.revoked_at_epoch_millis = t.expires_at_epoch_millis)
               AND NOT EXISTS(
                   SELECT 1 FROM tokens other
                   WHERE other.user_id = u.id
                     AND (other.activation_state <> 0
                          OR other.activated_at_epoch_millis <> 0
                          OR other.expires_at_epoch_millis > ?1
                          OR (other.revoked_at_epoch_millis IS NOT NULL
                              AND other.revoked_at_epoch_millis
                                  <> other.expires_at_epoch_millis))
               )
               AND (SELECT COUNT(*) FROM account_snapshot_history h
                    WHERE h.user_id = u.id) = 1
               AND EXISTS(
                   SELECT 1 FROM account_snapshot_history h
                   WHERE h.user_id = u.id
                     AND h.revision = 0
                     AND h.content_sha256 = s.content_sha256
                     AND h.created_at_epoch_millis = u.created_at_epoch_millis
                     AND h.media_snapshot_complete = 1
               )
               AND NOT EXISTS(SELECT 1 FROM request_dedup d WHERE d.user_id = u.id)
               AND NOT EXISTS(SELECT 1 FROM snapshot_history_prune_audit p WHERE p.user_id = u.id)
               AND NOT EXISTS(SELECT 1 FROM note_media m WHERE m.user_id = u.id)
               AND NOT EXISTS(SELECT 1 FROM note_media_tombstones mt WHERE mt.user_id = u.id)
               AND NOT EXISTS(SELECT 1 FROM account_snapshot_media_history mh WHERE mh.user_id = u.id)
               AND NOT EXISTS(SELECT 1 FROM account_snapshot_media_identities mi WHERE mi.user_id = u.id)
               AND NOT EXISTS(SELECT 1 FROM legacy_snapshot_bindings b WHERE b.user_id = u.id)
             ORDER BY u.created_at_epoch_millis, u.id",
        )?;
        let rows = statement.query_map(params![recovery_cutoff], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
            ))
        })?;
        let collected = rows.collect::<Result<Vec<_>, _>>()?;
        collected
    };
    let server_instance_id = transaction.query_row(
        "SELECT server_instance_id FROM server_identity WHERE singleton = 1",
        [],
        |row| row.get::<_, String>(0),
    )?;
    let empty_content_sha256 = sha256_hex(b"");
    let mut seen_user_ids = HashSet::new();
    let eligible = candidates
        .into_iter()
        .filter(
            |(
                user_id,
                created_at,
                content_sha256,
                envelope_sha256,
                account_namespace,
                token_created_at,
                token_last_seen_at,
                token_expires_at,
            )| {
                *token_created_at == *created_at
                    && *token_last_seen_at == *created_at
                    && *token_expires_at
                        == created_at.saturating_add(INITIAL_REGISTRATION_ACTIVATION_LEASE_MILLIS)
                    && content_sha256 == &empty_content_sha256
                    && envelope_sha256
                        == &account_snapshot_envelope_sha256(user_id, "", 0, *created_at, 0)
                    && account_namespace
                        == &account_namespace_identifier(&server_instance_id, user_id)
            },
        )
        .filter_map(|(user_id, ..)| seen_user_ids.insert(user_id.clone()).then_some(user_id))
        .collect::<Vec<_>>();
    let required = usize::try_from(required_reclaims).map_err(|_| {
        StoreError::Integrity("registered account reclaim count overflow".to_string())
    })?;
    if eligible.len() < required {
        return Ok(0);
    }
    for user_id in eligible.into_iter().take(required) {
        let deleted = transaction.execute("DELETE FROM users WHERE id = ?1", params![user_id])?;
        if deleted != 1 {
            return Err(StoreError::Integrity(
                "abandoned registration reclaim changed an unexpected row count".to_string(),
            ));
        }
    }
    verify_snapshot_content_index(transaction)?;
    Ok(required)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NewUserMediaPolicy {
    EnforceGrowth,
    GrandfatherVerifiedLegacyImport,
}

fn insert_new_user(
    transaction: &Transaction<'_>,
    user: &NewStoredUser,
    media_policy: NewUserMediaPolicy,
) -> StoreResult<()> {
    if media_policy == NewUserMediaPolicy::EnforceGrowth {
        validate_app_data_media_identity_quota(transaction, &user.id, &user.app_data_json)?;
        validate_archivable_app_data_media_metadata(&user.app_data_json)?;
    }
    ensure_snapshot_write_capacity(transaction, user.app_data_json.len() as u64)?;
    let content_sha256 = sha256_hex(user.app_data_json.as_bytes());
    let envelope_sha256 = account_snapshot_envelope_sha256(
        &user.id,
        &user.app_data_json,
        user.account_revision,
        user.updated_at_epoch_millis,
        0,
    );
    transaction.execute(
        "INSERT INTO users(
             id, email, password_salt, password_hash, password_scheme,
             created_at_epoch_millis, updated_at_epoch_millis
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            user.id,
            user.email.trim(),
            user.password_salt,
            user.password_hash,
            user.password_scheme,
            user.created_at_epoch_millis,
            user.updated_at_epoch_millis,
        ],
    )?;
    let server_instance_id = transaction.query_row(
        "SELECT server_instance_id FROM server_identity WHERE singleton = 1",
        [],
        |row| row.get::<_, String>(0),
    )?;
    transaction.execute(
        "INSERT INTO account_namespaces(
             user_id, account_namespace, created_at_epoch_millis
         ) VALUES (?1, ?2, ?3)",
        params![
            user.id,
            account_namespace_identifier(&server_instance_id, &user.id),
            user.created_at_epoch_millis
        ],
    )?;
    transaction.execute(
        "INSERT INTO account_snapshots(
             user_id, app_data_json, revision, updated_at_epoch_millis, content_sha256,
             envelope_sha256
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            user.id,
            user.app_data_json,
            user.account_revision,
            user.updated_at_epoch_millis,
            content_sha256,
            envelope_sha256,
        ],
    )?;
    match media_policy {
        NewUserMediaPolicy::EnforceGrowth => {
            replace_current_snapshot_media_identities(transaction, &user.id, &user.app_data_json)?;
        }
        NewUserMediaPolicy::GrandfatherVerifiedLegacyImport => {
            replace_current_snapshot_media_identities_tolerant(
                transaction,
                &user.id,
                &user.app_data_json,
            )?;
        }
    }
    let initial = AccountSnapshot {
        user_id: user.id.clone(),
        app_data_json: user.app_data_json.clone(),
        revision: user.account_revision,
        updated_at_epoch_millis: user.updated_at_epoch_millis,
    };
    let history_limit = match media_policy {
        NewUserMediaPolicy::EnforceGrowth => SNAPSHOT_HISTORY_HARD_LIMIT_BYTES_PER_USER,
        NewUserMediaPolicy::GrandfatherVerifiedLegacyImport => i64::MAX,
    };
    insert_snapshot_history_with_limit(
        transaction,
        &initial,
        user.updated_at_epoch_millis,
        history_limit,
        media_policy,
    )?;
    note_privacy::enforce(transaction, &user.id)?;
    Ok(())
}

fn read_account_in_transaction(
    transaction: &Transaction<'_>,
    user_id: &str,
) -> StoreResult<Option<AccountSnapshot>> {
    let stored = transaction
        .query_row(
            "SELECT user_id, app_data_json, revision, updated_at_epoch_millis, \
                    restore_generation, content_sha256, envelope_sha256 \
             FROM account_snapshots WHERE user_id = ?1",
            params![user_id],
            |row| {
                Ok((
                    AccountSnapshot {
                        user_id: row.get(0)?,
                        app_data_json: row.get(1)?,
                        revision: row.get(2)?,
                        updated_at_epoch_millis: row.get(3)?,
                    },
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                ))
            },
        )
        .optional()?;
    match stored {
        Some((snapshot, restore_generation, content_sha256, envelope_sha256)) => {
            verify_current_account_snapshot(
                &snapshot,
                restore_generation,
                &content_sha256,
                &envelope_sha256,
            )?;
            Ok(Some(snapshot))
        }
        None => Ok(None),
    }
}

fn restore_generation_in_transaction(
    transaction: &Transaction<'_>,
    user_id: &str,
) -> StoreResult<i64> {
    transaction
        .query_row(
            "SELECT restore_generation FROM account_snapshots WHERE user_id = ?1",
            params![user_id],
            |row| row.get(0),
        )
        .map_err(StoreError::from)
}

fn next_account_revision(current_revision: i64) -> StoreResult<i64> {
    current_revision.checked_add(1).ok_or_else(|| {
        StoreError::Integrity(
            "account revision is exhausted; refusing to overwrite the current snapshot".to_string(),
        )
    })
}

fn import_account_snapshot_in_transaction<F>(
    transaction: &Transaction<'_>,
    user_id: &str,
    incoming_app_data_json: &str,
    source_updated_at_epoch_millis: i64,
    now_epoch_millis: i64,
    merge_snapshot: &mut F,
) -> StoreResult<SnapshotImportReport>
where
    F: FnMut(&str, i64, &str, i64, i64) -> StoreResult<String>,
{
    let current = read_account_in_transaction(transaction, user_id)?
        .ok_or_else(|| StoreError::NotFound(format!("account for user {user_id}")))?;
    validate_app_data_json(&current.app_data_json)?;
    let merged_json = merge_snapshot(
        &current.app_data_json,
        current.updated_at_epoch_millis,
        incoming_app_data_json,
        source_updated_at_epoch_millis,
        now_epoch_millis,
    )?;
    validate_app_data_json(&merged_json)?;
    let changed = merged_json != current.app_data_json;
    let snapshot = if changed {
        let revision = next_account_revision(current.revision)?;
        let updated_at = current
            .updated_at_epoch_millis
            .max(source_updated_at_epoch_millis);
        insert_snapshot_history_with_limit(
            transaction,
            &current,
            now_epoch_millis,
            i64::MAX,
            NewUserMediaPolicy::GrandfatherVerifiedLegacyImport,
        )?;
        ensure_snapshot_write_capacity(transaction, merged_json.len() as u64)?;
        let content_sha256 = sha256_hex(merged_json.as_bytes());
        let restore_generation = restore_generation_in_transaction(transaction, user_id)?;
        let envelope_sha256 = account_snapshot_envelope_sha256(
            user_id,
            &merged_json,
            revision,
            updated_at,
            restore_generation,
        );
        let updated = transaction.execute(
            "UPDATE account_snapshots \
             SET app_data_json = ?1, revision = ?2, updated_at_epoch_millis = ?3, \
                 content_sha256 = ?4, envelope_sha256 = ?5 \
             WHERE user_id = ?6 AND revision = ?7",
            params![
                merged_json,
                revision,
                updated_at,
                content_sha256,
                envelope_sha256,
                user_id,
                current.revision
            ],
        )?;
        if updated != 1 {
            return Err(StoreError::Integrity(
                "legacy snapshot import changed an unexpected row count".to_string(),
            ));
        }
        replace_current_snapshot_media_identities_tolerant(transaction, user_id, &merged_json)?;
        note_privacy::enforce(transaction, user_id)?;
        AccountSnapshot {
            user_id: user_id.to_string(),
            app_data_json: merged_json,
            revision,
            updated_at_epoch_millis: updated_at,
        }
    } else {
        current
    };
    Ok(SnapshotImportReport { snapshot, changed })
}

fn legacy_snapshot_binding_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<LegacySnapshotBinding> {
    Ok(LegacySnapshotBinding {
        binding_key: row.get(0)?,
        user_id: row.get(1)?,
        content_sha256: row.get(2)?,
        source_path: row.get(3)?,
        bound_at_epoch_millis: row.get(4)?,
    })
}

fn insert_snapshot_history(
    transaction: &Transaction<'_>,
    snapshot: &AccountSnapshot,
    created_at_epoch_millis: i64,
) -> StoreResult<()> {
    insert_snapshot_history_with_limit(
        transaction,
        snapshot,
        created_at_epoch_millis,
        SNAPSHOT_HISTORY_HARD_LIMIT_BYTES_PER_USER,
        NewUserMediaPolicy::EnforceGrowth,
    )
}

fn insert_snapshot_history_for_account_mutation(
    transaction: &Transaction<'_>,
    database_path: &Path,
    current: &AccountSnapshot,
    incoming_app_data_json: &str,
    created_at_epoch_millis: i64,
) -> StoreResult<()> {
    insert_snapshot_history_for_account_mutation_with_limit(
        transaction,
        database_path,
        current,
        incoming_app_data_json,
        created_at_epoch_millis,
        SNAPSHOT_HISTORY_HARD_LIMIT_BYTES_PER_USER,
    )
}

fn insert_snapshot_history_for_account_mutation_with_limit(
    transaction: &Transaction<'_>,
    database_path: &Path,
    current: &AccountSnapshot,
    incoming_app_data_json: &str,
    created_at_epoch_millis: i64,
    hard_limit_bytes: i64,
) -> StoreResult<()> {
    match insert_snapshot_history_with_limit(
        transaction,
        current,
        created_at_epoch_millis,
        hard_limit_bytes,
        NewUserMediaPolicy::EnforceGrowth,
    ) {
        Ok(()) => Ok(()),
        Err(error @ StoreError::SnapshotHistoryQuotaExceeded { .. }) => {
            let authorization = match legacy_snapshot_repair_authorization(
                transaction,
                database_path,
                current,
                incoming_app_data_json,
            ) {
                Ok(Some(authorization)) => authorization,
                Ok(None) | Err(_) => return Err(error),
            };
            if authorization.schema_version != 12 && authorization.schema_version != 13 {
                return Err(error);
            }
            // Schema v14 repair allowances retain the exact identity of a
            // verified v12 or v13 migration backup. The backup and source tuple
            // are reverified before this one-shot allowance is consumed. Do not
            // duplicate an over-quota legacy source into live history: the
            // caller's prospective-archive guard must still prove that the
            // smaller replacement can be archived under the ordinary limit.
            record_legacy_snapshot_repair_history_omission(
                transaction,
                current,
                created_at_epoch_millis,
                &authorization,
            )?;
            consume_legacy_snapshot_repair_allowance(transaction, current, &authorization)?;
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn legacy_snapshot_repair_authorization(
    transaction: &Transaction<'_>,
    database_path: &Path,
    current: &AccountSnapshot,
    incoming_app_data_json: &str,
) -> StoreResult<Option<LegacySnapshotRepairBackupIdentity>> {
    if !app_data_resource_footprint_strictly_decreased(
        &current.app_data_json,
        incoming_app_data_json,
    )? {
        return Ok(None);
    }
    let source_content_sha256 = sha256_hex(current.app_data_json.as_bytes());
    let backup = transaction
        .query_row(
            "SELECT backup_file_name, backup_size_bytes, backup_sha256,
                    backup_schema_version, backup_created_at_epoch_millis
             FROM legacy_snapshot_repair_allowances
             WHERE user_id = ?1
               AND source_revision = ?2
               AND source_content_sha256 = ?3",
            params![current.user_id, current.revision, source_content_sha256],
            |row| {
                Ok(LegacySnapshotRepairBackupIdentity {
                    file_name: row.get(0)?,
                    size_bytes: row.get(1)?,
                    sha256: row.get(2)?,
                    schema_version: row.get(3)?,
                    created_at_epoch_millis: row.get(4)?,
                })
            },
        )
        .optional()?;
    let Some(backup) = backup else {
        return Ok(None);
    };
    let source = LegacySnapshotRepairSource {
        user_id: current.user_id.clone(),
        revision: current.revision,
        content_sha256: sha256_hex(current.app_data_json.as_bytes()),
    };
    if !legacy_snapshot_repair_backup_contains_source(database_path, &backup, &source)? {
        return Ok(None);
    }
    Ok(Some(backup))
}

fn legacy_snapshot_repair_backup_contains_source(
    database_path: &Path,
    backup: &LegacySnapshotRepairBackupIdentity,
    source: &LegacySnapshotRepairSource,
) -> StoreResult<bool> {
    if backup.size_bytes <= 0
        || !matches!(backup.schema_version, 12 | 13)
        || backup.created_at_epoch_millis < 0
        || !valid_lowercase_opaque_identifier(&backup.sha256)
    {
        return Ok(false);
    }
    let controlled_name = controlled_backup_file_name(Path::new(&backup.file_name))?;
    if controlled_name != backup.file_name {
        return Ok(false);
    }
    let live_database_path = fs::canonicalize(database_path)?;
    let parent = live_database_path.parent().ok_or_else(|| {
        StoreError::Integrity("live database has no parent directory".to_string())
    })?;
    let report = VerifiedBackupReport {
        destination: parent.join(&backup.file_name),
        size_bytes: u64::try_from(backup.size_bytes)
            .map_err(|_| StoreError::Integrity("backup size overflow".to_string()))?,
        sha256: backup.sha256.clone(),
        created_at_epoch_millis: backup.created_at_epoch_millis,
        server_instance_id: String::new(),
    };
    if preschema_privacy::contains_source(database_path, backup, source)? {
        return Ok(true);
    }
    let verified = read_verified_pre_schema_snapshot_manifest(&report, backup.schema_version)?;
    Ok(verified.backup == *backup && verified.sources.contains(source))
}

fn app_data_resource_footprint_strictly_decreased(
    current: &str,
    incoming: &str,
) -> StoreResult<bool> {
    if incoming.len() >= current.len() {
        return Ok(false);
    }
    let current_ids = referenced_attachment_ids_tolerant(current)?;
    let incoming_ids = referenced_attachment_ids(incoming)?;
    Ok(incoming_ids.len() < current_ids.len())
}

fn record_legacy_snapshot_repair_history_omission(
    transaction: &Transaction<'_>,
    current: &AccountSnapshot,
    omitted_at_epoch_millis: i64,
    backup: &LegacySnapshotRepairBackupIdentity,
) -> StoreResult<()> {
    let source_content_sha256 = sha256_hex(current.app_data_json.as_bytes());
    let compressed_size_bytes =
        i64::try_from(compress_snapshot_content(current.app_data_json.as_bytes())?.len())
            .map_err(|_| StoreError::Integrity("snapshot content size overflow".to_string()))?;
    let details_json = serde_json::to_string(&serde_json::json!({
        "recoverySource": "verified_pre_schema_sqlite_backup",
        "sourceRevision": current.revision,
        "sourceContentSha256": source_content_sha256.clone(),
        "sourceUpdatedAtEpochMillis": current.updated_at_epoch_millis,
        "backupFileName": &backup.file_name,
        "backupSizeBytes": backup.size_bytes,
        "backupSha256": &backup.sha256,
        "backupSchemaVersion": backup.schema_version,
        "backupCreatedAtEpochMillis": backup.created_at_epoch_millis,
    }))?;
    let changed = transaction.execute(
        "INSERT INTO snapshot_history_prune_audit(
             user_id, revision, content_sha256, history_created_at_epoch_millis,
             pruned_at_epoch_millis, compressed_size_bytes, reason, details_json
         ) VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6, ?7)",
        params![
            current.user_id,
            current.revision,
            source_content_sha256,
            omitted_at_epoch_millis,
            compressed_size_bytes,
            "legacy_repair_source_preserved_in_verified_pre_schema_backup",
            details_json,
        ],
    )?;
    if changed != 1 {
        return Err(StoreError::Integrity(
            "legacy snapshot repair audit changed an unexpected row count".to_string(),
        ));
    }
    Ok(())
}

fn consume_legacy_snapshot_repair_allowance(
    transaction: &Transaction<'_>,
    current: &AccountSnapshot,
    backup: &LegacySnapshotRepairBackupIdentity,
) -> StoreResult<()> {
    let source_content_sha256 = sha256_hex(current.app_data_json.as_bytes());
    let changed = transaction.execute(
        "DELETE FROM legacy_snapshot_repair_allowances
         WHERE user_id = ?1
           AND source_revision = ?2
           AND source_content_sha256 = ?3
           AND backup_file_name = ?4
           AND backup_size_bytes = ?5
           AND backup_sha256 = ?6
           AND backup_schema_version = ?7
           AND backup_created_at_epoch_millis = ?8",
        params![
            current.user_id,
            current.revision,
            source_content_sha256,
            &backup.file_name,
            backup.size_bytes,
            &backup.sha256,
            backup.schema_version,
            backup.created_at_epoch_millis,
        ],
    )?;
    if changed != 1 {
        return Err(StoreError::Integrity(
            "legacy snapshot repair allowance consumption changed an unexpected row count"
                .to_string(),
        ));
    }
    Ok(())
}

fn snapshot_has_complete_recovery_copy(
    transaction: &Transaction<'_>,
    snapshot: &AccountSnapshot,
) -> StoreResult<bool> {
    let sha256 = sha256_hex(snapshot.app_data_json.as_bytes());
    let candidates = {
        let mut statement = transaction.prepare(
            "SELECT h.user_id, h.revision, h.created_at_epoch_millis,
                    c.sha256, c.compression, c.uncompressed_size_bytes,
                    c.compressed_size_bytes, c.content
             FROM account_snapshot_history h
             JOIN snapshot_contents c ON c.sha256 = h.content_sha256
             WHERE h.user_id = ?1 AND h.content_sha256 = ?2
               AND h.media_snapshot_complete = 1
             ORDER BY h.revision DESC",
        )?;
        let collected = statement
            .query_map(params![snapshot.user_id, sha256], snapshot_content_row)?
            .collect::<Result<Vec<_>, _>>()?;
        collected
    };
    if let Some(candidate) = candidates.into_iter().next() {
        let history = decode_snapshot_history(candidate)?;
        if history.app_data_json != snapshot.app_data_json {
            return Err(StoreError::Integrity(format!(
                "snapshot SHA-256 collision or divergent history for user {}",
                snapshot.user_id
            )));
        }
        load_complete_media_snapshot(
            transaction,
            &snapshot.user_id,
            history.revision,
            &snapshot.app_data_json,
        )?;
        return Ok(true);
    }
    Ok(false)
}

fn insert_snapshot_history_with_limit(
    transaction: &Transaction<'_>,
    snapshot: &AccountSnapshot,
    created_at_epoch_millis: i64,
    hard_limit_bytes: i64,
    media_policy: NewUserMediaPolicy,
) -> StoreResult<()> {
    let snapshot_sha256 = sha256_hex(snapshot.app_data_json.as_bytes());
    let existing_sha256 = transaction
        .query_row(
            "SELECT content_sha256 FROM account_snapshot_history
             WHERE user_id = ?1 AND revision = ?2",
            params![snapshot.user_id, snapshot.revision],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if let Some(existing_sha256) = existing_sha256 {
        if existing_sha256 != snapshot_sha256 {
            return Err(StoreError::Integrity(format!(
                "snapshot history diverged for user {} at revision {}",
                snapshot.user_id, snapshot.revision
            )));
        }
        ensure_snapshot_content(
            transaction,
            &snapshot_sha256,
            &snapshot.app_data_json,
            created_at_epoch_millis,
        )?;
        verify_snapshot_content_index(transaction)?;
        return Ok(());
    }

    let metadata_archivable =
        match validate_archivable_app_data_media_metadata(&snapshot.app_data_json) {
            Ok(()) => true,
            Err(StoreError::Integrity(_)) => false,
            Err(error) => return Err(error),
        };
    let capture_media_content = if metadata_archivable {
        match validate_snapshot_history_quota_with_limit(transaction, snapshot, hard_limit_bytes) {
            Ok(_) => true,
            Err(StoreError::SnapshotHistoryQuotaExceeded { .. }) => {
                validate_minimal_snapshot_history_quota_with_limit(
                    transaction,
                    snapshot,
                    hard_limit_bytes,
                )?;
                false
            }
            Err(error) => return Err(error),
        }
    } else {
        validate_minimal_snapshot_history_quota_with_limit(
            transaction,
            snapshot,
            hard_limit_bytes,
        )?;
        false
    };
    let referenced_ids = if metadata_archivable {
        referenced_attachment_ids(&snapshot.app_data_json)?
    } else {
        referenced_attachment_ids_tolerant(&snapshot.app_data_json)?
    };
    if media_policy == NewUserMediaPolicy::EnforceGrowth {
        validate_media_identity_batch_quota(transaction, &snapshot.user_id, &referenced_ids)?;
    }
    let referenced_count = u64::try_from(referenced_ids.len()).map_err(|_| {
        StoreError::Integrity("snapshot media history row count overflow".to_string())
    })?;
    let metadata_growth_bytes = (SNAPSHOT_HISTORY_ROW_OVERHEAD_BYTES as u64)
        .checked_add(
            referenced_count
                .checked_mul(SCHEMA_MIGRATION_INDEX_ROW_ESTIMATE_BYTES)
                .ok_or_else(|| {
                    StoreError::Integrity(
                        "snapshot media history byte projection overflow".to_string(),
                    )
                })?,
        )
        .ok_or_else(|| {
            StoreError::Integrity("snapshot history byte projection overflow".to_string())
        })?;
    ensure_snapshot_write_capacity(transaction, metadata_growth_bytes)?;
    ensure_snapshot_content(
        transaction,
        &snapshot_sha256,
        &snapshot.app_data_json,
        created_at_epoch_millis,
    )?;
    transaction.execute(
        "INSERT INTO account_snapshot_history(
             user_id, revision, content_sha256, created_at_epoch_millis
         ) VALUES (?1, ?2, ?3, ?4)",
        params![
            snapshot.user_id,
            snapshot.revision,
            snapshot_sha256,
            created_at_epoch_millis,
        ],
    )?;
    if capture_media_content {
        transaction.execute_batch("SAVEPOINT guarded_snapshot_media_capture;")?;
        match capture_media_snapshot(transaction, snapshot, created_at_epoch_millis) {
            Ok(()) => transaction.execute_batch("RELEASE guarded_snapshot_media_capture;")?,
            Err(error) if is_snapshot_media_capacity_error(&error) => {
                transaction.execute_batch(
                    "ROLLBACK TO guarded_snapshot_media_capture;
                     RELEASE guarded_snapshot_media_capture;",
                )?;
                capture_incomplete_media_snapshot(transaction, snapshot)?;
            }
            Err(error) => {
                transaction.execute_batch(
                    "ROLLBACK TO guarded_snapshot_media_capture;
                     RELEASE guarded_snapshot_media_capture;",
                )?;
                return Err(error);
            }
        }
    } else {
        capture_incomplete_media_snapshot(transaction, snapshot)?;
    }
    verify_snapshot_content_index(transaction)?;
    Ok(())
}

#[derive(Debug)]
struct SnapshotContentRow {
    user_id: String,
    revision: i64,
    created_at_epoch_millis: i64,
    sha256: String,
    compression: String,
    uncompressed_size_bytes: i64,
    compressed_size_bytes: i64,
    content: Vec<u8>,
}

fn snapshot_content_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SnapshotContentRow> {
    Ok(SnapshotContentRow {
        user_id: row.get(0)?,
        revision: row.get(1)?,
        created_at_epoch_millis: row.get(2)?,
        sha256: row.get(3)?,
        compression: row.get(4)?,
        uncompressed_size_bytes: row.get(5)?,
        compressed_size_bytes: row.get(6)?,
        content: row.get(7)?,
    })
}

fn compress_snapshot_content(raw: &[u8]) -> StoreResult<Vec<u8>> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(6));
    encoder.write_all(raw)?;
    encoder.finish().map_err(StoreError::from)
}

fn decode_snapshot_content(stored: &SnapshotContentRow) -> StoreResult<String> {
    if stored.compression != SNAPSHOT_CONTENT_COMPRESSION {
        return Err(StoreError::Integrity(format!(
            "unsupported snapshot compression {} for {}",
            stored.compression, stored.sha256
        )));
    }
    if stored.uncompressed_size_bytes < 0
        || stored.compressed_size_bytes < 0
        || stored.compressed_size_bytes as usize != stored.content.len()
    {
        return Err(StoreError::Integrity(format!(
            "snapshot content size metadata is invalid for {}",
            stored.sha256
        )));
    }
    let expected_size = stored.uncompressed_size_bytes as u64;
    let mut decoder = ZlibDecoder::new(stored.content.as_slice()).take(expected_size + 1);
    let mut raw = Vec::new();
    decoder.read_to_end(&mut raw)?;
    if raw.len() as u64 != expected_size {
        return Err(StoreError::Integrity(format!(
            "snapshot content decompressed size mismatch for {}",
            stored.sha256
        )));
    }
    if !valid_sha256_hex(&stored.sha256) || !stored.sha256.eq_ignore_ascii_case(&sha256_hex(&raw)) {
        return Err(StoreError::Integrity(format!(
            "snapshot content SHA-256 mismatch for {}",
            stored.sha256
        )));
    }
    String::from_utf8(raw).map_err(|_| {
        StoreError::Integrity(format!(
            "snapshot content is not UTF-8 for {}",
            stored.sha256
        ))
    })
}

fn decode_snapshot_history(stored: SnapshotContentRow) -> StoreResult<AccountSnapshotHistory> {
    let app_data_json = decode_snapshot_content(&stored)?;
    let snapshot = AccountSnapshotHistory {
        user_id: stored.user_id,
        revision: stored.revision,
        app_data_json,
        created_at_epoch_millis: stored.created_at_epoch_millis,
        sha256: stored.sha256,
    };
    verify_snapshot_history_entry(&snapshot)?;
    Ok(snapshot)
}

fn ensure_snapshot_content(
    transaction: &Transaction<'_>,
    sha256: &str,
    app_data_json: &str,
    created_at_epoch_millis: i64,
) -> StoreResult<()> {
    let existing = transaction
        .query_row(
            "SELECT '' AS user_id, 0 AS revision, 0 AS created_at_epoch_millis,
                    sha256, compression, uncompressed_size_bytes,
                    compressed_size_bytes, content
             FROM snapshot_contents WHERE sha256 = ?1",
            params![sha256],
            snapshot_content_row,
        )
        .optional()?;
    if let Some(existing) = existing {
        if decode_snapshot_content(&existing)? != app_data_json {
            return Err(StoreError::Integrity(format!(
                "snapshot SHA-256 collision or divergent content for {sha256}"
            )));
        }
        return Ok(());
    }

    let compressed = compress_snapshot_content(app_data_json.as_bytes())?;
    ensure_snapshot_write_capacity(transaction, compressed.len() as u64)?;
    transaction.execute(
        "INSERT INTO snapshot_contents(
             sha256, compression, uncompressed_size_bytes, compressed_size_bytes,
             content, reference_count, created_at_epoch_millis
         ) VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)",
        params![
            sha256,
            SNAPSHOT_CONTENT_COMPRESSION,
            app_data_json.len() as i64,
            compressed.len() as i64,
            compressed,
            created_at_epoch_millis,
        ],
    )?;
    Ok(())
}

fn snapshot_history_warning(usage_bytes: i64) -> SnapshotHistoryWarning {
    if usage_bytes > SNAPSHOT_HISTORY_HARD_LIMIT_BYTES_PER_USER {
        SnapshotHistoryWarning::OverLimit
    } else if usage_bytes >= SNAPSHOT_HISTORY_CRITICAL_BYTES_PER_USER {
        SnapshotHistoryWarning::Critical
    } else if usage_bytes >= SNAPSHOT_HISTORY_WARNING_BYTES_PER_USER {
        SnapshotHistoryWarning::ApproachingLimit
    } else {
        SnapshotHistoryWarning::None
    }
}

fn snapshot_user_distinct_content_bytes(
    connection: &Connection,
    user_id: &str,
) -> StoreResult<i64> {
    let json_bytes: i64 = connection.query_row(
        "SELECT COALESCE(SUM(c.compressed_size_bytes), 0)
             FROM snapshot_contents c
             WHERE c.sha256 IN (
                 SELECT DISTINCT content_sha256
                 FROM account_snapshot_history
                 WHERE user_id = ?1
             )",
        params![user_id],
        |row| row.get(0),
    )?;
    let media_bytes = connection.query_row(
        "SELECT COALESCE(SUM(c.size_bytes), 0)
         FROM media_snapshot_contents c
         WHERE c.sha256 IN (
             SELECT DISTINCT content_sha256
             FROM account_snapshot_media_history
             WHERE user_id = ?1 AND content_sha256 IS NOT NULL
         )",
        params![user_id],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(json_bytes.saturating_add(media_bytes))
}

fn snapshot_history_charged_bytes(connection: &Connection, user_id: &str) -> StoreResult<i64> {
    let content_bytes = snapshot_user_distinct_content_bytes(connection, user_id)?;
    let history_rows = connection.query_row(
        "SELECT COUNT(*) FROM account_snapshot_history WHERE user_id = ?1",
        params![user_id],
        |row| row.get::<_, i64>(0),
    )?;
    let media_rows = connection.query_row(
        "SELECT COUNT(*) FROM account_snapshot_media_history WHERE user_id = ?1",
        params![user_id],
        |row| row.get::<_, i64>(0),
    )?;
    let media_metadata_bytes = connection.query_row(
        "SELECT COALESCE(SUM(
             length(CAST(attachment_id AS BLOB))
             + length(CAST(declared_sha256 AS BLOB))
             + length(CAST(mime_type AS BLOB))
             + length(CAST(missing_reason AS BLOB))
         ), 0)
         FROM account_snapshot_media_history WHERE user_id = ?1",
        params![user_id],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(content_bytes
        .saturating_add(history_rows.saturating_mul(SNAPSHOT_HISTORY_ROW_OVERHEAD_BYTES))
        .saturating_add(media_rows.saturating_mul(SNAPSHOT_MEDIA_HISTORY_ROW_OVERHEAD_BYTES))
        .saturating_add(media_metadata_bytes))
}

fn snapshot_history_projection(
    connection: &Connection,
    snapshot: &AccountSnapshot,
) -> StoreResult<(i64, i64)> {
    let usage_bytes = snapshot_history_charged_bytes(connection, &snapshot.user_id)?;
    let snapshot_sha256 = sha256_hex(snapshot.app_data_json.as_bytes());
    if let Some(existing_sha256) = connection
        .query_row(
            "SELECT content_sha256 FROM account_snapshot_history
             WHERE user_id = ?1 AND revision = ?2",
            params![snapshot.user_id, snapshot.revision],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        if existing_sha256 != snapshot_sha256 {
            return Err(StoreError::Integrity(format!(
                "snapshot history diverged for user {} at revision {}",
                snapshot.user_id, snapshot.revision
            )));
        }
        return Ok((usage_bytes, usage_bytes));
    }

    let json_already_referenced = connection.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM account_snapshot_history
             WHERE user_id = ?1 AND content_sha256 = ?2
         )",
        params![snapshot.user_id, snapshot_sha256],
        |row| row.get::<_, bool>(0),
    )?;
    let json_increment = if json_already_referenced {
        0
    } else {
        compress_snapshot_content(snapshot.app_data_json.as_bytes())?.len() as i64
    };

    let expected_ids = referenced_attachment_ids(&snapshot.app_data_json)?;
    let expectations = referenced_media_expectations(&snapshot.app_data_json)?;
    let active_media = {
        let mut statement = connection.prepare(
            "SELECT m.attachment_id, m.sha256, m.mime_type, m.size_bytes
             FROM note_media m
             LEFT JOIN note_media_tombstones t
               ON t.user_id = m.user_id AND t.attachment_id = m.attachment_id
             WHERE m.user_id = ?1
               AND m.deleted_at_epoch_millis IS NULL
               AND (t.deleted_revision_epoch_millis IS NULL
                    OR m.updated_at_epoch_millis > t.deleted_revision_epoch_millis)",
        )?;
        let collected = statement
            .query_map(params![snapshot.user_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        collected
    };
    let mut new_media_sha256 = HashSet::new();
    let mut captured_attachment_ids = HashSet::new();
    let mut media_increment = 0_i64;
    for (attachment_id, sha256, mime_type, size_bytes) in active_media {
        if !expected_ids.contains(&attachment_id) {
            continue;
        }
        let matches = expectations.get(&attachment_id).is_some_and(|expected| {
            valid_sha256_hex(&expected.sha256)
                && expected.size_bytes_declared
                && expected.size_bytes >= 0
                && expected.size_bytes <= MAX_MEDIA_BYTES as i64
                && !expected.mime_type.is_empty()
                && expected.sha256.eq_ignore_ascii_case(&sha256)
                && expected.size_bytes == size_bytes
                && expected.mime_type.eq_ignore_ascii_case(mime_type.trim())
        });
        if !matches {
            continue;
        }
        captured_attachment_ids.insert(attachment_id);
        if !new_media_sha256.insert(sha256.clone()) {
            continue;
        }
        let already_referenced = connection.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM account_snapshot_media_history
                 WHERE user_id = ?1 AND content_sha256 = ?2
             )",
            params![snapshot.user_id, sha256],
            |row| row.get::<_, bool>(0),
        )?;
        if !already_referenced {
            media_increment = media_increment.saturating_add(size_bytes.max(0));
        }
    }
    let mut metadata_increment = 0_i64;
    for attachment_id in &expected_ids {
        let expectation = expectations.get(attachment_id);
        let has_expected_metadata = expectation.is_some_and(|expected| {
            valid_sha256_hex(&expected.sha256)
                && expected.size_bytes_declared
                && (0..=MAX_MEDIA_BYTES as i64).contains(&expected.size_bytes)
                && !expected.mime_type.is_empty()
        });
        let declared_sha256_bytes = expectation
            .filter(|expected| valid_sha256_hex(&expected.sha256))
            .map(|expected| expected.sha256.len())
            .unwrap_or(0);
        let mime_type_bytes = expectation
            .map(|expected| expected.mime_type.len())
            .unwrap_or(0);
        let missing_reason_bytes = if captured_attachment_ids.contains(attachment_id) {
            0
        } else if has_expected_metadata {
            "content_unavailable_at_snapshot".len()
        } else {
            "reference_metadata_unavailable_at_snapshot".len()
        };
        let row_metadata_bytes = attachment_id
            .len()
            .checked_add(declared_sha256_bytes)
            .and_then(|value| value.checked_add(mime_type_bytes))
            .and_then(|value| value.checked_add(missing_reason_bytes))
            .ok_or_else(|| {
                StoreError::Integrity("snapshot media metadata projection overflow".to_string())
            })?;
        metadata_increment = metadata_increment
            .checked_add(i64::try_from(row_metadata_bytes).map_err(|_| {
                StoreError::Integrity("snapshot media metadata projection overflow".to_string())
            })?)
            .ok_or_else(|| {
                StoreError::Integrity("snapshot media metadata projection overflow".to_string())
            })?;
    }
    let history_row_increment = SNAPSHOT_HISTORY_ROW_OVERHEAD_BYTES
        .saturating_add(
            (expected_ids.len() as i64).saturating_mul(SNAPSHOT_MEDIA_HISTORY_ROW_OVERHEAD_BYTES),
        )
        .saturating_add(metadata_increment);
    let projected_bytes = usage_bytes
        .saturating_add(json_increment)
        .saturating_add(media_increment)
        .saturating_add(history_row_increment);
    Ok((usage_bytes, projected_bytes))
}

fn validate_snapshot_history_quota_with_limit(
    transaction: &Transaction<'_>,
    snapshot: &AccountSnapshot,
    hard_limit_bytes: i64,
) -> StoreResult<(i64, i64)> {
    if hard_limit_bytes < 1 {
        return Err(StoreError::Integrity(
            "snapshot history quota must be positive".to_string(),
        ));
    }
    let (usage_bytes, projected_bytes) = snapshot_history_projection(transaction, snapshot)?;
    if projected_bytes > hard_limit_bytes && projected_bytes > usage_bytes {
        return Err(StoreError::SnapshotHistoryQuotaExceeded {
            usage_bytes,
            projected_bytes,
            limit_bytes: hard_limit_bytes,
        });
    }
    Ok((usage_bytes, projected_bytes))
}

fn validate_minimal_snapshot_history_quota_with_limit(
    transaction: &Transaction<'_>,
    snapshot: &AccountSnapshot,
    hard_limit_bytes: i64,
) -> StoreResult<(i64, i64)> {
    if hard_limit_bytes < 1 {
        return Err(StoreError::Integrity(
            "snapshot history quota must be positive".to_string(),
        ));
    }
    let usage_bytes = snapshot_history_charged_bytes(transaction, &snapshot.user_id)?;
    let snapshot_sha256 = sha256_hex(snapshot.app_data_json.as_bytes());
    let json_already_referenced = transaction.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM account_snapshot_history
             WHERE user_id = ?1 AND content_sha256 = ?2
         )",
        params![snapshot.user_id, snapshot_sha256],
        |row| row.get::<_, bool>(0),
    )?;
    let json_increment = if json_already_referenced {
        0_i64
    } else {
        i64::try_from(compress_snapshot_content(snapshot.app_data_json.as_bytes())?.len())
            .map_err(|_| StoreError::Integrity("snapshot content size overflow".to_string()))?
    };
    let expected_ids = referenced_attachment_ids_tolerant(&snapshot.app_data_json)?;
    let missing_reason_bytes = i64::try_from("reference_metadata_unavailable_at_snapshot".len())
        .map_err(|_| StoreError::Integrity("snapshot metadata size overflow".to_string()))?;
    let mut metadata_increment = 0_i64;
    for attachment_id in &expected_ids {
        let attachment_bytes = i64::try_from(attachment_id.len())
            .map_err(|_| StoreError::Integrity("snapshot metadata size overflow".to_string()))?;
        metadata_increment = metadata_increment
            .checked_add(SNAPSHOT_MEDIA_HISTORY_ROW_OVERHEAD_BYTES)
            .and_then(|value| value.checked_add(attachment_bytes))
            .and_then(|value| value.checked_add(missing_reason_bytes))
            .ok_or_else(|| StoreError::Integrity("snapshot metadata size overflow".to_string()))?;
    }
    let projected_bytes = usage_bytes
        .checked_add(json_increment)
        .and_then(|value| value.checked_add(SNAPSHOT_HISTORY_ROW_OVERHEAD_BYTES))
        .and_then(|value| value.checked_add(metadata_increment))
        .ok_or_else(|| StoreError::Integrity("snapshot history projection overflow".to_string()))?;
    if projected_bytes > hard_limit_bytes && projected_bytes > usage_bytes {
        return Err(StoreError::SnapshotHistoryQuotaExceeded {
            usage_bytes,
            projected_bytes,
            limit_bytes: hard_limit_bytes,
        });
    }
    Ok((usage_bytes, projected_bytes))
}

fn validate_prospective_current_archivability(
    transaction: &Transaction<'_>,
    user_id: &str,
    revision: i64,
    app_data_json: &str,
    updated_at_epoch_millis: i64,
) -> StoreResult<()> {
    let prospective = AccountSnapshot {
        user_id: user_id.to_string(),
        app_data_json: app_data_json.to_string(),
        revision,
        updated_at_epoch_millis,
    };
    let (usage_bytes, projected_bytes) = validate_snapshot_history_quota_with_limit(
        transaction,
        &prospective,
        SNAPSHOT_HISTORY_HARD_LIMIT_BYTES_PER_USER,
    )?;
    let projected_growth = projected_bytes.saturating_sub(usage_bytes);
    let projected_growth = u64::try_from(projected_growth).map_err(|_| {
        StoreError::Integrity("prospective snapshot archive size overflow".to_string())
    })?;
    ensure_snapshot_write_capacity(transaction, projected_growth)
}

fn validate_media_history_repair_quota(
    transaction: &Transaction<'_>,
    user_id: &str,
    sha256: &str,
    content_size_bytes: i64,
) -> StoreResult<()> {
    let usage_bytes = snapshot_history_charged_bytes(transaction, user_id)?;
    let already_referenced = transaction.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM account_snapshot_media_history
             WHERE user_id = ?1 AND content_sha256 = ?2
         )",
        params![user_id, sha256],
        |row| row.get::<_, bool>(0),
    )?;
    let projected_bytes = usage_bytes.saturating_add(if already_referenced {
        0
    } else {
        content_size_bytes.max(0)
    });
    if projected_bytes > SNAPSHOT_HISTORY_HARD_LIMIT_BYTES_PER_USER && projected_bytes > usage_bytes
    {
        return Err(StoreError::SnapshotHistoryQuotaExceeded {
            usage_bytes,
            projected_bytes,
            limit_bytes: SNAPSHOT_HISTORY_HARD_LIMIT_BYTES_PER_USER,
        });
    }
    Ok(())
}

fn referenced_attachment_ids(app_data_json: &str) -> StoreResult<HashSet<String>> {
    let ids = collected_referenced_attachment_ids(app_data_json)?;
    if ids.iter().any(|id| !valid_media_attachment_id(id)) {
        return Err(StoreError::Integrity(
            "app data contains an invalid media attachment identity".to_string(),
        ));
    }
    Ok(ids)
}

fn referenced_attachment_ids_tolerant(app_data_json: &str) -> StoreResult<HashSet<String>> {
    let mut ids = collected_referenced_attachment_ids(app_data_json)?;
    ids.retain(|id| valid_media_attachment_id(id));
    Ok(ids)
}

fn collected_referenced_attachment_ids(app_data_json: &str) -> StoreResult<HashSet<String>> {
    collected_referenced_attachment_ids_at_format(app_data_json, true)
}

fn collected_referenced_attachment_ids_at_format(
    app_data_json: &str,
    include_typed_conflicts: bool,
) -> StoreResult<HashSet<String>> {
    if app_data_json.trim().is_empty() {
        return Ok(HashSet::new());
    }
    let value = serde_json::from_str::<serde_json::Value>(app_data_json)?;
    let mut ids = HashSet::new();
    for_each_referenced_note_snapshot_at_format(&value, include_typed_conflicts, |snapshot, _| {
        collect_note_snapshot_attachment_ids(snapshot, &mut ids);
        Ok(())
    })?;
    Ok(ids)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReferencedMediaExpectation {
    sha256: String,
    mime_type: String,
    size_bytes: i64,
    size_bytes_declared: bool,
    updated_at_epoch_millis: i64,
    timestamp_from_current_attachment: bool,
}

fn referenced_media_expectations(
    app_data_json: &str,
) -> StoreResult<HashMap<String, ReferencedMediaExpectation>> {
    referenced_media_expectations_at_format(app_data_json, true)
}

fn referenced_media_expectations_at_format(
    app_data_json: &str,
    include_typed_conflicts: bool,
) -> StoreResult<HashMap<String, ReferencedMediaExpectation>> {
    if app_data_json.trim().is_empty() {
        return Ok(HashMap::new());
    }
    let value = serde_json::from_str::<serde_json::Value>(app_data_json)?;
    let mut expectations = HashMap::new();
    for_each_referenced_note_snapshot_at_format(
        &value,
        include_typed_conflicts,
        |snapshot, is_current| {
            collect_note_snapshot_attachment_expectations(snapshot, is_current, &mut expectations)
        },
    )?;
    Ok(expectations)
}

fn validated_note_conflict_payload(conflict: &serde_json::Value) -> Option<&serde_json::Value> {
    crate::app_data::validated_note_conflict_payload(conflict)
}

fn for_each_referenced_note_snapshot_at_format(
    app_data: &serde_json::Value,
    include_typed_conflicts: bool,
    mut visit: impl FnMut(&serde_json::Value, bool) -> StoreResult<()>,
) -> StoreResult<()> {
    for note in app_data
        .get("notes")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        visit(note, true)?;
        for history_field in ["revisions", "versions"] {
            for snapshot in note
                .get(history_field)
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
            {
                visit(snapshot, false)?;
            }
        }
    }
    if include_typed_conflicts {
        for conflict in app_data
            .get("syncConflictHistory")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(note) = validated_note_conflict_payload(conflict) else {
                continue;
            };
            // Conflict payloads are retained history, never current-reference
            // authority for restoration or deletion revision comparisons.
            visit(note, false)?;
            for field in ["revisions", "versions"] {
                for snapshot in note
                    .get(field)
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    visit(snapshot, false)?;
                }
            }
        }
    }
    Ok(())
}

fn collect_note_snapshot_attachment_expectations(
    snapshot: &serde_json::Value,
    is_current_attachment_scope: bool,
    expectations: &mut HashMap<String, ReferencedMediaExpectation>,
) -> StoreResult<()> {
    for attachment in snapshot
        .get("attachments")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(id) = attachment
            .get("id")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
        else {
            continue;
        };
        if !valid_media_attachment_id(id) {
            return Err(StoreError::Integrity(
                "app data contains an invalid media attachment identity".to_string(),
            ));
        }
        let declared_size_bytes = attachment
            .get("sizeBytes")
            .and_then(serde_json::Value::as_i64);
        let expectation = ReferencedMediaExpectation {
            sha256: attachment
                .get("sha256")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase(),
            mime_type: attachment
                .get("mimeType")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string(),
            size_bytes: declared_size_bytes.unwrap_or(0),
            size_bytes_declared: declared_size_bytes.is_some(),
            updated_at_epoch_millis: attachment
                .get("updatedAtEpochMillis")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0),
            timestamp_from_current_attachment: is_current_attachment_scope,
        };
        if !expectation.sha256.is_empty() && !valid_sha256_hex(&expectation.sha256) {
            return Err(StoreError::Integrity(
                "app data contains an invalid media SHA-256".to_string(),
            ));
        }
        if !expectation.mime_type.is_empty() && !valid_media_mime_type(&expectation.mime_type) {
            return Err(StoreError::Integrity(
                "app data contains an invalid media MIME type".to_string(),
            ));
        }
        if expectation.size_bytes_declared
            && !(0..=MAX_MEDIA_BYTES as i64).contains(&expectation.size_bytes)
        {
            return Err(StoreError::Integrity(
                "app data contains an invalid media size".to_string(),
            ));
        }
        if let Some(existing) = expectations.get_mut(id) {
            let sha_conflicts = !existing.sha256.is_empty()
                && !expectation.sha256.is_empty()
                && !existing.sha256.eq_ignore_ascii_case(&expectation.sha256);
            let mime_conflicts = !existing.mime_type.is_empty()
                && !expectation.mime_type.is_empty()
                && !existing
                    .mime_type
                    .eq_ignore_ascii_case(&expectation.mime_type);
            let size_conflicts = existing.size_bytes_declared
                && expectation.size_bytes_declared
                && existing.size_bytes != expectation.size_bytes;
            if sha_conflicts || mime_conflicts || size_conflicts {
                return Err(StoreError::Integrity(format!(
                    "attachment {id} has conflicting historical content identity"
                )));
            }
            if existing.sha256.is_empty() && !expectation.sha256.is_empty() {
                existing.sha256 = expectation.sha256;
            }
            if existing.mime_type.is_empty() && !expectation.mime_type.is_empty() {
                existing.mime_type = expectation.mime_type;
            }
            if !existing.size_bytes_declared && expectation.size_bytes_declared {
                existing.size_bytes = expectation.size_bytes;
                existing.size_bytes_declared = true;
            }
            if is_current_attachment_scope {
                if existing.timestamp_from_current_attachment {
                    existing.updated_at_epoch_millis = existing
                        .updated_at_epoch_millis
                        .max(expectation.updated_at_epoch_millis);
                } else {
                    existing.updated_at_epoch_millis = expectation.updated_at_epoch_millis;
                }
                existing.timestamp_from_current_attachment = true;
            } else if !existing.timestamp_from_current_attachment {
                existing.updated_at_epoch_millis = existing
                    .updated_at_epoch_millis
                    .max(expectation.updated_at_epoch_millis);
            }
        } else {
            expectations.insert(id.to_string(), expectation);
        }
    }
    Ok(())
}

fn collect_note_snapshot_attachment_ids(snapshot: &serde_json::Value, ids: &mut HashSet<String>) {
    for attachment in snapshot
        .get("attachments")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(id) = attachment
            .get("id")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            ids.insert(id.to_string());
        }
    }
    for id in snapshot
        .get("attachmentIds")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        ids.insert(id.to_string());
    }
    if let Some(document) = snapshot.get("document") {
        collect_note_document_attachment_ids(document, ids);
    }
}

fn collect_note_document_attachment_ids(value: &serde_json::Value, ids: &mut HashSet<String>) {
    match value {
        serde_json::Value::Object(object) => {
            if let Some(id) = object
                .get("attachmentId")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
            {
                ids.insert(id.to_string());
            }
            for child in object.values() {
                collect_note_document_attachment_ids(child, ids);
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                collect_note_document_attachment_ids(child, ids);
            }
        }
        _ => {}
    }
}

fn reconcile_media_histories_for_schema_v12(
    transaction: &Transaction<'_>,
    now_epoch_millis: i64,
    include_typed_conflicts: bool,
) -> StoreResult<()> {
    if current_schema_version(transaction)? >= 16 {
        return snapshot_media::reconcile_all(transaction, now_epoch_millis);
    }
    let history_keys = {
        let mut statement = transaction.prepare(
            "SELECT user_id, revision, media_snapshot_complete
             FROM account_snapshot_history
             ORDER BY user_id, revision",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)? == 1,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };

    for (user_id, revision, was_complete) in history_keys {
        let stored = transaction.query_row(
            "SELECT h.user_id, h.revision, h.created_at_epoch_millis,
                    c.sha256, c.compression, c.uncompressed_size_bytes,
                    c.compressed_size_bytes, c.content
             FROM account_snapshot_history h
             JOIN snapshot_contents c ON c.sha256 = h.content_sha256
             WHERE h.user_id = ?1 AND h.revision = ?2",
            params![user_id, revision],
            snapshot_content_row,
        )?;
        let history = decode_snapshot_history(stored)?;
        let mut expected_ids = collected_referenced_attachment_ids_at_format(
            &history.app_data_json,
            include_typed_conflicts,
        )?;
        expected_ids.retain(|id| valid_media_attachment_id(id));
        let expectations = match referenced_media_expectations_at_format(
            &history.app_data_json,
            include_typed_conflicts,
        ) {
            Ok(expectations) => Some(expectations),
            Err(StoreError::Integrity(_)) => None,
            Err(error) => return Err(error),
        };
        let existing_manifests = {
            let mut statement = transaction.prepare(
                "SELECT attachment_id, content_sha256, declared_sha256, mime_type,
                        size_bytes, updated_at_epoch_millis, missing_reason
                 FROM account_snapshot_media_history
                 WHERE user_id = ?1 AND account_revision = ?2
                 ORDER BY attachment_id",
            )?;
            let rows = statement.query_map(params![history.user_id, history.revision], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    (
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, String>(6)?,
                    ),
                ))
            })?;
            rows.collect::<Result<HashMap<_, _>, _>>()?
        };
        let mut referenced_ids = expected_ids.into_iter().collect::<Vec<_>>();
        referenced_ids.sort();
        let mut can_be_complete = expectations.is_some();

        for attachment_id in referenced_ids {
            let expectation = expectations
                .as_ref()
                .and_then(|expectations| expectations.get(&attachment_id));
            let valid_expectation = expectation.filter(|expected| {
                valid_sha256_hex(&expected.sha256)
                    && expected.size_bytes_declared
                    && !expected.mime_type.is_empty()
                    && expected.size_bytes >= 0
                    && expected.size_bytes <= MAX_MEDIA_BYTES as i64
            });
            let has_valid_identity = valid_expectation.is_some();
            let existing_manifest = existing_manifests.get(&attachment_id);
            if let Some((
                Some(content_sha256),
                declared_sha256,
                mime_type,
                size_bytes,
                updated_at_epoch_millis,
                missing_reason,
            )) = existing_manifest
            {
                if !missing_reason.is_empty() {
                    can_be_complete = false;
                    continue;
                }
                if let Some(expected) = valid_expectation {
                    let identity_matches = content_sha256.eq_ignore_ascii_case(&expected.sha256)
                        && declared_sha256.eq_ignore_ascii_case(&expected.sha256)
                        && mime_type.trim().eq_ignore_ascii_case(&expected.mime_type)
                        && *size_bytes == expected.size_bytes;
                    let metadata_needs_update = declared_sha256 != &expected.sha256
                        || mime_type != &expected.mime_type
                        || *updated_at_epoch_millis != expected.updated_at_epoch_millis;
                    if identity_matches && metadata_needs_update {
                        let changed = transaction.execute(
                            "UPDATE account_snapshot_media_history
                             SET declared_sha256 = ?1, mime_type = ?2, size_bytes = ?3,
                                 updated_at_epoch_millis = ?4
                             WHERE user_id = ?5 AND account_revision = ?6
                               AND attachment_id = ?7 AND content_sha256 IS NOT NULL",
                            params![
                                expected.sha256,
                                expected.mime_type,
                                expected.size_bytes,
                                expected.updated_at_epoch_millis,
                                history.user_id,
                                history.revision,
                                attachment_id,
                            ],
                        )?;
                        if changed != 1 {
                            return Err(StoreError::Integrity(format!(
                                "schema v12 media reconciliation normalized an unexpected row count for user {} revision {} attachment {}",
                                history.user_id, history.revision, attachment_id
                            )));
                        }
                    }
                }
                continue;
            }
            let mut captured_from_live_media = false;
            if let Some(expected) = valid_expectation {
                let live_media = transaction
                    .query_row(
                        "SELECT m.sha256, m.mime_type, m.size_bytes, m.content
                         FROM note_media m
                         LEFT JOIN note_media_tombstones t
                           ON t.user_id = m.user_id AND t.attachment_id = m.attachment_id
                         WHERE m.user_id = ?1 AND m.attachment_id = ?2
                           AND m.deleted_at_epoch_millis IS NULL
                           AND (t.deleted_revision_epoch_millis IS NULL
                                OR m.updated_at_epoch_millis > t.deleted_revision_epoch_millis)",
                        params![history.user_id, attachment_id],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, i64>(2)?,
                                row.get::<_, Vec<u8>>(3)?,
                            ))
                        },
                    )
                    .optional()?;
                if let Some((live_sha256, live_mime_type, live_size_bytes, content)) = live_media {
                    let live_matches = expected.sha256.eq_ignore_ascii_case(&live_sha256)
                        && expected
                            .mime_type
                            .eq_ignore_ascii_case(live_mime_type.trim())
                        && expected.size_bytes == live_size_bytes
                        && content.len() as i64 == live_size_bytes
                        && expected.sha256.eq_ignore_ascii_case(&sha256_hex(&content));
                    if live_matches {
                        let content_available = migration_media_snapshot_content_available(
                            ensure_media_snapshot_content(
                                transaction,
                                &expected.sha256,
                                &content,
                                now_epoch_millis,
                            ),
                        )?;
                        if content_available {
                            let changed = if existing_manifest.is_some() {
                                transaction.execute(
                                    "UPDATE account_snapshot_media_history
                                     SET content_sha256 = ?1, declared_sha256 = ?1,
                                         mime_type = ?2, size_bytes = ?3,
                                         updated_at_epoch_millis = ?4, missing_reason = ''
                                     WHERE user_id = ?5 AND account_revision = ?6
                                       AND attachment_id = ?7 AND content_sha256 IS NULL",
                                    params![
                                        expected.sha256,
                                        expected.mime_type,
                                        expected.size_bytes,
                                        expected.updated_at_epoch_millis,
                                        history.user_id,
                                        history.revision,
                                        attachment_id,
                                    ],
                                )?
                            } else {
                                transaction.execute(
                                    "INSERT INTO account_snapshot_media_history(
                                         user_id, account_revision, attachment_id, content_sha256,
                                         declared_sha256, mime_type, size_bytes,
                                         updated_at_epoch_millis, missing_reason
                                     ) VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6, ?7, '')",
                                    params![
                                        history.user_id,
                                        history.revision,
                                        attachment_id,
                                        expected.sha256,
                                        expected.mime_type,
                                        expected.size_bytes,
                                        expected.updated_at_epoch_millis,
                                    ],
                                )?
                            };
                            if changed != 1 {
                                return Err(StoreError::Integrity(format!(
                                    "schema v12 media reconciliation captured an unexpected row count for user {} revision {} attachment {}",
                                    history.user_id, history.revision, attachment_id
                                )));
                            }
                            captured_from_live_media = true;
                        }
                    }
                }
            }

            if !captured_from_live_media {
                can_be_complete = false;
                let declared_sha256 = valid_expectation
                    .map(|expected| expected.sha256.clone())
                    .unwrap_or_default();
                let mime_type = valid_expectation
                    .map(|expected| expected.mime_type.clone())
                    .unwrap_or_default();
                let size_bytes = valid_expectation
                    .map(|expected| expected.size_bytes)
                    .unwrap_or(0);
                let updated_at_epoch_millis = valid_expectation
                    .map(|expected| expected.updated_at_epoch_millis)
                    .unwrap_or(0);
                let missing_reason = if has_valid_identity {
                    "content_unavailable_at_snapshot"
                } else {
                    "reference_metadata_unavailable_at_snapshot"
                };
                let changed = if existing_manifest.is_some() {
                    transaction.execute(
                        "UPDATE account_snapshot_media_history
                         SET declared_sha256 = ?1, mime_type = ?2, size_bytes = ?3,
                             updated_at_epoch_millis = ?4, missing_reason = ?5
                         WHERE user_id = ?6 AND account_revision = ?7
                           AND attachment_id = ?8 AND content_sha256 IS NULL",
                        params![
                            declared_sha256,
                            mime_type,
                            size_bytes,
                            updated_at_epoch_millis,
                            missing_reason,
                            history.user_id,
                            history.revision,
                            attachment_id,
                        ],
                    )?
                } else {
                    transaction.execute(
                        "INSERT INTO account_snapshot_media_history(
                             user_id, account_revision, attachment_id, content_sha256,
                             declared_sha256, mime_type, size_bytes,
                             updated_at_epoch_millis, missing_reason
                         ) VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, ?7, ?8)",
                        params![
                            history.user_id,
                            history.revision,
                            attachment_id,
                            declared_sha256,
                            mime_type,
                            size_bytes,
                            updated_at_epoch_millis,
                            missing_reason,
                        ],
                    )?
                };
                if changed != 1 {
                    return Err(StoreError::Integrity(format!(
                        "schema v12 media reconciliation preserved an unexpected evidence row count for user {} revision {} attachment {}",
                        history.user_id, history.revision, attachment_id
                    )));
                }
            }
        }

        let mut complete_flag = was_complete;
        if can_be_complete && !complete_flag {
            let changed = transaction.execute(
                "UPDATE account_snapshot_history
                 SET media_snapshot_complete = 1
                 WHERE user_id = ?1 AND revision = ?2 AND media_snapshot_complete = 0",
                params![history.user_id, history.revision],
            )?;
            if changed != 1 {
                return Err(StoreError::Integrity(format!(
                    "schema v12 media reconciliation promoted an unexpected history row count for user {} revision {}",
                    history.user_id, history.revision
                )));
            }
            complete_flag = true;
        }
        if can_be_complete {
            match load_complete_media_snapshot(
                transaction,
                &history.user_id,
                history.revision,
                &history.app_data_json,
            ) {
                Ok(_) => {}
                Err(StoreError::Integrity(_)) | Err(StoreError::NotFound(_)) => {
                    can_be_complete = false;
                }
                Err(error) => return Err(error),
            }
        }
        if complete_flag && !can_be_complete {
            let changed = transaction.execute(
                "UPDATE account_snapshot_history
                 SET media_snapshot_complete = 0
                 WHERE user_id = ?1 AND revision = ?2 AND media_snapshot_complete = 1",
                params![history.user_id, history.revision],
            )?;
            if changed != 1 {
                return Err(StoreError::Integrity(format!(
                    "schema v12 media reconciliation changed an unexpected history row count for user {} revision {}",
                    history.user_id, history.revision
                )));
            }
        }
    }
    Ok(())
}

fn ensure_media_snapshot_content(
    transaction: &Transaction<'_>,
    sha256: &str,
    content: &[u8],
    created_at_epoch_millis: i64,
) -> StoreResult<()> {
    ensure_media_snapshot_content_with_limit(
        transaction,
        sha256,
        content,
        created_at_epoch_millis,
        MAX_MEDIA_RETAINED_BYTES_GLOBAL,
    )
}

fn migration_media_snapshot_content_available(result: StoreResult<()>) -> StoreResult<bool> {
    match result {
        Ok(()) => Ok(true),
        Err(StoreError::Integrity(_))
        | Err(StoreError::Io(_))
        | Err(StoreError::MediaServerRetainedQuotaExceeded { .. })
        | Err(StoreError::DiskReserveExceeded { .. }) => Ok(false),
        Err(error) => Err(error),
    }
}

fn ensure_media_snapshot_content_with_limit(
    transaction: &Transaction<'_>,
    sha256: &str,
    content: &[u8],
    created_at_epoch_millis: i64,
    global_limit_bytes: i64,
) -> StoreResult<()> {
    let existing = transaction
        .query_row(
            "SELECT size_bytes, content FROM media_snapshot_contents WHERE sha256 = ?1",
            params![sha256],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )
        .optional()?;
    if let Some((size_bytes, existing_content)) = existing {
        if size_bytes != content.len() as i64
            || existing_content != content
            || !sha256.eq_ignore_ascii_case(&sha256_hex(content))
        {
            return Err(StoreError::Integrity(format!(
                "media snapshot content diverged for SHA-256 {sha256}"
            )));
        }
        return Ok(());
    }
    if !valid_sha256_hex(sha256) || !sha256.eq_ignore_ascii_case(&sha256_hex(content)) {
        return Err(StoreError::Integrity(
            "media snapshot SHA-256 does not match content".to_string(),
        ));
    }
    validate_media_server_retained_growth_with_limit(
        global_media_retained_bytes(transaction)?,
        None,
        content.len() as i64,
        global_limit_bytes,
    )?;
    ensure_snapshot_write_capacity(transaction, content.len() as u64)?;
    transaction.execute(
        "INSERT INTO media_snapshot_contents(
             sha256, size_bytes, content, reference_count, created_at_epoch_millis
         ) VALUES (?1, ?2, ?3, 0, ?4)",
        params![
            sha256,
            content.len() as i64,
            content,
            created_at_epoch_millis
        ],
    )?;
    Ok(())
}

fn is_snapshot_media_capacity_error(error: &StoreError) -> bool {
    matches!(
        error,
        StoreError::SnapshotHistoryQuotaExceeded { .. }
            | StoreError::MediaServerRetainedQuotaExceeded { .. }
            | StoreError::DiskReserveExceeded { .. }
    )
}

fn capture_incomplete_media_snapshot(
    transaction: &Transaction<'_>,
    snapshot: &AccountSnapshot,
) -> StoreResult<()> {
    let mut attachment_ids = referenced_attachment_ids_tolerant(&snapshot.app_data_json)?
        .into_iter()
        .collect::<Vec<_>>();
    attachment_ids.sort();
    let mut statement = transaction.prepare(
        "INSERT INTO account_snapshot_media_history(
             user_id, account_revision, attachment_id, content_sha256,
             declared_sha256, mime_type, size_bytes, updated_at_epoch_millis,
             missing_reason
         ) VALUES (?1, ?2, ?3, NULL, '', '', 0, 0,
                   'reference_metadata_unavailable_at_snapshot')",
    )?;
    for attachment_id in attachment_ids {
        if statement.execute(params![snapshot.user_id, snapshot.revision, attachment_id])? != 1 {
            return Err(StoreError::Integrity(
                "incomplete media snapshot changed an unexpected row count".to_string(),
            ));
        }
    }
    let changed = transaction.execute(
        "UPDATE account_snapshot_history
         SET media_snapshot_complete = 0
         WHERE user_id = ?1 AND revision = ?2",
        params![snapshot.user_id, snapshot.revision],
    )?;
    if changed != 1 {
        return Err(StoreError::Integrity(
            "incomplete media snapshot changed an unexpected parent row count".to_string(),
        ));
    }
    Ok(())
}

fn capture_media_snapshot(
    transaction: &Transaction<'_>,
    snapshot: &AccountSnapshot,
    created_at_epoch_millis: i64,
) -> StoreResult<()> {
    if current_schema_version(transaction)? >= 16 {
        return snapshot_media::reconcile_one(
            transaction,
            &snapshot.user_id,
            snapshot.revision,
            &snapshot.app_data_json,
            created_at_epoch_millis,
            None,
            false,
        );
    }
    let mut expected_ids = referenced_attachment_ids(&snapshot.app_data_json)?;
    let expectations = match referenced_media_expectations(&snapshot.app_data_json) {
        Ok(expectations) => expectations,
        Err(StoreError::Integrity(_)) => HashMap::new(),
        Err(error) => return Err(error),
    };
    // Load only this snapshot's referenced blobs, one at a time. A small
    // historical note must not allocate every live account attachment at once.
    let mut media_lookup = transaction.prepare(
        "SELECT m.sha256, m.mime_type, m.size_bytes, m.content
             FROM note_media m
             LEFT JOIN note_media_tombstones t
               ON t.user_id = m.user_id AND t.attachment_id = m.attachment_id
             WHERE m.user_id = ?1 AND m.attachment_id = ?2
               AND m.deleted_at_epoch_millis IS NULL
               AND (t.deleted_revision_epoch_millis IS NULL
                    OR m.updated_at_epoch_millis > t.deleted_revision_epoch_millis)
             ",
    )?;
    let referenced_ids = expected_ids.iter().cloned().collect::<Vec<_>>();
    for attachment_id in referenced_ids {
        let active_media = media_lookup
            .query_row(params![snapshot.user_id, attachment_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                ))
            })
            .optional()?;
        let Some((sha256, mime_type, size_bytes, content)) = active_media else {
            continue;
        };
        let expectation_matches = expectations.get(&attachment_id).is_some_and(|expected| {
            valid_sha256_hex(&expected.sha256)
                && expected.size_bytes_declared
                && expected.size_bytes >= 0
                && expected.size_bytes <= MAX_MEDIA_BYTES as i64
                && !expected.mime_type.is_empty()
                && expected.sha256.eq_ignore_ascii_case(&sha256)
                && expected.size_bytes == size_bytes
                && expected.mime_type.eq_ignore_ascii_case(mime_type.trim())
        });
        if !expectation_matches {
            continue;
        }
        ensure_media_snapshot_content(transaction, &sha256, &content, created_at_epoch_millis)?;
        transaction.execute(
            "INSERT INTO account_snapshot_media_history(
                 user_id, account_revision, attachment_id, content_sha256,
                 declared_sha256, mime_type, size_bytes, updated_at_epoch_millis,
                 missing_reason
             ) VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6, ?7, '')",
            params![
                snapshot.user_id,
                snapshot.revision,
                attachment_id,
                sha256,
                mime_type,
                size_bytes,
                expectations
                    .get(&attachment_id)
                    .map(|expected| expected.updated_at_epoch_millis)
                    .unwrap_or(0),
            ],
        )?;
        expected_ids.remove(&attachment_id);
    }
    let media_snapshot_complete = expected_ids.is_empty();
    for attachment_id in &expected_ids {
        let expectation = expectations.get(attachment_id);
        let has_expected_metadata = expectation.is_some_and(|expected| {
            valid_sha256_hex(&expected.sha256)
                && expected.size_bytes_declared
                && expected.size_bytes >= 0
                && expected.size_bytes <= MAX_MEDIA_BYTES as i64
                && !expected.mime_type.is_empty()
        });
        let missing_reason = if has_expected_metadata {
            "content_unavailable_at_snapshot"
        } else {
            "reference_metadata_unavailable_at_snapshot"
        };
        let declared_sha256 = expectation
            .filter(|expected| valid_sha256_hex(&expected.sha256))
            .map(|expected| expected.sha256.as_str())
            .unwrap_or_default();
        let mime_type = expectation
            .map(|expected| expected.mime_type.as_str())
            .unwrap_or_default();
        let size_bytes = expectation
            .filter(|expected| {
                expected.size_bytes_declared
                    && (0..=MAX_MEDIA_BYTES as i64).contains(&expected.size_bytes)
            })
            .map(|expected| expected.size_bytes)
            .unwrap_or(0);
        let updated_at_epoch_millis = expectation
            .map(|expected| expected.updated_at_epoch_millis)
            .unwrap_or(0);
        transaction.execute(
            "INSERT INTO account_snapshot_media_history(
                 user_id, account_revision, attachment_id, content_sha256,
                 declared_sha256, mime_type, size_bytes, updated_at_epoch_millis,
                 missing_reason
             ) VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, ?7, ?8)",
            params![
                snapshot.user_id,
                snapshot.revision,
                attachment_id,
                declared_sha256,
                mime_type,
                size_bytes,
                updated_at_epoch_millis,
                missing_reason,
            ],
        )?;
    }
    transaction.execute(
        "UPDATE account_snapshot_history
         SET media_snapshot_complete = ?1
         WHERE user_id = ?2 AND revision = ?3",
        params![
            i64::from(media_snapshot_complete),
            snapshot.user_id,
            snapshot.revision
        ],
    )?;
    Ok(())
}

fn repair_missing_media_history_entries(
    transaction: &Transaction<'_>,
    user_id: &str,
    attachment_id: &str,
    sha256: &str,
    mime_type: &str,
    size_bytes: i64,
    content: &[u8],
    created_at_epoch_millis: i64,
) -> StoreResult<()> {
    if current_schema_version(transaction)? >= 16 {
        snapshot_media::reconcile_user(
            transaction,
            user_id,
            None,
            created_at_epoch_millis,
            true,
            false,
        )?;
        return verify_snapshot_content_index(transaction);
    }
    let affected_revisions = {
        let mut statement = transaction.prepare(
            "SELECT account_revision
             FROM account_snapshot_media_history
             WHERE user_id = ?1 AND attachment_id = ?2
               AND content_sha256 IS NULL
               AND missing_reason = 'content_unavailable_at_snapshot'
               AND lower(declared_sha256) = lower(?3)
               AND size_bytes = ?4
               AND lower(mime_type) = lower(?5)
             ORDER BY account_revision",
        )?;
        let collected = statement
            .query_map(
                params![user_id, attachment_id, sha256, size_bytes, mime_type],
                |row| row.get::<_, i64>(0),
            )?
            .collect::<Result<Vec<_>, _>>()?;
        collected
    };
    if affected_revisions.is_empty() {
        return Ok(());
    }
    validate_media_history_repair_quota(transaction, user_id, sha256, content.len() as i64)?;
    ensure_media_snapshot_content(transaction, sha256, content, created_at_epoch_millis)?;
    for revision in affected_revisions {
        let changed = transaction.execute(
            "UPDATE account_snapshot_media_history
             SET content_sha256 = ?1, missing_reason = ''
             WHERE user_id = ?2 AND account_revision = ?3 AND attachment_id = ?4
               AND content_sha256 IS NULL
               AND missing_reason = 'content_unavailable_at_snapshot'",
            params![sha256, user_id, revision, attachment_id],
        )?;
        if changed != 1 {
            return Err(StoreError::Integrity(format!(
                "media history repair changed an unexpected row count for {user_id} revision {revision} attachment {attachment_id}"
            )));
        }
        transaction.execute(
            "UPDATE account_snapshot_history
             SET media_snapshot_complete = 1
             WHERE user_id = ?1 AND revision = ?2
               AND NOT EXISTS (
                   SELECT 1 FROM account_snapshot_media_history m
                   WHERE m.user_id = ?1 AND m.account_revision = ?2
                     AND (m.content_sha256 IS NULL OR length(m.missing_reason) > 0)
               )",
            params![user_id, revision],
        )?;
    }
    verify_snapshot_content_index(transaction)?;
    Ok(())
}

#[derive(Clone, Debug)]
struct LoadedMediaSnapshotEntry {
    attachment_id: String,
    sha256: String,
    mime_type: String,
    size_bytes: i64,
    updated_at_epoch_millis: i64,
    content: Vec<u8>,
}

// Retain byte-verified metadata only. A restore owns an IMMEDIATE transaction
// and re-reads each exact account/history/attachment source while writing it.
#[derive(Debug, Eq, PartialEq)]
struct VerifiedMediaSnapshotEntry {
    attachment_id: String,
    sha256: String,
    mime_type: String,
    size_bytes: i64,
    updated_at_epoch_millis: i64,
}

impl From<&LoadedMediaSnapshotEntry> for VerifiedMediaSnapshotEntry {
    fn from(entry: &LoadedMediaSnapshotEntry) -> Self {
        Self {
            attachment_id: entry.attachment_id.clone(),
            sha256: entry.sha256.clone(),
            mime_type: entry.mime_type.clone(),
            size_bytes: entry.size_bytes,
            updated_at_epoch_millis: entry.updated_at_epoch_millis,
        }
    }
}

struct VerifiedMediaSnapshotManifest {
    user_id: String,
    account_revision: i64,
    entries: Vec<VerifiedMediaSnapshotEntry>,
}

fn load_complete_media_snapshot(
    connection: &Connection,
    user_id: &str,
    account_revision: i64,
    app_data_json: &str,
) -> StoreResult<VerifiedMediaSnapshotManifest> {
    let complete = connection
        .query_row(
            "SELECT media_snapshot_complete
             FROM account_snapshot_history
             WHERE user_id = ?1 AND revision = ?2",
            params![user_id, account_revision],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .ok_or_else(|| {
            StoreError::NotFound(format!(
                "media snapshot revision {account_revision} for user {user_id}"
            ))
        })?;
    if complete != 1 {
        return Err(StoreError::Integrity(format!(
            "media snapshot is incomplete for user {user_id} revision {account_revision}"
        )));
    }
    if current_schema_version(connection)? >= 16 {
        return snapshot_media::load_complete(connection, user_id, account_revision, app_data_json);
    }
    let include_typed_conflicts = current_schema_version(connection)? >= 16;
    let expectations =
        referenced_media_expectations_at_format(app_data_json, include_typed_conflicts)?;
    let mut statement = connection.prepare(
        "SELECT h.attachment_id, h.content_sha256, h.declared_sha256,
                    h.mime_type, h.size_bytes, h.updated_at_epoch_millis,
                    h.missing_reason, c.size_bytes, c.content
             FROM account_snapshot_media_history h
             LEFT JOIN media_snapshot_contents c ON c.sha256 = h.content_sha256
             WHERE h.user_id = ?1 AND h.account_revision = ?2
             ORDER BY h.attachment_id",
    )?;
    let rows = statement.query_map(params![user_id, account_revision], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, i64>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, Option<i64>>(7)?,
            row.get::<_, Option<Vec<u8>>>(8)?,
        ))
    })?;
    let mut loaded = Vec::new();
    for row in rows {
        let (
            attachment_id,
            content_sha256,
            declared_sha256,
            mime_type,
            size_bytes,
            updated_at_epoch_millis,
            missing_reason,
            stored_size_bytes,
            content,
        ) = row?;
        let Some(content_sha256) = content_sha256 else {
            return Err(StoreError::Integrity(format!(
                "media snapshot lacks attachment {attachment_id} for user {user_id} revision {account_revision}: {missing_reason}"
            )));
        };
        let Some(content) = content else {
            return Err(StoreError::Integrity(format!(
                "media snapshot content is missing for attachment {attachment_id}"
            )));
        };
        if !missing_reason.is_empty()
            || mime_type.trim().is_empty()
            || size_bytes < 0
            || stored_size_bytes != Some(size_bytes)
            || content.len() as i64 != size_bytes
            || !content_sha256.eq_ignore_ascii_case(&declared_sha256)
            || !content_sha256.eq_ignore_ascii_case(&sha256_hex(&content))
        {
            return Err(StoreError::Integrity(format!(
                "media snapshot metadata/content mismatch for attachment {attachment_id}"
            )));
        }
        let Some(expected) = expectations.get(&attachment_id) else {
            return Err(StoreError::Integrity(format!(
                "media snapshot lacks JSON metadata for attachment {attachment_id}"
            )));
        };
        if !expected.sha256.eq_ignore_ascii_case(&content_sha256)
            || !expected.mime_type.eq_ignore_ascii_case(mime_type.trim())
            || !expected.size_bytes_declared
            || expected.size_bytes != size_bytes
            || expected.updated_at_epoch_millis != updated_at_epoch_millis
        {
            return Err(StoreError::Integrity(format!(
                "media snapshot does not match historical JSON metadata for attachment {attachment_id}"
            )));
        }
        loaded.push(VerifiedMediaSnapshotEntry {
            attachment_id,
            sha256: content_sha256,
            mime_type,
            size_bytes,
            updated_at_epoch_millis,
        });
    }
    let expected_ids =
        collected_referenced_attachment_ids_at_format(app_data_json, include_typed_conflicts)?;
    if expected_ids.iter().any(|id| !valid_media_attachment_id(id)) {
        return Err(StoreError::Integrity(
            "historical media attachment identity is invalid".into(),
        ));
    }
    let loaded_ids = loaded
        .iter()
        .map(|entry| entry.attachment_id.clone())
        .collect::<HashSet<_>>();
    if expected_ids != loaded_ids {
        return Err(StoreError::Integrity(format!(
            "media snapshot manifest does not match JSON references for user {user_id} revision {account_revision}"
        )));
    }
    Ok(VerifiedMediaSnapshotManifest {
        user_id: user_id.to_owned(),
        account_revision,
        entries: loaded,
    })
}

fn restore_media_snapshot(
    transaction: &Transaction<'_>,
    user_id: &str,
    manifest: &VerifiedMediaSnapshotManifest,
    now_epoch_millis: i64,
) -> StoreResult<()> {
    if manifest.user_id != user_id {
        return Err(StoreError::Integrity(
            "restored media manifest belongs to a different account".into(),
        ));
    }
    let entries = &manifest.entries;
    let target_bytes = entries.iter().try_fold(0_i64, |total, entry| {
        total
            .checked_add(entry.size_bytes)
            .ok_or_else(|| StoreError::Integrity("restored media size overflow".to_string()))
    })?;
    if entries.len() as i64 > MAX_MEDIA_ITEMS_PER_ACCOUNT || target_bytes > MAX_MEDIA_ACCOUNT_BYTES
    {
        return Err(StoreError::Integrity(
            "restored media exceeds the account quota".to_string(),
        ));
    }
    ensure_media_write_capacity(transaction, target_bytes as u64)?;
    let target_ids = entries
        .iter()
        .map(|entry| entry.attachment_id.as_str())
        .collect::<HashSet<_>>();
    let active_media = {
        let mut statement = transaction.prepare(
            "SELECT m.attachment_id, m.updated_at_epoch_millis
             FROM note_media m
             LEFT JOIN note_media_tombstones t
               ON t.user_id = m.user_id AND t.attachment_id = m.attachment_id
             WHERE m.user_id = ?1
               AND m.deleted_at_epoch_millis IS NULL
               AND (t.deleted_revision_epoch_millis IS NULL
                    OR m.updated_at_epoch_millis > t.deleted_revision_epoch_millis)",
        )?;
        let collected = statement
            .query_map(params![user_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        collected
    };
    for entry in entries {
        if !valid_media_attachment_id(&entry.attachment_id)
            || !valid_media_mime_type(&entry.mime_type)
        {
            return Err(StoreError::Integrity(
                "restored media metadata is invalid".to_string(),
            ));
        }
        validate_media_identity_quota(transaction, user_id, &entry.attachment_id)?;
        let existing_size = transaction
            .query_row(
                "SELECT size_bytes FROM note_media
                 WHERE user_id = ?1 AND attachment_id = ?2",
                params![user_id, entry.attachment_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let retained_bytes = transaction.query_row(
            "SELECT COALESCE(SUM(size_bytes), 0) FROM note_media WHERE user_id = ?1",
            params![user_id],
            |row| row.get::<_, i64>(0),
        )?;
        validate_media_retained_quota(retained_bytes, existing_size, entry.size_bytes)?;
        validate_media_server_retained_growth(transaction, existing_size, entry.size_bytes)?;
        // The metadata was proved before any restore writes. Re-read bytes
        // from that same owner, historical revision and attachment, never by
        // global content hash alone. Only this one BLOB remains resident.
        let source = snapshot_media::historical_entry(
            transaction,
            user_id,
            manifest.account_revision,
            &entry.attachment_id,
        )?
        .filter(|source| VerifiedMediaSnapshotEntry::from(source) == *entry)
        .ok_or_else(|| {
            StoreError::Integrity(format!(
                "verified historical media changed before restoring attachment {}",
                entry.attachment_id
            ))
        })?;
        transaction.execute(
            "INSERT INTO note_media(
                 user_id, attachment_id, sha256, mime_type, size_bytes, content,
                 updated_at_epoch_millis, deleted_at_epoch_millis
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)
             ON CONFLICT(user_id, attachment_id) DO UPDATE SET
                 sha256 = excluded.sha256,
                 mime_type = excluded.mime_type,
                 size_bytes = excluded.size_bytes,
                 content = excluded.content,
                 updated_at_epoch_millis = excluded.updated_at_epoch_millis,
                 deleted_at_epoch_millis = NULL",
            params![
                user_id,
                entry.attachment_id,
                entry.sha256,
                entry.mime_type,
                entry.size_bytes,
                source.content,
                entry.updated_at_epoch_millis,
            ],
        )?;
        transaction.execute(
            "DELETE FROM note_media_tombstones
             WHERE user_id = ?1 AND attachment_id = ?2",
            params![user_id, entry.attachment_id],
        )?;
    }
    for (attachment_id, updated_at_epoch_millis) in active_media {
        if target_ids.contains(attachment_id.as_str()) {
            continue;
        }
        let deleted_revision = now_epoch_millis
            .max(updated_at_epoch_millis.saturating_add(1))
            .max(1);
        if !valid_media_attachment_id(&attachment_id) {
            return Err(StoreError::Integrity(
                "stored media attachment id is invalid".to_string(),
            ));
        }
        validate_media_identity_quota(transaction, user_id, &attachment_id)?;
        transaction.execute(
            "INSERT INTO note_media_tombstones(
                 user_id, attachment_id, deleted_revision_epoch_millis,
                 recorded_at_epoch_millis
             ) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(user_id, attachment_id) DO UPDATE SET
                 deleted_revision_epoch_millis = excluded.deleted_revision_epoch_millis,
                 recorded_at_epoch_millis = excluded.recorded_at_epoch_millis
             WHERE excluded.deleted_revision_epoch_millis
                   > note_media_tombstones.deleted_revision_epoch_millis",
            params![
                user_id,
                attachment_id,
                deleted_revision,
                now_epoch_millis.max(0)
            ],
        )?;
        transaction.execute(
            "UPDATE note_media
             SET deleted_at_epoch_millis = ?1
             WHERE user_id = ?2 AND attachment_id = ?3",
            params![deleted_revision, user_id, attachment_id],
        )?;
    }
    Ok(())
}

fn mark_legacy_media_free_histories_complete(transaction: &Transaction<'_>) -> StoreResult<()> {
    let histories = {
        let mut statement = transaction.prepare(
            "SELECT h.user_id, h.revision, h.created_at_epoch_millis,
                    c.sha256, c.compression, c.uncompressed_size_bytes,
                    c.compressed_size_bytes, c.content
             FROM account_snapshot_history h
             JOIN snapshot_contents c ON c.sha256 = h.content_sha256
             ORDER BY h.user_id, h.revision",
        )?;
        let collected = statement
            .query_map([], snapshot_content_row)?
            .collect::<Result<Vec<_>, _>>()?;
        collected
    };
    for history in histories {
        let raw = decode_snapshot_content(&history)?;
        if referenced_attachment_ids(&raw)?.is_empty() {
            transaction.execute(
                "UPDATE account_snapshot_history
                 SET media_snapshot_complete = 1
                 WHERE user_id = ?1 AND revision = ?2",
                params![history.user_id, history.revision],
            )?;
        }
    }
    Ok(())
}

fn migrate_snapshot_history_to_content_store(transaction: &Transaction<'_>) -> StoreResult<()> {
    let rows = {
        let mut statement = transaction.prepare(
            "SELECT user_id, revision, app_data_json, created_at_epoch_millis, sha256
             FROM account_snapshot_history
             ORDER BY user_id, revision",
        )?;
        let collected = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        collected
    };
    for (user_id, revision, app_data_json, created_at, stored_sha256) in rows {
        validate_app_data_json(&app_data_json)?;
        let computed_sha256 = sha256_hex(app_data_json.as_bytes());
        if !valid_sha256_hex(&stored_sha256)
            || !stored_sha256.eq_ignore_ascii_case(&computed_sha256)
        {
            return Err(StoreError::Integrity(format!(
                "snapshot history SHA-256 mismatch while migrating user {user_id} revision {revision}"
            )));
        }
        ensure_snapshot_content(transaction, &computed_sha256, &app_data_json, created_at)?;
        transaction.execute(
            "INSERT INTO account_snapshot_history_v4(
                 user_id, revision, content_sha256, created_at_epoch_millis
             ) VALUES (?1, ?2, ?3, ?4)",
            params![user_id, revision, computed_sha256, created_at],
        )?;
    }
    transaction.execute(
        "UPDATE snapshot_contents
         SET reference_count = (
             SELECT COUNT(*) FROM account_snapshot_history_v4 h
             WHERE h.content_sha256 = snapshot_contents.sha256
         )",
        [],
    )?;
    transaction.execute(
        "DELETE FROM snapshot_contents WHERE reference_count = 0",
        [],
    )?;
    Ok(())
}

fn verify_snapshot_content_index(connection: &Connection) -> StoreResult<()> {
    let invalid_reference_counts = connection.query_row(
        "SELECT COUNT(*)
         FROM snapshot_contents c
         WHERE c.reference_count <> (
             SELECT COUNT(*) FROM account_snapshot_history h
             WHERE h.content_sha256 = c.sha256
         )
            OR c.reference_count = 0",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    if invalid_reference_counts != 0 {
        return Err(StoreError::Integrity(format!(
            "snapshot content index has {invalid_reference_counts} invalid reference counts"
        )));
    }
    let missing_contents = connection.query_row(
        "SELECT COUNT(*)
         FROM account_snapshot_history h
         LEFT JOIN snapshot_contents c ON c.sha256 = h.content_sha256
         WHERE c.sha256 IS NULL",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    if missing_contents != 0 {
        return Err(StoreError::Integrity(format!(
            "snapshot history has {missing_contents} missing content objects"
        )));
    }
    // Migration 4 deliberately verifies the JSON content-addressed index before
    // migration 6 has created the media-history tables.  Keep that intermediate
    // migration state valid while still refusing a partially-created media
    // schema once either media table exists.
    let media_contents_exists = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'media_snapshot_contents'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    let media_history_exists = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'account_snapshot_media_history'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !media_contents_exists && !media_history_exists {
        return Ok(());
    }
    if !media_contents_exists || !media_history_exists {
        return Err(StoreError::Integrity(
            "media snapshot schema is only partially present".to_string(),
        ));
    }
    let invalid_media_reference_counts = connection.query_row(
        "SELECT COUNT(*)
         FROM media_snapshot_contents c
         WHERE c.reference_count <> (
             SELECT COUNT(*) FROM account_snapshot_media_history h
             WHERE h.content_sha256 = c.sha256
         )
            OR c.reference_count = 0",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    if invalid_media_reference_counts != 0 {
        return Err(StoreError::Integrity(format!(
            "media snapshot content index has {invalid_media_reference_counts} invalid reference counts"
        )));
    }
    let missing_media_contents = connection.query_row(
        "SELECT COUNT(*)
         FROM account_snapshot_media_history h
         LEFT JOIN media_snapshot_contents c ON c.sha256 = h.content_sha256
         WHERE h.content_sha256 IS NOT NULL AND c.sha256 IS NULL",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    if missing_media_contents != 0 {
        return Err(StoreError::Integrity(format!(
            "media snapshot history has {missing_media_contents} missing content objects"
        )));
    }
    let falsely_complete_media_histories = connection.query_row(
        "SELECT COUNT(*)
         FROM account_snapshot_history h
         WHERE h.media_snapshot_complete = 1
           AND EXISTS (
               SELECT 1 FROM account_snapshot_media_history m
               WHERE m.user_id = h.user_id AND m.account_revision = h.revision
                 AND (m.content_sha256 IS NULL OR length(m.missing_reason) > 0)
           )",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    if falsely_complete_media_histories != 0 {
        return Err(StoreError::Integrity(format!(
            "snapshot history has {falsely_complete_media_histories} falsely complete media manifests"
        )));
    }
    let complete_history_keys = {
        let mut statement = connection.prepare(
            "SELECT user_id, revision
             FROM account_snapshot_history
             WHERE media_snapshot_complete = 1
             ORDER BY user_id, revision",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for (user_id, revision) in complete_history_keys {
        let stored = connection.query_row(
            "SELECT h.user_id, h.revision, h.created_at_epoch_millis,
                    c.sha256, c.compression, c.uncompressed_size_bytes,
                    c.compressed_size_bytes, c.content
             FROM account_snapshot_history h
             JOIN snapshot_contents c ON c.sha256 = h.content_sha256
             WHERE h.user_id = ?1 AND h.revision = ?2",
            params![user_id, revision],
            snapshot_content_row,
        )?;
        let history = decode_snapshot_history(stored)?;
        load_complete_media_snapshot(
            connection,
            &history.user_id,
            history.revision,
            &history.app_data_json,
        )?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct MediaStorageAudit {
    over_limit_account_identities: i64,
    over_limit_account_retained_bytes: i64,
    over_limit_account_active_items: i64,
    global_identities_over_limit: bool,
    global_retained_bytes_over_limit: bool,
}

impl MediaStorageAudit {
    fn has_grandfathered_overage(self) -> bool {
        self.over_limit_account_identities > 0
            || self.over_limit_account_retained_bytes > 0
            || self.over_limit_account_active_items > 0
            || self.global_identities_over_limit
            || self.global_retained_bytes_over_limit
    }
}

fn install_current_snapshot_media_identity_table(connection: &Connection) -> StoreResult<()> {
    connection.execute_batch(&format!(
        r#"
        DROP TABLE IF EXISTS account_snapshot_media_identities;
        {}
        "#,
        current_snapshot_media_identity_table_sql()
    ))?;
    Ok(())
}

fn current_snapshot_media_identity_table_sql() -> String {
    format!(
        r#"CREATE TABLE account_snapshot_media_identities (
            user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            attachment_id TEXT NOT NULL,
            PRIMARY KEY(user_id, attachment_id),
            CHECK(length(CAST(attachment_id AS BLOB)) BETWEEN 1 AND {MAX_MEDIA_ATTACHMENT_ID_BYTES}),
            CHECK(substr(attachment_id, 1, 1) GLOB '[0-9A-Za-z]'),
            CHECK(attachment_id NOT GLOB '*[^0-9A-Za-z_-]*'),
            CHECK(instr(attachment_id, char(0)) = 0)
        ) STRICT;"#
    )
}

fn install_legacy_snapshot_repair_allowance_table(connection: &Connection) -> StoreResult<()> {
    connection.execute_batch(&format!(
        "DROP TABLE IF EXISTS legacy_snapshot_repair_allowances;\n{}",
        legacy_snapshot_repair_allowance_table_sql()
    ))?;
    Ok(())
}

fn legacy_snapshot_repair_allowance_v13_table_sql() -> &'static str {
    "CREATE TABLE legacy_snapshot_repair_allowances (
         user_id TEXT PRIMARY KEY NOT NULL REFERENCES users(id) ON DELETE CASCADE,
         source_revision INTEGER NOT NULL CHECK(source_revision >= 0),
         source_content_sha256 TEXT NOT NULL
             CHECK(length(CAST(source_content_sha256 AS BLOB)) = 64)
             CHECK(source_content_sha256 NOT GLOB '*[^0-9a-f]*')
             CHECK(source_content_sha256 = lower(source_content_sha256)),
         seeded_at_epoch_millis INTEGER NOT NULL CHECK(seeded_at_epoch_millis >= 0)
     ) STRICT;"
}

fn legacy_snapshot_repair_allowance_table_sql() -> &'static str {
    "CREATE TABLE legacy_snapshot_repair_allowances (
         user_id TEXT PRIMARY KEY NOT NULL REFERENCES users(id) ON DELETE CASCADE,
         source_revision INTEGER NOT NULL CHECK(source_revision >= 0),
         source_content_sha256 TEXT NOT NULL
             CHECK(length(CAST(source_content_sha256 AS BLOB)) = 64)
             CHECK(source_content_sha256 NOT GLOB '*[^0-9a-f]*')
             CHECK(source_content_sha256 = lower(source_content_sha256)),
         seeded_at_epoch_millis INTEGER NOT NULL CHECK(seeded_at_epoch_millis >= 0),
         backup_file_name TEXT NOT NULL
             CHECK(length(CAST(backup_file_name AS BLOB)) BETWEEN 1 AND 1024)
             CHECK(backup_file_name NOT IN ('.', '..'))
             CHECK(instr(backup_file_name, '/') = 0)
             CHECK(instr(backup_file_name, char(92)) = 0),
         backup_size_bytes INTEGER NOT NULL CHECK(backup_size_bytes > 0),
         backup_sha256 TEXT NOT NULL
             CHECK(length(CAST(backup_sha256 AS BLOB)) = 64)
             CHECK(backup_sha256 NOT GLOB '*[^0-9a-f]*')
             CHECK(backup_sha256 = lower(backup_sha256)),
         backup_schema_version INTEGER NOT NULL CHECK(backup_schema_version IN (12, 13)),
         backup_created_at_epoch_millis INTEGER NOT NULL
             CHECK(backup_created_at_epoch_millis >= 0)
     ) STRICT;"
}

fn read_legacy_snapshot_repair_allowance_seeds_for_v14_migration(
    connection: &Connection,
) -> StoreResult<Vec<LegacySnapshotRepairAllowanceSeed>> {
    let stored_sql = connection
        .query_row(
            "SELECT sql FROM sqlite_master
             WHERE type = 'table' AND name = 'legacy_snapshot_repair_allowances'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| {
            StoreError::Integrity(
                "schema v13 legacy snapshot repair allowance table is missing".to_string(),
            )
        })?;
    let normalized = normalized_schema_sql(&stored_sql);
    if normalized != normalized_schema_sql(legacy_snapshot_repair_allowance_v13_table_sql())
        && normalized != normalized_schema_sql(legacy_snapshot_repair_allowance_table_sql())
    {
        return Err(StoreError::Integrity(
            "schema v13 legacy snapshot repair allowance table is divergent".to_string(),
        ));
    }

    let mut statement = connection.prepare(
        "SELECT user_id, source_revision, source_content_sha256, seeded_at_epoch_millis
         FROM legacy_snapshot_repair_allowances
         ORDER BY user_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(LegacySnapshotRepairAllowanceSeed {
            source: LegacySnapshotRepairSource {
                user_id: row.get(0)?,
                revision: row.get(1)?,
                content_sha256: row.get(2)?,
            },
            seeded_at_epoch_millis: row.get(3)?,
        })
    })?;
    let seeds = rows.collect::<Result<Vec<_>, _>>()?;
    Ok(seeds)
}

fn legacy_snapshot_repair_manifest(
    connection: &Connection,
) -> StoreResult<Vec<LegacySnapshotRepairSource>> {
    let mut statement = connection.prepare(
        "SELECT user_id, revision, content_sha256, app_data_json
         FROM account_snapshots
         ORDER BY user_id",
    )?;
    let mut rows = statement.query([])?;
    let mut manifest = Vec::new();
    while let Some(row) = rows.next()? {
        let user_id = row.get::<_, String>(0)?;
        let revision = row.get::<_, i64>(1)?;
        let content_sha256 = row.get::<_, String>(2)?;
        let app_data_json = row.get::<_, String>(3)?;
        if revision < 0 {
            return Err(StoreError::Integrity(format!(
                "negative account revision in snapshot repair manifest for user {user_id}"
            )));
        }
        validate_app_data_json(&app_data_json)?;
        let computed_sha256 = sha256_hex(app_data_json.as_bytes());
        if content_sha256 != computed_sha256 {
            return Err(StoreError::Integrity(format!(
                "account snapshot content hash is invalid in snapshot repair manifest for user {user_id}"
            )));
        }
        manifest.push(LegacySnapshotRepairSource {
            user_id,
            revision,
            content_sha256,
        });
    }
    Ok(manifest)
}

fn verify_legacy_snapshot_repair_manifest_matches_live(
    connection: &Connection,
    verified_backup_manifest: &[LegacySnapshotRepairSource],
) -> StoreResult<()> {
    let live_manifest = legacy_snapshot_repair_manifest(connection)?;
    if live_manifest != verified_backup_manifest {
        return Err(StoreError::Integrity(
            "live account snapshots diverged from the verified pre-schema backup; refusing snapshot repair allowance seeding"
                .to_string(),
        ));
    }
    Ok(())
}

fn seed_legacy_snapshot_repair_allowances(
    connection: &Connection,
    seeds: &[LegacySnapshotRepairAllowanceSeed],
    backup: &LegacySnapshotRepairBackupIdentity,
) -> StoreResult<()> {
    let mut statement = connection.prepare(
        "INSERT INTO legacy_snapshot_repair_allowances(
             user_id, source_revision, source_content_sha256, seeded_at_epoch_millis,
             backup_file_name, backup_size_bytes, backup_sha256,
             backup_schema_version, backup_created_at_epoch_millis
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )?;
    for seed in seeds {
        let changed = statement.execute(params![
            &seed.source.user_id,
            seed.source.revision,
            &seed.source.content_sha256,
            seed.seeded_at_epoch_millis,
            &backup.file_name,
            backup.size_bytes,
            &backup.sha256,
            backup.schema_version,
            backup.created_at_epoch_millis,
        ])?;
        if changed != 1 {
            return Err(StoreError::Integrity(
                "snapshot repair allowance seeding changed an unexpected row count".to_string(),
            ));
        }
    }
    Ok(())
}

fn replace_current_snapshot_media_identities(
    connection: &Connection,
    user_id: &str,
    app_data_json: &str,
) -> StoreResult<()> {
    let expected = collected_referenced_attachment_ids_at_format(
        app_data_json,
        current_schema_version(connection)? >= 16,
    )?;
    if expected.iter().any(|id| !valid_media_attachment_id(id)) {
        return Err(StoreError::Integrity(
            "current media attachment identity is invalid".into(),
        ));
    }
    replace_current_snapshot_media_identity_set(connection, user_id, &expected)
}

fn replace_current_snapshot_media_identities_tolerant(
    connection: &Connection,
    user_id: &str,
    app_data_json: &str,
) -> StoreResult<()> {
    let mut expected = collected_referenced_attachment_ids_at_format(
        app_data_json,
        current_schema_version(connection)? >= 16,
    )?;
    expected.retain(|id| valid_media_attachment_id(id));
    replace_current_snapshot_media_identity_set(connection, user_id, &expected)
}

fn replace_current_snapshot_media_identity_set(
    connection: &Connection,
    user_id: &str,
    expected: &HashSet<String>,
) -> StoreResult<()> {
    let existing = {
        let mut statement = connection.prepare(
            "SELECT attachment_id FROM account_snapshot_media_identities WHERE user_id = ?1",
        )?;
        let rows = statement.query_map(params![user_id], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<HashSet<_>, _>>()?
    };
    let mut removals = existing.difference(expected).cloned().collect::<Vec<_>>();
    let mut additions = expected.difference(&existing).cloned().collect::<Vec<_>>();
    removals.sort();
    additions.sort();
    if !additions.is_empty() {
        let payload_bytes = u64::try_from(additions.len())
            .map_err(|_| {
                StoreError::Integrity("current media identity index size overflow".to_string())
            })?
            .checked_mul(SCHEMA_MIGRATION_INDEX_ROW_ESTIMATE_BYTES)
            .ok_or_else(|| {
                StoreError::Integrity("current media identity index size overflow".to_string())
            })?;
        ensure_snapshot_write_capacity(connection, payload_bytes)?;
    }
    {
        let mut statement = connection.prepare(
            "DELETE FROM account_snapshot_media_identities
             WHERE user_id = ?1 AND attachment_id = ?2",
        )?;
        for attachment_id in removals {
            if statement.execute(params![user_id, attachment_id])? != 1 {
                return Err(StoreError::Integrity(
                    "current media identity removal changed an unexpected row count".to_string(),
                ));
            }
        }
    }
    let mut statement = connection.prepare(
        "INSERT INTO account_snapshot_media_identities(user_id, attachment_id)
         VALUES (?1, ?2)",
    )?;
    for attachment_id in additions {
        if statement.execute(params![user_id, attachment_id])? != 1 {
            return Err(StoreError::Integrity(
                "current media identity insertion changed an unexpected row count".to_string(),
            ));
        }
    }
    Ok(())
}

fn rebuild_current_snapshot_media_identities(connection: &Connection) -> StoreResult<()> {
    connection.execute("DELETE FROM account_snapshot_media_identities", [])?;
    let user_ids = {
        let mut statement =
            connection.prepare("SELECT user_id FROM account_snapshots ORDER BY user_id")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for user_id in user_ids {
        let app_data_json = connection.query_row(
            "SELECT app_data_json FROM account_snapshots WHERE user_id = ?1",
            params![user_id],
            |row| row.get::<_, String>(0),
        )?;
        replace_current_snapshot_media_identities_tolerant(connection, &user_id, &app_data_json)?;
    }
    Ok(())
}

fn verify_current_snapshot_media_identity_index(connection: &Connection) -> StoreResult<()> {
    let include_typed_conflicts = current_schema_version(connection)? >= 16;
    let mut indexed = HashMap::<String, (Option<HashSet<String>>, HashSet<String>)>::new();
    let mut statement = connection.prepare(
        "SELECT user_id, row_kind, app_data_json, attachment_id
         FROM (
             SELECT user_id, 0 AS row_kind, app_data_json,
                    NULL AS attachment_id
             FROM account_snapshots
             UNION ALL
             SELECT user_id, 1 AS row_kind, NULL AS app_data_json,
                    attachment_id
             FROM account_snapshot_media_identities
         )
         ORDER BY user_id, row_kind, attachment_id",
    )?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let user_id = row.get::<_, String>(0)?;
        let row_kind = row.get::<_, i64>(1)?;
        let entry = indexed
            .entry(user_id.clone())
            .or_insert_with(|| (None, HashSet::new()));
        match row_kind {
            0 => {
                let app_data_json = row.get::<_, String>(2)?;
                if entry.0.is_some() {
                    return Err(StoreError::Integrity(format!(
                        "current snapshot media identity index saw duplicate snapshot rows for user {user_id}"
                    )));
                }
                let mut expected = collected_referenced_attachment_ids_at_format(
                    &app_data_json,
                    include_typed_conflicts,
                )?;
                expected.retain(|id| valid_media_attachment_id(id));
                entry.0 = Some(expected);
            }
            1 => {
                let attachment_id = row.get::<_, String>(3)?;
                if !entry.1.insert(attachment_id) {
                    return Err(StoreError::Integrity(format!(
                        "current snapshot media identity index contains duplicate identities for user {user_id}"
                    )));
                }
            }
            _ => {
                return Err(StoreError::Integrity(
                    "current snapshot media identity index query returned an invalid row kind"
                        .to_string(),
                ));
            }
        }
    }
    for (user_id, (expected, stored)) in indexed {
        let Some(expected) = expected else {
            return Err(StoreError::Integrity(format!(
                "current snapshot media identity index contains rows without a snapshot for user {user_id}"
            )));
        };
        if stored != expected {
            return Err(StoreError::Integrity(format!(
                "current snapshot media identity index diverged for user {user_id}"
            )));
        }
    }
    Ok(())
}

fn normalize_legacy_media_history_declared_sha256(connection: &Connection) -> StoreResult<()> {
    connection.execute(
        "UPDATE account_snapshot_media_history
         SET declared_sha256 = ''
         WHERE content_sha256 IS NULL
           AND length(declared_sha256) > 0
           AND (length(CAST(declared_sha256 AS BLOB)) <> 64
                OR declared_sha256 GLOB '*[^0-9a-f]*'
                OR declared_sha256 <> lower(declared_sha256))",
        [],
    )?;
    Ok(())
}

fn media_metadata_validation_schema_sql() -> String {
    format!(
        r#"
        DROP INDEX IF EXISTS account_snapshot_media_history_identity_index;
        DROP TRIGGER IF EXISTS note_media_metadata_insert_guard;
        DROP TRIGGER IF EXISTS note_media_metadata_update_guard;
        DROP TRIGGER IF EXISTS note_media_tombstone_insert_guard;
        DROP TRIGGER IF EXISTS note_media_tombstone_update_guard;
        DROP TRIGGER IF EXISTS media_history_metadata_insert_guard;
        DROP TRIGGER IF EXISTS media_history_metadata_update_guard;

        CREATE TRIGGER note_media_metadata_insert_guard
        BEFORE INSERT ON note_media
        WHEN length(CAST(NEW.attachment_id AS BLOB)) < 1
          OR length(CAST(NEW.attachment_id AS BLOB)) > {MAX_MEDIA_ATTACHMENT_ID_BYTES}
          OR substr(NEW.attachment_id, 1, 1) NOT GLOB '[0-9A-Za-z]'
          OR NEW.attachment_id GLOB '*[^0-9A-Za-z_-]*'
          OR instr(NEW.attachment_id, char(0)) > 0
          OR length(CAST(NEW.mime_type AS BLOB)) < 1
          OR length(CAST(NEW.mime_type AS BLOB)) > {MAX_MEDIA_MIME_TYPE_BYTES}
          OR NEW.mime_type GLOB '*[^!-~]*'
          OR instr(NEW.mime_type, char(0)) > 0
        BEGIN
            SELECT RAISE(ABORT, 'invalid note_media metadata');
        END;

        CREATE TRIGGER note_media_metadata_update_guard
        BEFORE UPDATE OF attachment_id, mime_type ON note_media
        WHEN length(CAST(NEW.attachment_id AS BLOB)) < 1
          OR length(CAST(NEW.attachment_id AS BLOB)) > {MAX_MEDIA_ATTACHMENT_ID_BYTES}
          OR substr(NEW.attachment_id, 1, 1) NOT GLOB '[0-9A-Za-z]'
          OR NEW.attachment_id GLOB '*[^0-9A-Za-z_-]*'
          OR instr(NEW.attachment_id, char(0)) > 0
          OR length(CAST(NEW.mime_type AS BLOB)) < 1
          OR length(CAST(NEW.mime_type AS BLOB)) > {MAX_MEDIA_MIME_TYPE_BYTES}
          OR NEW.mime_type GLOB '*[^!-~]*'
          OR instr(NEW.mime_type, char(0)) > 0
        BEGIN
            SELECT RAISE(ABORT, 'invalid note_media metadata');
        END;

        CREATE TRIGGER note_media_tombstone_insert_guard
        BEFORE INSERT ON note_media_tombstones
        WHEN length(CAST(NEW.attachment_id AS BLOB)) < 1
          OR length(CAST(NEW.attachment_id AS BLOB)) > {MAX_MEDIA_ATTACHMENT_ID_BYTES}
          OR substr(NEW.attachment_id, 1, 1) NOT GLOB '[0-9A-Za-z]'
          OR NEW.attachment_id GLOB '*[^0-9A-Za-z_-]*'
          OR instr(NEW.attachment_id, char(0)) > 0
        BEGIN
            SELECT RAISE(ABORT, 'invalid note_media tombstone identity');
        END;

        CREATE TRIGGER note_media_tombstone_update_guard
        BEFORE UPDATE OF attachment_id ON note_media_tombstones
        WHEN length(CAST(NEW.attachment_id AS BLOB)) < 1
          OR length(CAST(NEW.attachment_id AS BLOB)) > {MAX_MEDIA_ATTACHMENT_ID_BYTES}
          OR substr(NEW.attachment_id, 1, 1) NOT GLOB '[0-9A-Za-z]'
          OR NEW.attachment_id GLOB '*[^0-9A-Za-z_-]*'
          OR instr(NEW.attachment_id, char(0)) > 0
        BEGIN
            SELECT RAISE(ABORT, 'invalid note_media tombstone identity');
        END;

        CREATE TRIGGER media_history_metadata_insert_guard
        BEFORE INSERT ON account_snapshot_media_history
        WHEN length(CAST(NEW.attachment_id AS BLOB)) < 1
          OR length(CAST(NEW.attachment_id AS BLOB)) > {MAX_MEDIA_ATTACHMENT_ID_BYTES}
          OR substr(NEW.attachment_id, 1, 1) NOT GLOB '[0-9A-Za-z]'
          OR NEW.attachment_id GLOB '*[^0-9A-Za-z_-]*'
          OR instr(NEW.attachment_id, char(0)) > 0
          OR (length(CAST(NEW.declared_sha256 AS BLOB)) > 0
              AND (length(CAST(NEW.declared_sha256 AS BLOB)) <> 64
                   OR NEW.declared_sha256 GLOB '*[^0-9a-f]*'
                   OR NEW.declared_sha256 <> lower(NEW.declared_sha256)))
          OR (NEW.content_sha256 IS NOT NULL
              AND NEW.declared_sha256 <> lower(NEW.content_sha256))
          OR length(CAST(NEW.mime_type AS BLOB)) > {MAX_MEDIA_MIME_TYPE_BYTES}
          OR (length(CAST(NEW.mime_type AS BLOB)) = 0
              AND NEW.missing_reason <> 'reference_metadata_unavailable_at_snapshot')
          OR (length(CAST(NEW.mime_type AS BLOB)) > 0
              AND (NEW.mime_type GLOB '*[^!-~]*'
                   OR instr(NEW.mime_type, char(0)) > 0))
        BEGIN
            SELECT RAISE(ABORT, 'invalid media history metadata');
        END;

        CREATE TRIGGER media_history_metadata_update_guard
        BEFORE UPDATE OF attachment_id, content_sha256, declared_sha256, mime_type, missing_reason
        ON account_snapshot_media_history
        WHEN length(CAST(NEW.attachment_id AS BLOB)) < 1
          OR length(CAST(NEW.attachment_id AS BLOB)) > {MAX_MEDIA_ATTACHMENT_ID_BYTES}
          OR substr(NEW.attachment_id, 1, 1) NOT GLOB '[0-9A-Za-z]'
          OR NEW.attachment_id GLOB '*[^0-9A-Za-z_-]*'
          OR instr(NEW.attachment_id, char(0)) > 0
          OR (length(CAST(NEW.declared_sha256 AS BLOB)) > 0
              AND (length(CAST(NEW.declared_sha256 AS BLOB)) <> 64
                   OR NEW.declared_sha256 GLOB '*[^0-9a-f]*'
                   OR NEW.declared_sha256 <> lower(NEW.declared_sha256)))
          OR (NEW.content_sha256 IS NOT NULL
              AND NEW.declared_sha256 <> lower(NEW.content_sha256))
          OR length(CAST(NEW.mime_type AS BLOB)) > {MAX_MEDIA_MIME_TYPE_BYTES}
          OR (length(CAST(NEW.mime_type AS BLOB)) = 0
              AND NEW.missing_reason <> 'reference_metadata_unavailable_at_snapshot')
          OR (length(CAST(NEW.mime_type AS BLOB)) > 0
              AND (NEW.mime_type GLOB '*[^!-~]*'
                   OR instr(NEW.mime_type, char(0)) > 0))
        BEGIN
            SELECT RAISE(ABORT, 'invalid media history metadata');
        END;

        CREATE INDEX account_snapshot_media_history_identity_index
            ON account_snapshot_media_history(user_id, attachment_id);
        "#
    )
}

fn drop_media_metadata_validation_triggers(connection: &Connection) -> StoreResult<()> {
    connection.execute_batch(
        "DROP TRIGGER IF EXISTS note_media_metadata_insert_guard;
         DROP TRIGGER IF EXISTS note_media_metadata_update_guard;
         DROP TRIGGER IF EXISTS note_media_tombstone_insert_guard;
         DROP TRIGGER IF EXISTS note_media_tombstone_update_guard;
         DROP TRIGGER IF EXISTS media_history_metadata_insert_guard;
         DROP TRIGGER IF EXISTS media_history_metadata_update_guard;",
    )?;
    Ok(())
}

fn install_media_metadata_validation_triggers(connection: &Connection) -> StoreResult<()> {
    let sql = media_metadata_validation_schema_sql();
    connection.execute_batch(&sql)?;
    Ok(())
}

fn canonical_media_metadata_trigger_sql(trigger_name: &str) -> StoreResult<String> {
    let schema_sql = media_metadata_validation_schema_sql();
    let prefix = format!("CREATE TRIGGER {trigger_name}");
    let start = schema_sql.find(&prefix).ok_or_else(|| {
        StoreError::Integrity(format!(
            "canonical media metadata trigger is undefined: {trigger_name}"
        ))
    })?;
    let definition = &schema_sql[start..];
    let end = definition.find("END;").ok_or_else(|| {
        StoreError::Integrity(format!(
            "canonical media metadata trigger is incomplete: {trigger_name}"
        ))
    })? + "END;".len();
    Ok(definition[..end].to_string())
}

fn normalized_schema_sql(sql: &str) -> String {
    sql.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches(';')
        .to_string()
}

/// Audits media metadata before schema v13 is committed and on every verified
/// open. Structural violations fail closed without rewriting evidence. Existing
/// quota overages are reported separately so reads and cleanup remain available
/// while all subsequent positive growth is rejected transactionally.
fn verify_media_storage_invariants(connection: &Connection) -> StoreResult<MediaStorageAudit> {
    let oversized_live_rows = connection.query_row(
        "SELECT COUNT(*) FROM note_media
         WHERE length(CAST(attachment_id AS BLOB)) < 1
            OR length(CAST(attachment_id AS BLOB)) > ?1
            OR length(CAST(mime_type AS BLOB)) < 1
            OR length(CAST(mime_type AS BLOB)) > ?2",
        params![
            MAX_MEDIA_ATTACHMENT_ID_BYTES as i64,
            MAX_MEDIA_MIME_TYPE_BYTES as i64
        ],
        |row| row.get::<_, i64>(0),
    )?;
    let oversized_tombstone_rows = connection.query_row(
        "SELECT COUNT(*) FROM note_media_tombstones
         WHERE length(CAST(attachment_id AS BLOB)) < 1
            OR length(CAST(attachment_id AS BLOB)) > ?1",
        params![MAX_MEDIA_ATTACHMENT_ID_BYTES as i64],
        |row| row.get::<_, i64>(0),
    )?;
    let oversized_history_rows = connection.query_row(
        "SELECT COUNT(*) FROM account_snapshot_media_history
         WHERE length(CAST(attachment_id AS BLOB)) < 1
            OR length(CAST(attachment_id AS BLOB)) > ?1
            OR length(CAST(mime_type AS BLOB)) > ?2
            OR (length(CAST(declared_sha256 AS BLOB)) > 0
                AND (length(CAST(declared_sha256 AS BLOB)) <> 64
                     OR declared_sha256 GLOB '*[^0-9a-f]*'
                     OR declared_sha256 <> lower(declared_sha256)))
            OR (content_sha256 IS NOT NULL
                AND declared_sha256 <> lower(content_sha256))",
        params![
            MAX_MEDIA_ATTACHMENT_ID_BYTES as i64,
            MAX_MEDIA_MIME_TYPE_BYTES as i64
        ],
        |row| row.get::<_, i64>(0),
    )?;
    if oversized_live_rows > 0 || oversized_tombstone_rows > 0 || oversized_history_rows > 0 {
        return Err(StoreError::Integrity(
            "stored media metadata exceeds its structural byte limit".to_string(),
        ));
    }

    {
        let mut statement = connection.prepare(
            "SELECT attachment_id, mime_type FROM note_media ORDER BY user_id, attachment_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (attachment_id, mime_type) = row?;
            if !valid_media_attachment_id(&attachment_id) || !valid_media_mime_type(&mime_type) {
                return Err(StoreError::Integrity(
                    "stored note_media metadata is structurally invalid".to_string(),
                ));
            }
        }
    }
    {
        let mut statement = connection.prepare(
            "SELECT attachment_id FROM note_media_tombstones ORDER BY user_id, attachment_id",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        for row in rows {
            if !valid_media_attachment_id(&row?) {
                return Err(StoreError::Integrity(
                    "stored note_media tombstone identity is structurally invalid".to_string(),
                ));
            }
        }
    }
    {
        let mut statement = connection.prepare(
            "SELECT attachment_id, content_sha256, declared_sha256, mime_type, missing_reason
             FROM account_snapshot_media_history
             ORDER BY user_id, account_revision, attachment_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        for row in rows {
            let (attachment_id, content_sha256, declared_sha256, mime_type, missing_reason) = row?;
            let missing_reference_metadata =
                missing_reason == "reference_metadata_unavailable_at_snapshot";
            let declared_sha256_valid = declared_sha256.is_empty()
                || (valid_sha256_hex(&declared_sha256)
                    && declared_sha256 == declared_sha256.to_ascii_lowercase());
            if !valid_media_attachment_id(&attachment_id)
                || (!mime_type.is_empty() && !valid_media_mime_type(&mime_type))
                || (mime_type.is_empty() && !missing_reference_metadata)
                || !declared_sha256_valid
                || content_sha256
                    .as_deref()
                    .is_some_and(|content| declared_sha256 != content.to_ascii_lowercase())
            {
                return Err(StoreError::Integrity(
                    "stored media history metadata is structurally invalid".to_string(),
                ));
            }
        }
    }

    let media_identities = media_identity_pairs(connection)?;
    let mut account_identity_counts = HashMap::<String, i64>::new();
    for (user_id, _) in &media_identities {
        let count = account_identity_counts.entry(user_id.clone()).or_default();
        *count = count.checked_add(1).ok_or_else(|| {
            StoreError::Integrity("media account identity count overflow".to_string())
        })?;
    }
    let over_limit_account_identities = i64::try_from(
        account_identity_counts
            .values()
            .filter(|count| **count > MAX_MEDIA_IDENTITIES_PER_ACCOUNT)
            .count(),
    )
    .map_err(|_| StoreError::Integrity("media over-limit account count overflow".to_string()))?;
    let over_limit_account_retained_bytes = connection.query_row(
        "SELECT COUNT(*) FROM (
             SELECT user_id FROM note_media
             GROUP BY user_id HAVING COALESCE(SUM(size_bytes), 0) > ?1
         )",
        params![MAX_MEDIA_ACCOUNT_BYTES],
        |row| row.get::<_, i64>(0),
    )?;
    let over_limit_account_active_items = connection.query_row(
        "SELECT COUNT(*) FROM (
             SELECT m.user_id
             FROM note_media m
             LEFT JOIN note_media_tombstones t
               ON t.user_id = m.user_id AND t.attachment_id = m.attachment_id
             WHERE m.deleted_at_epoch_millis IS NULL
               AND (t.deleted_revision_epoch_millis IS NULL
                    OR m.updated_at_epoch_millis > t.deleted_revision_epoch_millis)
             GROUP BY m.user_id HAVING COUNT(*) > ?1
         )",
        params![MAX_MEDIA_ITEMS_PER_ACCOUNT],
        |row| row.get::<_, i64>(0),
    )?;
    let global_identities = i64::try_from(media_identities.len())
        .map_err(|_| StoreError::Integrity("global media identity count overflow".to_string()))?;
    let global_retained_bytes = connection.query_row(
        "SELECT
             COALESCE((SELECT SUM(size_bytes) FROM note_media), 0)
             + COALESCE((SELECT SUM(size_bytes) FROM media_snapshot_contents), 0)",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(MediaStorageAudit {
        over_limit_account_identities,
        over_limit_account_retained_bytes,
        over_limit_account_active_items,
        global_identities_over_limit: global_identities > MAX_MEDIA_IDENTITIES_GLOBAL,
        global_retained_bytes_over_limit: global_retained_bytes > MAX_MEDIA_RETAINED_BYTES_GLOBAL,
    })
}

fn verify_semantic_storage_integrity(connection: &Connection) -> StoreResult<()> {
    verify_semantic_storage_integrity_with_privacy(connection, true)
}

fn verify_semantic_storage_integrity_with_privacy(
    connection: &Connection,
    privacy: bool,
) -> StoreResult<()> {
    verify_semantic_storage_integrity_at_schema(connection, if privacy { 15 } else { 14 })
}

fn verify_semantic_storage_integrity_at_schema(
    connection: &Connection,
    version: i64,
) -> StoreResult<()> {
    if version >= 15 {
        note_privacy::verify(connection)?;
    }
    verify_snapshot_content_index(connection)?;
    if version >= 13 {
        verify_current_snapshot_media_identity_index(connection)?;
        let _ = verify_media_storage_invariants(connection)?;
    }

    let (server_instance_id, workspace_capability_secret) = connection
        .query_row(
            "SELECT server_instance_id, workspace_capability_secret
             FROM server_identity WHERE singleton = 1",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
        .ok_or_else(|| StoreError::Integrity("server identity is missing".to_string()))?;
    if !valid_sha256_hex(&server_instance_id)
        || server_instance_id != server_instance_id.to_ascii_lowercase()
    {
        return Err(StoreError::Integrity(
            "server instance identifier is malformed".to_string(),
        ));
    }
    if !valid_workspace_capability_secret(&workspace_capability_secret) {
        return Err(StoreError::Integrity(
            "workspace capability secret is malformed".to_string(),
        ));
    }
    let identity_rows =
        connection.query_row("SELECT COUNT(*) FROM server_identity", [], |row| {
            row.get::<_, i64>(0)
        })?;
    if identity_rows != 1 {
        return Err(StoreError::Integrity(format!(
            "server identity table contains {identity_rows} rows"
        )));
    }
    let mut namespace_statement = connection.prepare(
        "SELECT u.id, a.account_namespace
         FROM users u LEFT JOIN account_namespaces a ON a.user_id = u.id
         ORDER BY u.id",
    )?;
    let namespace_rows = namespace_statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
    })?;
    for row in namespace_rows {
        let (user_id, stored_namespace) = row?;
        let stored_namespace = stored_namespace.ok_or_else(|| {
            StoreError::Integrity(format!("account namespace is missing for user {user_id}"))
        })?;
        let expected_namespace = account_namespace_identifier(&server_instance_id, &user_id);
        if stored_namespace != expected_namespace {
            return Err(StoreError::Integrity(format!(
                "account namespace does not match server identity for user {user_id}"
            )));
        }
    }

    let mut current_snapshot_statement = connection.prepare(
        "SELECT user_id, app_data_json, revision, updated_at_epoch_millis,
                restore_generation, content_sha256, envelope_sha256
         FROM account_snapshots ORDER BY user_id",
    )?;
    let current_snapshot_rows = current_snapshot_statement.query_map([], |row| {
        Ok((
            AccountSnapshot {
                user_id: row.get(0)?,
                app_data_json: row.get(1)?,
                revision: row.get(2)?,
                updated_at_epoch_millis: row.get(3)?,
            },
            row.get::<_, i64>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
        ))
    })?;
    for row in current_snapshot_rows {
        let (snapshot, restore_generation, content_sha256, envelope_sha256) = row?;
        verify_current_account_snapshot(
            &snapshot,
            restore_generation,
            &content_sha256,
            &envelope_sha256,
        )?;
    }

    let mut snapshot_statement = connection.prepare(
        "SELECT '' AS user_id, 0 AS revision, 0 AS created_at_epoch_millis,
                sha256, compression, uncompressed_size_bytes,
                compressed_size_bytes, content
         FROM snapshot_contents ORDER BY sha256",
    )?;
    let snapshot_rows = snapshot_statement.query_map([], snapshot_content_row)?;
    for row in snapshot_rows {
        let row = row?;
        content_verification::verify_compressed_schema(&row)?;
    }

    let mut media_snapshot_statement = connection.prepare(
        "SELECT sha256, size_bytes, content FROM media_snapshot_contents ORDER BY sha256",
    )?;
    let media_snapshot_rows = media_snapshot_statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, Vec<u8>>(2)?,
        ))
    })?;
    for row in media_snapshot_rows {
        let (sha256, size_bytes, content) = row?;
        if !stored_blob_matches_sha256(&sha256, size_bytes, &content) {
            return Err(StoreError::Integrity(format!(
                "media snapshot content is corrupt for SHA-256 {sha256}"
            )));
        }
    }

    let mut live_media_statement = connection.prepare(
        "SELECT user_id, attachment_id, sha256, size_bytes, content
         FROM note_media ORDER BY user_id, attachment_id",
    )?;
    let live_media_rows = live_media_statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, Vec<u8>>(4)?,
        ))
    })?;
    for row in live_media_rows {
        let (user_id, attachment_id, sha256, size_bytes, content) = row?;
        if !stored_blob_matches_sha256(&sha256, size_bytes, &content) {
            return Err(StoreError::Integrity(format!(
                "live media content is corrupt for user {user_id} attachment {attachment_id}"
            )));
        }
    }
    Ok(())
}

fn stored_blob_matches_sha256(sha256: &str, size_bytes: i64, content: &[u8]) -> bool {
    size_bytes >= 0
        && size_bytes as usize == content.len()
        && valid_sha256_hex(sha256)
        && sha256.eq_ignore_ascii_case(&sha256_hex(content))
}

fn read_restore_barrier(
    connection: &Connection,
    user_id: &str,
    token_id: i64,
) -> StoreResult<RestoreBarrierState> {
    connection
        .query_row(
            "SELECT s.restore_generation, t.last_seen_restore_generation,
                    t.restore_acknowledged
             FROM account_snapshots s
             JOIN tokens t ON t.user_id = s.user_id
             WHERE s.user_id = ?1 AND t.id = ?2",
            params![user_id, token_id],
            |row| {
                Ok(RestoreBarrierState {
                    current_generation: row.get(0)?,
                    token_last_seen_generation: row.get(1)?,
                    token_restore_acknowledged: row.get::<_, i64>(2)? == 1,
                })
            },
        )
        .optional()?
        .ok_or_else(|| {
            StoreError::NotFound(format!(
                "restore barrier for user {user_id} token {token_id}"
            ))
        })
}

fn ensure_restore_receipt_in_transaction(
    transaction: &Transaction<'_>,
    user_id: &str,
    token_id: i64,
) -> StoreResult<RestoreReceipt> {
    let state = read_restore_barrier(transaction, user_id, token_id)?;
    let pending = transaction
        .query_row(
            "SELECT pending_restore_generation, pending_restore_receipt
             FROM tokens WHERE id = ?1 AND user_id = ?2",
            params![token_id, user_id],
            |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
        .ok_or_else(|| {
            StoreError::NotFound(format!(
                "restore receipt token {token_id} for user {user_id}"
            ))
        })?;
    let receipt = if pending.0 == Some(state.current_generation) && !pending.1.is_empty() {
        pending.1
    } else {
        let mut bytes = [0_u8; 32];
        OsRng.fill_bytes(&mut bytes);
        let receipt = bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let changed = transaction.execute(
            "UPDATE tokens
             SET restore_acknowledged = 0,
                 pending_restore_generation = ?1,
                 pending_restore_receipt = ?2
             WHERE id = ?3 AND user_id = ?4",
            params![state.current_generation, receipt, token_id, user_id],
        )?;
        if changed != 1 {
            return Err(StoreError::NotFound(format!(
                "restore receipt token {token_id} for user {user_id}"
            )));
        }
        receipt
    };
    Ok(RestoreReceipt {
        current_generation: state.current_generation,
        receipt,
    })
}

fn acknowledge_restore_barrier_in_transaction(
    transaction: &Transaction<'_>,
    user_id: &str,
    restore_barrier: Option<(i64, i64, &str, bool)>,
) -> StoreResult<()> {
    let Some((
        token_id,
        acknowledged_generation,
        restore_receipt,
        allow_receiptless_current_generation,
    )) = restore_barrier
    else {
        return Ok(());
    };
    if acknowledged_generation < 0 {
        return Err(StoreError::Integrity(
            "acknowledged restore generation cannot be negative".to_string(),
        ));
    }
    let state = read_restore_barrier(transaction, user_id, token_id)?;
    if acknowledged_generation > state.current_generation {
        return Err(StoreError::ServerGenerationRollback {
            client_generation: acknowledged_generation,
            server_generation: state.current_generation,
        });
    }
    if acknowledged_generation < state.current_generation {
        return Err(StoreError::RestoreGenerationConflict {
            expected_generation: acknowledged_generation,
            actual_generation: state.current_generation,
        });
    }
    if !state.token_restore_acknowledged && !allow_receiptless_current_generation {
        let pending = transaction
            .query_row(
                "SELECT pending_restore_generation, pending_restore_receipt
                 FROM tokens WHERE id = ?1 AND user_id = ?2",
                params![token_id, user_id],
                |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
            .ok_or_else(|| {
                StoreError::NotFound(format!(
                    "restore barrier token {token_id} for user {user_id}"
                ))
            })?;
        if pending.0 != Some(state.current_generation)
            || pending.1.is_empty()
            || restore_receipt.is_empty()
            || !constant_time_bytes_eq(pending.1.as_bytes(), restore_receipt.as_bytes())
        {
            return Err(StoreError::RestoreReceiptRequired {
                actual_generation: state.current_generation,
            });
        }
    }
    let acknowledged = transaction.execute(
        "UPDATE tokens
         SET last_seen_restore_generation = ?1,
             restore_acknowledged = 1,
             pending_restore_generation = NULL,
             pending_restore_receipt = ''
         WHERE id = ?2 AND user_id = ?3",
        params![acknowledged_generation, token_id, user_id],
    )?;
    if acknowledged != 1 {
        return Err(StoreError::NotFound(format!(
            "restore barrier token {token_id} for user {user_id}"
        )));
    }
    Ok(())
}

fn constant_time_bytes_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (*left ^ *right)
        })
        == 0
}

fn hmac_sha256_hex(key: &[u8], message: &[u8]) -> String {
    const BLOCK_BYTES: usize = 64;
    let mut key_block = [0_u8; BLOCK_BYTES];
    if key.len() > BLOCK_BYTES {
        let digest = Sha256::digest(key);
        key_block[..digest.len()].copy_from_slice(&digest);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut inner_pad = [0x36_u8; BLOCK_BYTES];
    let mut outer_pad = [0x5c_u8; BLOCK_BYTES];
    for index in 0..BLOCK_BYTES {
        inner_pad[index] ^= key_block[index];
        outer_pad[index] ^= key_block[index];
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(message);
    let inner_digest = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner_digest);
    let digest = outer.finalize();
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn valid_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_lowercase_opaque_identifier(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_workspace_capability_secret(value: &str) -> bool {
    valid_lowercase_opaque_identifier(value) && value.bytes().any(|byte| byte != b'0')
}

fn read_workspace_capability_material(
    connection: &Connection,
    user_id: &str,
) -> StoreResult<(String, String, String, i64)> {
    let material = connection
        .query_row(
            "SELECT s.server_instance_id, s.workspace_capability_secret,
                    a.account_namespace, p.restore_generation
             FROM server_identity s
             JOIN account_namespaces a ON a.user_id = ?1
             JOIN account_snapshots p ON p.user_id = a.user_id
             WHERE s.singleton = 1",
            params![user_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| {
            StoreError::NotFound(format!("workspace capability material for user {user_id}"))
        })?;
    if !valid_lowercase_opaque_identifier(&material.0)
        || !valid_workspace_capability_secret(&material.1)
        || material.2 != account_namespace_identifier(&material.0, user_id)
        || material.3 < 0
    {
        return Err(StoreError::Integrity(format!(
            "workspace capability material is malformed for user {user_id}"
        )));
    }
    Ok(material)
}

fn workspace_capability_hmac(
    secret_hex: &str,
    server_instance_id: &str,
    user_id: &str,
    account_namespace: &str,
    workspace_id: &str,
    generation: i64,
) -> StoreResult<String> {
    let secret = decode_lowercase_hex_32(secret_hex)?;
    let mut inner_pad = [0x36_u8; 64];
    let mut outer_pad = [0x5c_u8; 64];
    for (index, byte) in secret.iter().enumerate() {
        inner_pad[index] ^= byte;
        outer_pad[index] ^= byte;
    }
    let mut message = Vec::with_capacity(
        WORKSPACE_CAPABILITY_DOMAIN.len()
            + server_instance_id.len()
            + user_id.len()
            + account_namespace.len()
            + workspace_id.len()
            + 5 * std::mem::size_of::<u64>(),
    );
    message.extend_from_slice(WORKSPACE_CAPABILITY_DOMAIN);
    for component in [
        server_instance_id.as_bytes(),
        user_id.as_bytes(),
        account_namespace.as_bytes(),
        workspace_id.as_bytes(),
    ] {
        message.extend_from_slice(&(component.len() as u64).to_le_bytes());
        message.extend_from_slice(component);
    }
    message.extend_from_slice(&generation.to_le_bytes());

    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(message);
    let inner_digest = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner_digest);
    Ok(outer
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn decode_lowercase_hex_32(value: &str) -> StoreResult<[u8; 32]> {
    if !valid_lowercase_opaque_identifier(value) {
        return Err(StoreError::Integrity(
            "expected 32 bytes encoded as lowercase hexadecimal".to_string(),
        ));
    }
    let mut decoded = [0_u8; 32];
    for (index, byte) in decoded.iter_mut().enumerate() {
        let offset = index * 2;
        *byte = u8::from_str_radix(&value[offset..offset + 2], 16).map_err(|_| {
            StoreError::Integrity("invalid lowercase hexadecimal encoding".to_string())
        })?;
    }
    Ok(decoded)
}

fn random_opaque_identifier() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn account_snapshot_envelope_sha256(
    user_id: &str,
    app_data_json: &str,
    revision: i64,
    updated_at_epoch_millis: i64,
    restore_generation: i64,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"gridtimer-account-snapshot-envelope-v1\0");
    hasher.update((user_id.len() as u64).to_le_bytes());
    hasher.update(user_id.as_bytes());
    hasher.update(revision.to_le_bytes());
    hasher.update(updated_at_epoch_millis.to_le_bytes());
    hasher.update(restore_generation.to_le_bytes());
    hasher.update((app_data_json.len() as u64).to_le_bytes());
    hasher.update(app_data_json.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn ensure_snapshot_write_capacity(connection: &Connection, payload_bytes: u64) -> StoreResult<()> {
    let available = available_database_disk_space(connection)?;
    validate_snapshot_write_capacity(payload_bytes, available)
}

fn ensure_media_write_capacity(connection: &Connection, payload_bytes: u64) -> StoreResult<()> {
    let available = available_database_disk_space(connection)?;
    validate_media_write_capacity(payload_bytes, available)
}

fn ensure_schema_migration_capacity(connection: &Connection) -> StoreResult<()> {
    let database_bytes = database_allocation_bytes(connection)?;
    let available = available_database_disk_space(connection)?;
    validate_schema_migration_capacity(database_bytes, available)
}

fn database_allocation_bytes(connection: &Connection) -> StoreResult<u64> {
    let page_count = connection.query_row("PRAGMA page_count", [], |row| row.get::<_, i64>(0))?;
    let page_size = connection.query_row("PRAGMA page_size", [], |row| row.get::<_, i64>(0))?;
    let page_count = u64::try_from(page_count).map_err(|_| {
        StoreError::Integrity("SQLite page count is negative before schema migration".to_string())
    })?;
    let page_size = u64::try_from(page_size).map_err(|_| {
        StoreError::Integrity("SQLite page size is negative before schema migration".to_string())
    })?;
    let database_bytes = page_count.checked_mul(page_size).ok_or_else(|| {
        StoreError::Integrity("SQLite size overflow before schema migration".to_string())
    })?;
    Ok(database_bytes)
}

fn ensure_schema_v12_reconciliation_capacity(connection: &Connection) -> StoreResult<()> {
    let history_keys = {
        let mut statement = connection.prepare(
            "SELECT user_id, revision FROM account_snapshot_history ORDER BY user_id, revision",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let mut missing_manifest_rows = 0_u64;
    for (user_id, revision) in history_keys {
        let stored = connection.query_row(
            "SELECT h.user_id, h.revision, h.created_at_epoch_millis,
                    c.sha256, c.compression, c.uncompressed_size_bytes,
                    c.compressed_size_bytes, c.content
             FROM account_snapshot_history h
             JOIN snapshot_contents c ON c.sha256 = h.content_sha256
             WHERE h.user_id = ?1 AND h.revision = ?2",
            params![user_id, revision],
            snapshot_content_row,
        )?;
        let history = decode_snapshot_history(stored)?;
        let expected_ids = referenced_attachment_ids_tolerant(&history.app_data_json)?;
        let existing_ids = {
            let mut statement = connection.prepare(
                "SELECT attachment_id FROM account_snapshot_media_history
                 WHERE user_id = ?1 AND account_revision = ?2",
            )?;
            let rows = statement.query_map(params![history.user_id, history.revision], |row| {
                row.get::<_, String>(0)
            })?;
            rows.collect::<Result<HashSet<_>, _>>()?
        };
        let newly_materialized = u64::try_from(expected_ids.difference(&existing_ids).count())
            .map_err(|_| {
                StoreError::Integrity("schema v12 media manifest projection overflow".to_string())
            })?;
        missing_manifest_rows = missing_manifest_rows
            .checked_add(newly_materialized)
            .ok_or_else(|| {
                StoreError::Integrity("schema v12 media manifest projection overflow".to_string())
            })?;
    }
    let metadata_bytes = missing_manifest_rows
        .checked_mul(SCHEMA_MIGRATION_INDEX_ROW_ESTIMATE_BYTES)
        .ok_or_else(|| {
            StoreError::Integrity("schema v12 media metadata size overflow".to_string())
        })?;
    let transient_payload_bytes = database_allocation_bytes(connection)?
        .checked_add(metadata_bytes)
        .ok_or_else(|| StoreError::Integrity("schema v12 size estimate overflow".to_string()))?;
    let available = available_database_disk_space(connection)?;
    validate_schema_migration_capacity(transient_payload_bytes, available)
}

fn ensure_schema_v13_media_index_capacity(connection: &Connection) -> StoreResult<()> {
    let history_rows = connection.query_row(
        "SELECT COUNT(*) FROM account_snapshot_media_history",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    let history_rows = u64::try_from(history_rows).map_err(|_| {
        StoreError::Integrity("schema v13 media history row count is negative".to_string())
    })?;
    let user_ids = {
        let mut statement =
            connection.prepare("SELECT user_id FROM account_snapshots ORDER BY user_id")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let mut current_identity_rows = 0_u64;
    for user_id in user_ids {
        let app_data_json = connection.query_row(
            "SELECT app_data_json FROM account_snapshots WHERE user_id = ?1",
            params![user_id],
            |row| row.get::<_, String>(0),
        )?;
        current_identity_rows = current_identity_rows
            .checked_add(
                u64::try_from(referenced_attachment_ids_tolerant(&app_data_json)?.len()).map_err(
                    |_| {
                        StoreError::Integrity(
                            "schema v13 current identity projection overflow".to_string(),
                        )
                    },
                )?,
            )
            .ok_or_else(|| {
                StoreError::Integrity("schema v13 current identity projection overflow".to_string())
            })?;
    }
    let index_rows = history_rows
        .checked_add(current_identity_rows)
        .ok_or_else(|| StoreError::Integrity("schema v13 index row overflow".to_string()))?;
    let index_bytes = index_rows
        .checked_mul(SCHEMA_MIGRATION_INDEX_ROW_ESTIMATE_BYTES)
        .ok_or_else(|| StoreError::Integrity("schema v13 index size overflow".to_string()))?;
    let transient_payload_bytes = database_allocation_bytes(connection)?
        .checked_add(index_bytes)
        .ok_or_else(|| StoreError::Integrity("schema v13 size estimate overflow".to_string()))?;
    let available = available_database_disk_space(connection)?;
    validate_schema_migration_capacity(transient_payload_bytes, available)
}

fn available_database_disk_space(connection: &Connection) -> StoreResult<Option<u64>> {
    let database_path = connection
        .query_row(
            "SELECT file FROM pragma_database_list WHERE name = 'main'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(database_path) = database_path.filter(|path| !path.is_empty()) else {
        return Ok(None);
    };
    available_disk_space(Path::new(&database_path))
}

fn validate_snapshot_write_capacity(
    payload_bytes: u64,
    available_bytes: Option<u64>,
) -> StoreResult<()> {
    let Some(available_bytes) = available_bytes else {
        return Ok(());
    };
    let required = SNAPSHOT_WRITE_MIN_FREE_BYTES
        .saturating_add(SNAPSHOT_WRITE_OVERHEAD_BYTES)
        .saturating_add(payload_bytes.saturating_mul(2));
    if available_bytes < required {
        return Err(StoreError::DiskReserveExceeded {
            available_bytes,
            required_bytes: required,
        });
    }
    Ok(())
}

fn validate_media_write_capacity(
    payload_bytes: u64,
    available_bytes: Option<u64>,
) -> StoreResult<()> {
    let Some(available_bytes) = available_bytes else {
        return Ok(());
    };
    let required = SNAPSHOT_WRITE_MIN_FREE_BYTES
        .saturating_add(SNAPSHOT_WRITE_OVERHEAD_BYTES)
        .saturating_add(payload_bytes.saturating_mul(2));
    if available_bytes < required {
        return Err(StoreError::DiskReserveExceeded {
            available_bytes,
            required_bytes: required,
        });
    }
    Ok(())
}

fn validate_schema_migration_capacity(
    transient_payload_bytes: u64,
    available_bytes: Option<u64>,
) -> StoreResult<()> {
    // A migration first retains a full verified backup and may then need a
    // second database-sized WAL/temp copy while rebuilding indexes. Reuse the
    // normal two-copy projection so the permanent safety reserve survives
    // either phase and fail before any backup file is created.
    validate_snapshot_write_capacity(transient_payload_bytes, available_bytes)
}

#[cfg(windows)]
fn available_disk_space(database_path: &Path) -> StoreResult<Option<u64>> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(
            directory_name: *const u16,
            free_bytes_available: *mut u64,
            total_number_of_bytes: *mut u64,
            total_number_of_free_bytes: *mut u64,
        ) -> i32;
    }

    let directory = database_path.parent().unwrap_or(database_path);
    let wide = directory
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut available = 0_u64;
    let result = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if result == 0 {
        return Err(StoreError::Io(io::Error::last_os_error()));
    }
    Ok(Some(available))
}

#[cfg(not(windows))]
fn available_disk_space(_database_path: &Path) -> StoreResult<Option<u64>> {
    Ok(None)
}

fn verify_snapshot_history_entry(snapshot: &AccountSnapshotHistory) -> StoreResult<()> {
    let computed = sha256_hex(snapshot.app_data_json.as_bytes());
    if snapshot.sha256.len() != 64
        || !snapshot.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        || !snapshot.sha256.eq_ignore_ascii_case(&computed)
    {
        return Err(StoreError::Integrity(format!(
            "snapshot history SHA-256 mismatch for user {} at revision {}",
            snapshot.user_id, snapshot.revision
        )));
    }
    content_verification::verify_structure(&snapshot.app_data_json)
}

fn verify_current_account_snapshot(
    snapshot: &AccountSnapshot,
    restore_generation: i64,
    stored_content_sha256: &str,
    stored_envelope_sha256: &str,
) -> StoreResult<()> {
    let computed = sha256_hex(snapshot.app_data_json.as_bytes());
    if !valid_sha256_hex(stored_content_sha256)
        || !stored_content_sha256.eq_ignore_ascii_case(&computed)
    {
        return Err(StoreError::Integrity(format!(
            "current account snapshot SHA-256 mismatch for user {} at revision {}",
            snapshot.user_id, snapshot.revision
        )));
    }
    let expected_envelope = account_snapshot_envelope_sha256(
        &snapshot.user_id,
        &snapshot.app_data_json,
        snapshot.revision,
        snapshot.updated_at_epoch_millis,
        restore_generation,
    );
    if !valid_sha256_hex(stored_envelope_sha256)
        || !stored_envelope_sha256.eq_ignore_ascii_case(&expected_envelope)
    {
        return Err(StoreError::Integrity(format!(
            "current account snapshot envelope SHA-256 mismatch for user {} at revision {}",
            snapshot.user_id, snapshot.revision
        )));
    }
    content_verification::verify_structure(&snapshot.app_data_json)
}

pub(crate) fn valid_media_attachment_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=MAX_MEDIA_ATTACHMENT_ID_BYTES).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

pub(crate) fn valid_media_mime_type(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=MAX_MEDIA_MIME_TYPE_BYTES).contains(&bytes.len()) && bytes.iter().all(u8::is_ascii_graphic)
}

fn validate_archivable_app_data_media_metadata(app_data_json: &str) -> StoreResult<()> {
    let referenced_ids = referenced_attachment_ids(app_data_json)?;
    let expectations = referenced_media_expectations(app_data_json)?;
    if expectations.len() != referenced_ids.len()
        || referenced_ids
            .iter()
            .any(|attachment_id| !expectations.contains_key(attachment_id))
        || expectations.values().any(|expected| {
            !valid_sha256_hex(&expected.sha256)
                || expected.sha256 != expected.sha256.to_ascii_lowercase()
                || !valid_media_mime_type(&expected.mime_type)
                || !expected.size_bytes_declared
                || !(0..=MAX_MEDIA_BYTES as i64).contains(&expected.size_bytes)
        })
    {
        return Err(StoreError::Integrity(
            "app data media references are not completely archivable".to_string(),
        ));
    }
    Ok(())
}

fn media_expectation_is_archivable(expected: &ReferencedMediaExpectation) -> bool {
    valid_sha256_hex(&expected.sha256)
        && expected.sha256 == expected.sha256.to_ascii_lowercase()
        && valid_media_mime_type(&expected.mime_type)
        && expected.size_bytes_declared
        && (0..=MAX_MEDIA_BYTES as i64).contains(&expected.size_bytes)
}

pub(crate) fn unresolved_media_metadata_count(app_data_json: &str) -> StoreResult<usize> {
    let ids = referenced_attachment_ids(app_data_json)?;
    let expectations = referenced_media_expectations(app_data_json)?;
    // v2.22.43 - Intentionally deleted media are not missing historical files.
    let data: serde_json::Value = if app_data_json.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(app_data_json)?
    };
    let mut deleted = HashMap::<String, i64>::new();
    for tombstone in data
        .get("tombstones")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        if tombstone
            .get("entityType")
            .and_then(serde_json::Value::as_str)
            != Some("noteMedia")
        {
            continue;
        }
        if let (Some(id), Some(at)) = (
            tombstone
                .get("entityId")
                .and_then(serde_json::Value::as_str),
            tombstone
                .get("deletedAtEpochMillis")
                .and_then(serde_json::Value::as_i64),
        ) {
            if at > 0 {
                deleted
                    .entry(id.to_string())
                    .and_modify(|existing| *existing = (*existing).max(at))
                    .or_insert(at);
            }
        }
    }
    Ok(ids
        .iter()
        .filter(|id| {
            if deleted.get(*id).is_some_and(|at| {
                *at >= expectations
                    .get(*id)
                    .map_or(0, |item| item.updated_at_epoch_millis)
            }) {
                return false;
            }
            !expectations
                .get(*id)
                .is_some_and(media_expectation_is_archivable)
        })
        .count())
}

fn legacy_unhashed_media_references(
    app_data_json: &str,
) -> StoreResult<Vec<crate::sync_core::LegacyMediaReference>> {
    if app_data_json.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut references = referenced_media_expectations(app_data_json)?
        .into_iter()
        .filter(|(_, expected)| {
            expected.sha256.is_empty()
                && valid_media_mime_type(&expected.mime_type)
                && expected.size_bytes_declared
                && (1..=MAX_MEDIA_BYTES as i64).contains(&expected.size_bytes)
        })
        .map(
            |(attachment_id, expected)| crate::sync_core::LegacyMediaReference {
                attachment_id,
                mime_type: expected.mime_type,
                size_bytes: expected.size_bytes,
            },
        )
        .collect::<Vec<_>>();
    references.sort_by(|left, right| left.attachment_id.cmp(&right.attachment_id));
    Ok(references)
}

fn validate_app_data_media_transition(current: &str, incoming: &str) -> StoreResult<()> {
    let incoming_ids = referenced_attachment_ids(incoming)?;
    let incoming_expectations = referenced_media_expectations(incoming)?;
    if incoming_ids.iter().all(|id| {
        incoming_expectations
            .get(id)
            .is_some_and(media_expectation_is_archivable)
    }) {
        return Ok(());
    }
    // The authority for a legacy exception is this account's verified current
    // snapshot, read under the same write transaction. A client cannot create
    // a new incomplete identity or weaken/change an existing content identity.
    // Timestamps may advance during note edits; content metadata may not.
    let current_ids = referenced_attachment_ids(current)?;
    let current_expectations = referenced_media_expectations(current)?;
    for id in incoming_ids {
        let incoming = incoming_expectations.get(&id);
        if incoming.is_some_and(media_expectation_is_archivable) {
            continue;
        }
        let unchanged = match (current_expectations.get(&id), incoming) {
            (None, None) => true,
            (Some(before), Some(after)) => {
                before.sha256 == after.sha256
                    && before.mime_type == after.mime_type
                    && before.size_bytes == after.size_bytes
                    && before.size_bytes_declared == after.size_bytes_declared
            }
            _ => false,
        };
        if !current_ids.contains(&id) || !unchanged {
            return Err(StoreError::Integrity(
                "new or changed attachment metadata is incomplete".to_string(),
            ));
        }
    }
    // History capture still records incomplete media explicitly. All JSON,
    // history, identity, disk-capacity and restore-generation guards still run.
    Ok(())
}

fn media_identity_ids_without_current_for_user(
    connection: &Connection,
    user_id: &str,
) -> StoreResult<HashSet<String>> {
    let mut statement = connection.prepare(
        "SELECT attachment_id FROM note_media WHERE user_id = ?1
         UNION
         SELECT attachment_id FROM note_media_tombstones WHERE user_id = ?1
         UNION
         SELECT attachment_id FROM account_snapshot_media_history WHERE user_id = ?1",
    )?;
    let rows = statement.query_map(params![user_id], |row| row.get::<_, String>(0))?;
    Ok(rows.collect::<Result<HashSet<_>, _>>()?)
}

fn media_identity_ids_for_user(
    connection: &Connection,
    user_id: &str,
) -> StoreResult<HashSet<String>> {
    let mut statement = connection.prepare(
        "SELECT attachment_id FROM note_media WHERE user_id = ?1
         UNION
         SELECT attachment_id FROM note_media_tombstones WHERE user_id = ?1
         UNION
         SELECT attachment_id FROM account_snapshot_media_history WHERE user_id = ?1
         UNION
         SELECT attachment_id FROM account_snapshot_media_identities WHERE user_id = ?1",
    )?;
    let rows = statement.query_map(params![user_id], |row| row.get::<_, String>(0))?;
    Ok(rows.collect::<Result<HashSet<_>, _>>()?)
}

fn media_identity_pairs(connection: &Connection) -> StoreResult<HashSet<(String, String)>> {
    let mut statement = connection.prepare(
        "SELECT user_id, attachment_id FROM note_media
         UNION
         SELECT user_id, attachment_id FROM note_media_tombstones
         UNION
         SELECT user_id, attachment_id FROM account_snapshot_media_history
         UNION
         SELECT user_id, attachment_id FROM account_snapshot_media_identities",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    Ok(rows.collect::<Result<HashSet<_>, _>>()?)
}

fn validate_app_data_media_identity_quota(
    transaction: &Transaction<'_>,
    user_id: &str,
    app_data_json: &str,
) -> StoreResult<()> {
    let incoming_ids = referenced_attachment_ids(app_data_json)?;
    let persistent_ids = media_identity_ids_without_current_for_user(transaction, user_id)?;
    let current_ids = {
        let mut statement = transaction.prepare(
            "SELECT attachment_id FROM account_snapshot_media_identities WHERE user_id = ?1",
        )?;
        let rows = statement.query_map(params![user_id], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<HashSet<_>, _>>()?
    };
    let current_account_identities = i64::try_from(persistent_ids.union(&current_ids).count())
        .map_err(|_| StoreError::Integrity("media account identity count overflow".to_string()))?;
    let projected_account_identities =
        i64::try_from(persistent_ids.union(&incoming_ids).count())
            .map_err(|_| StoreError::Integrity("media identity projection overflow".to_string()))?;
    if projected_account_identities > MAX_MEDIA_IDENTITIES_PER_ACCOUNT
        && projected_account_identities > current_account_identities
    {
        return Err(StoreError::MediaAccountIdentityQuotaExceeded {
            usage_items: current_account_identities,
            projected_items: projected_account_identities,
            limit_items: MAX_MEDIA_IDENTITIES_PER_ACCOUNT,
        });
    }
    let global_identities = i64::try_from(media_identity_pairs(transaction)?.len())
        .map_err(|_| StoreError::Integrity("global media identity count overflow".to_string()))?;
    let projected_global_identities = global_identities
        .checked_sub(current_account_identities)
        .and_then(|value| value.checked_add(projected_account_identities))
        .ok_or_else(|| {
            StoreError::Integrity("global media identity projection overflow".to_string())
        })?;
    if projected_global_identities > MAX_MEDIA_IDENTITIES_GLOBAL
        && projected_global_identities > global_identities
    {
        return Err(StoreError::MediaServerIdentityQuotaExceeded {
            usage_items: global_identities,
            projected_items: projected_global_identities,
            limit_items: MAX_MEDIA_IDENTITIES_GLOBAL,
        });
    }
    Ok(())
}

fn validate_media_identity_quota(
    transaction: &Transaction<'_>,
    user_id: &str,
    attachment_id: &str,
) -> StoreResult<()> {
    validate_media_identity_quota_with_limits(
        transaction,
        user_id,
        attachment_id,
        MAX_MEDIA_IDENTITIES_PER_ACCOUNT,
        MAX_MEDIA_IDENTITIES_GLOBAL,
    )
}

fn validate_media_identity_quota_with_limits(
    transaction: &Transaction<'_>,
    user_id: &str,
    attachment_id: &str,
    account_limit: i64,
    global_limit: i64,
) -> StoreResult<()> {
    let attachment_ids = HashSet::from([attachment_id.to_string()]);
    validate_media_identity_batch_quota_with_limits(
        transaction,
        user_id,
        &attachment_ids,
        account_limit,
        global_limit,
    )
}

fn validate_media_identity_batch_quota(
    transaction: &Transaction<'_>,
    user_id: &str,
    attachment_ids: &HashSet<String>,
) -> StoreResult<()> {
    validate_media_identity_batch_quota_with_limits(
        transaction,
        user_id,
        attachment_ids,
        MAX_MEDIA_IDENTITIES_PER_ACCOUNT,
        MAX_MEDIA_IDENTITIES_GLOBAL,
    )
}

fn validate_media_identity_batch_quota_with_limits(
    transaction: &Transaction<'_>,
    user_id: &str,
    attachment_ids: &HashSet<String>,
    account_limit: i64,
    global_limit: i64,
) -> StoreResult<()> {
    if attachment_ids.is_empty() {
        return Ok(());
    }
    let existing_account_ids = media_identity_ids_for_user(transaction, user_id)?;
    let account_identities = i64::try_from(existing_account_ids.len())
        .map_err(|_| StoreError::Integrity("media account identity count overflow".to_string()))?;
    let new_identities = i64::try_from(attachment_ids.difference(&existing_account_ids).count())
        .map_err(|_| StoreError::Integrity("media identity projection overflow".to_string()))?;
    let projected_account_identities = account_identities
        .checked_add(new_identities)
        .ok_or_else(|| StoreError::Integrity("media identity projection overflow".to_string()))?;
    if projected_account_identities > account_limit
        && projected_account_identities > account_identities
    {
        return Err(StoreError::MediaAccountIdentityQuotaExceeded {
            usage_items: account_identities,
            projected_items: projected_account_identities,
            limit_items: account_limit,
        });
    }
    let global_identities = i64::try_from(media_identity_pairs(transaction)?.len())
        .map_err(|_| StoreError::Integrity("global media identity count overflow".to_string()))?;
    let projected_global_identities =
        global_identities
            .checked_add(new_identities)
            .ok_or_else(|| {
                StoreError::Integrity("global media identity projection overflow".to_string())
            })?;
    if projected_global_identities > global_limit && projected_global_identities > global_identities
    {
        return Err(StoreError::MediaServerIdentityQuotaExceeded {
            usage_items: global_identities,
            projected_items: projected_global_identities,
            limit_items: global_limit,
        });
    }
    Ok(())
}

fn validate_media_identity_count(
    current_identities: i64,
    identity_exists: bool,
) -> StoreResult<()> {
    validate_media_identity_count_with_limit(
        current_identities,
        identity_exists,
        MAX_MEDIA_IDENTITIES_PER_ACCOUNT,
    )
}

fn validate_media_identity_count_with_limit(
    current_identities: i64,
    identity_exists: bool,
    limit: i64,
) -> StoreResult<()> {
    if !identity_exists && current_identities >= limit {
        return Err(StoreError::MediaAccountIdentityQuotaExceeded {
            usage_items: current_identities,
            projected_items: current_identities.saturating_add(1),
            limit_items: limit,
        });
    }
    Ok(())
}

fn validate_media_retained_quota(
    current_retained_bytes: i64,
    existing_item_bytes: Option<i64>,
    incoming_bytes: i64,
) -> StoreResult<()> {
    let projected_bytes =
        checked_media_projection(current_retained_bytes, existing_item_bytes, incoming_bytes)?;
    if projected_bytes > MAX_MEDIA_ACCOUNT_BYTES && projected_bytes > current_retained_bytes {
        return Err(StoreError::Integrity(
            "media retained byte quota exceeded".to_string(),
        ));
    }
    Ok(())
}

fn validate_media_quota(
    current_items: i64,
    current_bytes: i64,
    existing_item_bytes: Option<i64>,
    incoming_bytes: i64,
) -> StoreResult<()> {
    let projected_items = current_items
        .checked_add(i64::from(existing_item_bytes.is_none()))
        .ok_or_else(|| StoreError::Integrity("media item projection overflow".to_string()))?;
    let projected_bytes =
        checked_media_projection(current_bytes, existing_item_bytes, incoming_bytes)?;
    if projected_items > MAX_MEDIA_ITEMS_PER_ACCOUNT && projected_items > current_items {
        return Err(StoreError::Integrity(
            "media item limit exceeded".to_string(),
        ));
    }
    if projected_bytes > MAX_MEDIA_ACCOUNT_BYTES && projected_bytes > current_bytes {
        return Err(StoreError::Integrity(
            "media account byte quota exceeded".to_string(),
        ));
    }
    Ok(())
}

fn checked_media_projection(
    current_bytes: i64,
    existing_item_bytes: Option<i64>,
    incoming_bytes: i64,
) -> StoreResult<i64> {
    if current_bytes < 0 || incoming_bytes < 0 || existing_item_bytes.is_some_and(|value| value < 0)
    {
        return Err(StoreError::Integrity(
            "media byte projection contains a negative value".to_string(),
        ));
    }
    current_bytes
        .checked_sub(existing_item_bytes.unwrap_or(0))
        .and_then(|value| value.checked_add(incoming_bytes))
        .ok_or_else(|| StoreError::Integrity("media byte projection overflow".to_string()))
}

fn global_media_retained_bytes(connection: &Connection) -> StoreResult<i64> {
    connection
        .query_row(
            "SELECT
                 COALESCE((SELECT SUM(size_bytes) FROM note_media), 0)
                 + COALESCE((SELECT SUM(size_bytes) FROM media_snapshot_contents), 0)",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(StoreError::from)
}

fn validate_media_server_retained_growth(
    connection: &Connection,
    existing_item_bytes: Option<i64>,
    incoming_bytes: i64,
) -> StoreResult<()> {
    validate_media_server_retained_growth_with_limit(
        global_media_retained_bytes(connection)?,
        existing_item_bytes,
        incoming_bytes,
        MAX_MEDIA_RETAINED_BYTES_GLOBAL,
    )
}

fn validate_media_server_retained_growth_with_limit(
    current_bytes: i64,
    existing_item_bytes: Option<i64>,
    incoming_bytes: i64,
    limit_bytes: i64,
) -> StoreResult<()> {
    let projected_bytes =
        checked_media_projection(current_bytes, existing_item_bytes, incoming_bytes)?;
    if projected_bytes > limit_bytes && projected_bytes > current_bytes {
        return Err(StoreError::MediaServerRetainedQuotaExceeded {
            usage_bytes: current_bytes,
            projected_bytes,
            limit_bytes,
        });
    }
    Ok(())
}

fn media_metadata_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MediaMetadata> {
    Ok(MediaMetadata {
        user_id: row.get(0)?,
        attachment_id: row.get(1)?,
        sha256: row.get(2)?,
        mime_type: row.get(3)?,
        size_bytes: row.get(4)?,
        updated_at_epoch_millis: row.get(5)?,
        deleted_at_epoch_millis: row.get(6)?,
    })
}

fn read_media_from_connection(
    connection: &Connection,
    user_id: &str,
    attachment_id: &str,
) -> StoreResult<Option<StoredMedia>> {
    let stored = connection
        .query_row(
            "SELECT user_id, attachment_id, sha256, mime_type, size_bytes,
                    updated_at_epoch_millis, deleted_at_epoch_millis, content,
                    (SELECT deleted_revision_epoch_millis
                     FROM note_media_tombstones t
                     WHERE t.user_id = note_media.user_id
                       AND t.attachment_id = note_media.attachment_id)
             FROM note_media
             WHERE user_id = ?1 AND attachment_id = ?2",
            params![user_id, attachment_id],
            |row| {
                let metadata = MediaMetadata {
                    user_id: row.get(0)?,
                    attachment_id: row.get(1)?,
                    sha256: row.get(2)?,
                    mime_type: row.get(3)?,
                    size_bytes: row.get(4)?,
                    updated_at_epoch_millis: row.get(5)?,
                    deleted_at_epoch_millis: row.get(6)?,
                };
                let tombstone_revision = row.get::<_, Option<i64>>(8)?;
                Ok(StoredMedia {
                    metadata: media_metadata_with_effective_tombstone(metadata, tombstone_revision),
                    content: row.get(7)?,
                })
            },
        )
        .optional()
        .map_err(StoreError::from)?;
    if let Some(media) = stored.as_ref() {
        if !stored_blob_matches_sha256(
            &media.metadata.sha256,
            media.metadata.size_bytes,
            &media.content,
        ) {
            return Err(StoreError::Integrity(format!(
                "live media content is corrupt for user {} attachment {}",
                media.metadata.user_id, media.metadata.attachment_id
            )));
        }
    }
    Ok(stored.filter(|media| media.metadata.deleted_at_epoch_millis.is_none()))
}

fn list_media_metadata_from_connection(
    connection: &Connection,
    user_id: &str,
    include_deleted: bool,
) -> StoreResult<Vec<MediaMetadata>> {
    let mut statement = connection.prepare(
        "SELECT user_id, attachment_id, sha256, mime_type, size_bytes,
                 updated_at_epoch_millis, deleted_at_epoch_millis
         FROM note_media
         WHERE user_id = ?1",
    )?;
    let rows = statement.query_map(params![user_id], media_metadata_from_row)?;
    let mut items = rows
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|metadata| (metadata.attachment_id.clone(), metadata))
        .collect::<HashMap<_, _>>();
    drop(statement);
    let mut tombstone_statement = connection.prepare(
        "SELECT attachment_id, deleted_revision_epoch_millis
         FROM note_media_tombstones WHERE user_id = ?1",
    )?;
    let tombstones = tombstone_statement.query_map(params![user_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    for tombstone in tombstones {
        let (attachment_id, deleted_revision) = tombstone?;
        match items.get_mut(&attachment_id) {
            Some(metadata) => {
                *metadata = media_metadata_with_effective_tombstone(
                    metadata.clone(),
                    Some(deleted_revision),
                );
            }
            None => {
                items.insert(
                    attachment_id.clone(),
                    media_tombstone_only_metadata(user_id, &attachment_id, deleted_revision),
                );
            }
        }
    }
    drop(tombstone_statement);
    let mut items = items
        .into_values()
        .filter(|metadata| include_deleted || metadata.deleted_at_epoch_millis.is_none())
        .collect::<Vec<_>>();
    items.sort_by(|left, right| {
        right
            .deleted_at_epoch_millis
            .unwrap_or(right.updated_at_epoch_millis)
            .cmp(
                &left
                    .deleted_at_epoch_millis
                    .unwrap_or(left.updated_at_epoch_millis),
            )
            .then_with(|| left.attachment_id.cmp(&right.attachment_id))
    });
    Ok(items)
}

fn media_tombstone_revision(
    transaction: &Transaction<'_>,
    user_id: &str,
    attachment_id: &str,
) -> StoreResult<Option<i64>> {
    transaction
        .query_row(
            "SELECT deleted_revision_epoch_millis
             FROM note_media_tombstones
             WHERE user_id = ?1 AND attachment_id = ?2",
            params![user_id, attachment_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(StoreError::from)
}

fn media_metadata_with_effective_tombstone(
    mut metadata: MediaMetadata,
    tombstone_revision: Option<i64>,
) -> MediaMetadata {
    let effective_deleted = match (metadata.deleted_at_epoch_millis, tombstone_revision) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    };
    metadata.deleted_at_epoch_millis = effective_deleted
        .filter(|deleted_revision| *deleted_revision >= metadata.updated_at_epoch_millis);
    metadata
}

fn max_optional_revision(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn media_tombstone_only_metadata(
    user_id: &str,
    attachment_id: &str,
    deleted_revision_epoch_millis: i64,
) -> MediaMetadata {
    MediaMetadata {
        user_id: user_id.to_string(),
        attachment_id: attachment_id.to_string(),
        sha256: String::new(),
        mime_type: String::new(),
        size_bytes: 0,
        updated_at_epoch_millis: 0,
        deleted_at_epoch_millis: Some(deleted_revision_epoch_millis),
    }
}

fn validate_new_user(user: &NewStoredUser) -> StoreResult<()> {
    if user.id.trim().is_empty() {
        return Err(StoreError::Integrity("user id cannot be empty".to_string()));
    }
    if user.email.trim().is_empty() || !user.email.contains('@') {
        return Err(StoreError::Integrity(format!(
            "invalid email for user {}",
            user.id
        )));
    }
    if user.password_hash.is_empty() || user.password_scheme.is_empty() {
        return Err(StoreError::Integrity(format!(
            "password material is missing for user {}",
            user.id
        )));
    }
    if user.password_scheme == "legacy_sha256" && user.password_salt.is_empty() {
        return Err(StoreError::Integrity(format!(
            "legacy password salt is missing for user {}",
            user.id
        )));
    }
    if user.account_revision < 0 {
        return Err(StoreError::Integrity(format!(
            "negative account revision for user {}",
            user.id
        )));
    }
    if user.account_revision == i64::MAX {
        return Err(StoreError::Integrity(format!(
            "exhausted account revision for user {}",
            user.id
        )));
    }
    validate_app_data_json(&user.app_data_json)
}

fn validate_legacy_store(legacy: &LegacyServerStore) -> StoreResult<()> {
    let mut ids = HashSet::new();
    let mut emails = HashSet::new();
    let mut token_hashes = HashSet::new();
    for user in &legacy.users {
        validate_new_user(&NewStoredUser {
            id: user.id.clone(),
            email: user.email.clone(),
            password_salt: user.password_salt.clone(),
            password_hash: user.password_hash.clone(),
            password_scheme: "legacy_sha256".to_string(),
            created_at_epoch_millis: user.created_at_epoch_millis,
            updated_at_epoch_millis: user.updated_at_epoch_millis,
            app_data_json: user.app_data_json.clone(),
            account_revision: 0,
        })?;
        if !ids.insert(user.id.clone()) {
            return Err(StoreError::Integrity(format!(
                "duplicate legacy user id {}",
                user.id
            )));
        }
        if !emails.insert(user.email.trim().to_ascii_lowercase()) {
            return Err(StoreError::Integrity(format!(
                "duplicate legacy email {}",
                user.email
            )));
        }
        for token in &user.tokens {
            if token.token.is_empty() {
                return Err(StoreError::Integrity(format!(
                    "empty legacy token for user {}",
                    user.id
                )));
            }
            let hash = token_fingerprint(&token.token);
            if !token_hashes.insert(hash) {
                return Err(StoreError::Integrity(
                    "duplicate bearer token in legacy store".to_string(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_app_data_json(app_data_json: &str) -> StoreResult<()> {
    let Some(value) = validate_app_data_json_structure(app_data_json)? else {
        return Ok(());
    };
    let root = value
        .as_object()
        .expect("structure validation requires an object");
    let Some(schema_version) = root.get("schemaVersion") else {
        return Ok(());
    };
    let is_future = match schema_version {
        serde_json::Value::Number(number) => number
            .as_i64()
            .map(|version| version > i64::from(APP_DATA_SCHEMA_VERSION))
            .or_else(|| {
                number
                    .as_u64()
                    .map(|version| version > APP_DATA_SCHEMA_VERSION as u64)
            })
            .ok_or_else(|| {
                StoreError::Integrity("app data schemaVersion must be an integer".to_string())
            })?,
        _ => {
            return Err(StoreError::Integrity(
                "app data schemaVersion must be an integer".to_string(),
            ));
        }
    };
    if is_future {
        return Err(StoreError::Integrity(format!(
            "app data schemaVersion is newer than supported version {}; upgrade required",
            APP_DATA_SCHEMA_VERSION
        )));
    }
    Ok(())
}

fn validate_app_data_growth_quota(current_json: &str, projected_json: &str) -> StoreResult<()> {
    validate_app_data_growth_quota_with_limit(
        current_json,
        projected_json,
        ACCOUNT_APP_DATA_HARD_LIMIT_BYTES,
    )
}

fn validate_app_data_growth_quota_with_limit(
    current_json: &str,
    projected_json: &str,
    limit_bytes: i64,
) -> StoreResult<()> {
    if limit_bytes < 1 {
        return Err(StoreError::Integrity(
            "account app-data quota must be positive".to_string(),
        ));
    }
    let current_bytes = current_json.len() as i64;
    let projected_bytes = projected_json.len() as i64;
    if projected_bytes > limit_bytes && projected_bytes > current_bytes {
        return Err(StoreError::AppDataQuotaExceeded {
            current_bytes,
            projected_bytes,
            limit_bytes,
        });
    }
    Ok(())
}

fn validate_app_data_json_structure(app_data_json: &str) -> StoreResult<Option<serde_json::Value>> {
    if app_data_json.trim().is_empty() {
        return Ok(None);
    }
    let value = serde_json::from_str::<serde_json::Value>(app_data_json)?;
    if !value.is_object() {
        return Err(StoreError::Integrity(
            "app data root must be a JSON object".to_string(),
        ));
    }
    Ok(Some(value))
}

fn validate_json_document(raw: &str, label: &str) -> StoreResult<()> {
    if raw.trim().is_empty() {
        return Err(StoreError::Integrity(format!("{label} cannot be empty")));
    }
    serde_json::from_str::<serde_json::Value>(raw)
        .map(|_| ())
        .map_err(StoreError::from)
}

fn canonical_json_sha256(raw: &str) -> StoreResult<String> {
    let value = serde_json::from_str::<serde_json::Value>(raw)?;
    let mut canonical = Vec::with_capacity(raw.len());
    write_canonical_json(&value, &mut canonical)?;
    Ok(sha256_hex(&canonical))
}

fn write_canonical_json(value: &serde_json::Value, output: &mut Vec<u8>) -> StoreResult<()> {
    match value {
        serde_json::Value::Null => output.extend_from_slice(b"null"),
        serde_json::Value::Bool(value) => {
            output.extend_from_slice(if *value { b"true" } else { b"false" })
        }
        serde_json::Value::Number(value) => output.extend_from_slice(value.to_string().as_bytes()),
        serde_json::Value::String(value) => serde_json::to_writer(output, value)?,
        serde_json::Value::Array(values) => {
            output.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_canonical_json(value, output)?;
            }
            output.push(b']');
        }
        serde_json::Value::Object(values) => {
            output.push(b'{');
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                serde_json::to_writer(&mut *output, key)?;
                output.push(b':');
                write_canonical_json(&values[key], output)?;
            }
            output.push(b'}');
        }
    }
    Ok(())
}

fn prune_request_dedup_bytes_for_user(
    transaction: &Transaction<'_>,
    user_id: &str,
    max_response_bytes: i64,
) -> StoreResult<()> {
    let mut total_bytes = transaction.query_row(
        "SELECT COALESCE(SUM(length(CAST(response_json AS BLOB))), 0) \
         FROM request_dedup WHERE user_id = ?1",
        params![user_id],
        |row| row.get::<_, i64>(0),
    )?;
    while total_bytes > max_response_bytes {
        let oldest = transaction
            .query_row(
                "SELECT rowid, length(CAST(response_json AS BLOB)) \
                 FROM request_dedup WHERE user_id = ?1 \
                 ORDER BY created_at_epoch_millis, rowid LIMIT 1",
                params![user_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?
            .ok_or_else(|| {
                StoreError::Integrity(
                    "request deduplication byte accounting is inconsistent".to_string(),
                )
            })?;
        let deleted = transaction.execute(
            "DELETE FROM request_dedup WHERE rowid = ?1",
            params![oldest.0],
        )?;
        if deleted != 1 {
            return Err(StoreError::Integrity(
                "request deduplication pruning changed an unexpected row count".to_string(),
            ));
        }
        total_bytes = total_bytes.saturating_sub(oldest.1);
    }
    Ok(())
}

fn stored_user_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredUser> {
    let id = row.get::<_, String>(0)?;
    Ok(StoredUser {
        id: id.clone(),
        email: row.get(1)?,
        password_salt: row.get(2)?,
        password_hash: row.get(3)?,
        password_scheme: row.get(4)?,
        created_at_epoch_millis: row.get(5)?,
        updated_at_epoch_millis: row.get(6)?,
        account: AccountSnapshot {
            user_id: id,
            app_data_json: row.get(7)?,
            revision: row.get(8)?,
            updated_at_epoch_millis: row.get(9)?,
        },
    })
}

fn verify_quick_check(connection: &Connection) -> StoreResult<()> {
    let result =
        connection.query_row("PRAGMA quick_check(1)", [], |row| row.get::<_, String>(0))?;
    if result.eq_ignore_ascii_case("ok") {
        Ok(())
    } else {
        Err(StoreError::Integrity(format!(
            "SQLite quick_check returned {result}"
        )))
    }
}

fn verify_integrity_check(connection: &Connection) -> StoreResult<()> {
    let mut statement = connection.prepare("PRAGMA integrity_check")?;
    let mut rows = statement.query([])?;
    let mut messages = Vec::new();
    while let Some(row) = rows.next()? {
        messages.push(row.get::<_, String>(0)?);
        if messages.len() >= 8 {
            break;
        }
    }
    if messages.len() == 1 && messages[0].eq_ignore_ascii_case("ok") {
        Ok(())
    } else {
        Err(StoreError::Integrity(format!(
            "SQLite integrity_check returned {}",
            messages.join("; ")
        )))
    }
}

fn verify_foreign_keys(connection: &Connection) -> StoreResult<()> {
    let enabled = connection.query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))?;
    if enabled != 1 {
        return Err(StoreError::Integrity(
            "SQLite foreign_keys is not enabled".to_string(),
        ));
    }
    let mut statement = connection.prepare("PRAGMA foreign_key_check")?;
    if statement.query([])?.next()?.is_some() {
        return Err(StoreError::Integrity(
            "SQLite foreign_key_check found a violation".to_string(),
        ));
    }
    Ok(())
}

fn verify_required_schema(connection: &Connection) -> StoreResult<()> {
    verify_required_schema_at_version(connection, SCHEMA_VERSION)
}

fn verify_required_schema_at_version(
    connection: &Connection,
    expected_version: i64,
) -> StoreResult<()> {
    if !matches!(expected_version, 14..=17) {
        return Err(StoreError::Integrity(
            "unsupported recovery schema layout".into(),
        ));
    }
    let privacy = expected_version >= 15;
    for table in [
        "schema_migrations",
        "server_identity",
        "account_namespaces",
        "users",
        "account_snapshots",
        "account_note_privacy",
        "note_privacy_commit_witnesses",
        "account_snapshot_history",
        "account_snapshot_media_identities",
        "legacy_snapshot_repair_allowances",
        "snapshot_contents",
        "snapshot_history_prune_audit",
        "media_snapshot_contents",
        "account_snapshot_media_history",
        "tokens",
        "legacy_imports",
        "legacy_snapshot_bindings",
        "request_dedup",
        "note_media",
        "note_media_tombstones",
    ] {
        if matches!(
            table,
            "account_note_privacy" | "note_privacy_commit_witnesses"
        ) && !privacy
        {
            continue;
        }
        let exists = connection
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                params![table],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            return Err(StoreError::Integrity(format!(
                "required table is missing: {table}"
            )));
        }
    }
    if expected_version >= 17 {
        legal_reports::verify_schema(connection)?;
    }
    if privacy {
        let privacy_sql: String = connection.query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='account_note_privacy'",
            [],
            |row| row.get(0),
        )?;
        if normalized_schema_sql(&privacy_sql) != normalized_schema_sql(note_privacy::schema_sql())
        {
            return Err(StoreError::Integrity(
                "required note privacy table is divergent".to_string(),
            ));
        }
        let witness_sql: String = connection.query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='note_privacy_commit_witnesses'", [], |row| row.get(0))?;
        if normalized_schema_sql(&witness_sql)
            != normalized_schema_sql(privacy_journal::witness_schema_sql())
        {
            return Err(StoreError::Integrity(
                "required privacy commit witness table is divergent".to_string(),
            ));
        }
    }
    let current_identity_table_sql = connection
        .query_row(
            "SELECT sql FROM sqlite_master
             WHERE type = 'table' AND name = 'account_snapshot_media_identities'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if current_identity_table_sql.as_deref().is_none_or(|stored| {
        normalized_schema_sql(stored)
            != normalized_schema_sql(&current_snapshot_media_identity_table_sql())
    }) {
        return Err(StoreError::Integrity(
            "required current snapshot media identity table is missing or divergent".to_string(),
        ));
    }
    let repair_allowance_table_sql = connection
        .query_row(
            "SELECT sql FROM sqlite_master
             WHERE type = 'table' AND name = 'legacy_snapshot_repair_allowances'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if repair_allowance_table_sql.as_deref().is_none_or(|stored| {
        normalized_schema_sql(stored)
            != normalized_schema_sql(legacy_snapshot_repair_allowance_table_sql())
    }) {
        return Err(StoreError::Integrity(
            "required legacy snapshot repair allowance table is missing or divergent".to_string(),
        ));
    }
    if !table_has_column(connection, "account_snapshots", "content_sha256")? {
        return Err(StoreError::Integrity(
            "required account snapshot content hash column is missing".to_string(),
        ));
    }
    if !table_has_column(connection, "account_snapshots", "envelope_sha256")? {
        return Err(StoreError::Integrity(
            "required account snapshot envelope hash column is missing".to_string(),
        ));
    }
    if !table_has_column(connection, "server_identity", "workspace_capability_secret")? {
        return Err(StoreError::Integrity(
            "required workspace capability secret column is missing".to_string(),
        ));
    }
    if !table_has_column(connection, "tokens", "activation_state")? {
        return Err(StoreError::Integrity(
            "required token activation state column is missing".to_string(),
        ));
    }
    if !table_has_column(connection, "tokens", "activated_at_epoch_millis")? {
        return Err(StoreError::Integrity(
            "required token activation timestamp column is missing".to_string(),
        ));
    }
    let pending_activation_index_exists = connection
        .query_row(
            "SELECT 1 FROM sqlite_master \
             WHERE type = 'index' AND name = 'tokens_pending_activation_index'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !pending_activation_index_exists {
        return Err(StoreError::Integrity(
            "required pending token activation index is missing".to_string(),
        ));
    }
    for (trigger, target_table) in [
        ("note_media_metadata_insert_guard", "note_media"),
        ("note_media_metadata_update_guard", "note_media"),
        ("note_media_tombstone_insert_guard", "note_media_tombstones"),
        ("note_media_tombstone_update_guard", "note_media_tombstones"),
        (
            "media_history_metadata_insert_guard",
            "account_snapshot_media_history",
        ),
        (
            "media_history_metadata_update_guard",
            "account_snapshot_media_history",
        ),
    ] {
        let stored = connection
            .query_row(
                "SELECT tbl_name, sql FROM sqlite_master
                 WHERE type = 'trigger' AND name = ?1",
                params![trigger],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let Some((stored_target, stored_sql)) = stored else {
            return Err(StoreError::Integrity(format!(
                "required media metadata trigger is missing: {trigger}"
            )));
        };
        let canonical_sql = canonical_media_metadata_trigger_sql(trigger)?;
        if stored_target != target_table
            || normalized_schema_sql(&stored_sql) != normalized_schema_sql(&canonical_sql)
        {
            return Err(StoreError::Integrity(format!(
                "required media metadata trigger definition diverged: {trigger}"
            )));
        }
    }
    let identity_index_sql = connection
        .query_row(
            "SELECT sql FROM sqlite_master
             WHERE type = 'index' AND name = 'account_snapshot_media_history_identity_index'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let expected_identity_index_sql =
        "CREATE INDEX account_snapshot_media_history_identity_index ON account_snapshot_media_history(user_id, attachment_id)";
    if identity_index_sql.as_deref().is_none_or(|stored| {
        normalized_schema_sql(stored) != normalized_schema_sql(expected_identity_index_sql)
    }) {
        return Err(StoreError::Integrity(
            "required media history identity index is missing or divergent".to_string(),
        ));
    }
    let migration_version = current_schema_version(connection)?;
    if migration_version != expected_version {
        return Err(StoreError::Integrity(format!(
            "expected schema migration {expected_version}, found {migration_version}"
        )));
    }
    Ok(())
}

fn write_timestamped_backup(source: &Path, raw: &[u8], timestamp: i64) -> StoreResult<PathBuf> {
    let parent = source.parent().unwrap_or_else(|| Path::new("."));
    let stem = source
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("server_store");
    let container = protect_legacy_backup(raw)?;
    for sequence in 0..10_000_u32 {
        let suffix = if sequence == 0 {
            String::new()
        } else {
            format!("_{sequence}")
        };
        let backup_path = parent.join(format!(
            "{stem}_pre_sqlite_migration_{timestamp}{suffix}.gtlbak"
        ));
        if backup_path.exists() {
            continue;
        }
        let temp_path = parent.join(format!(
            ".{stem}_pre_sqlite_migration_{timestamp}{suffix}.{}.tmp",
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = match options.open(&temp_path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(StoreError::Io(error)),
        };
        if let Err(error) = file.write_all(&container).and_then(|_| file.sync_all()) {
            drop(file);
            let _ = fs::remove_file(&temp_path);
            return Err(StoreError::Io(error));
        }
        drop(file);
        match fs::rename(&temp_path, &backup_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let _ = fs::remove_file(&temp_path);
                continue;
            }
            Err(error) => {
                let _ = fs::remove_file(&temp_path);
                return Err(StoreError::Io(error));
            }
        }
        let verified = fs::read(&backup_path)
            .map_err(StoreError::Io)
            .and_then(|bytes| unprotect_legacy_backup(&bytes));
        match verified {
            Ok(restored) if restored == raw => return Ok(backup_path),
            Ok(_) => {
                let _ = fs::remove_file(&backup_path);
                return Err(StoreError::Integrity(
                    "encrypted legacy backup did not restore the exact source bytes".to_string(),
                ));
            }
            Err(error) => {
                let _ = fs::remove_file(&backup_path);
                return Err(error);
            }
        }
    }
    Err(StoreError::Integrity(
        "could not allocate a unique legacy backup name".to_string(),
    ))
}

fn protect_legacy_backup(raw: &[u8]) -> StoreResult<Vec<u8>> {
    #[cfg(target_os = "windows")]
    let (flags, payload) = (
        LEGACY_BACKUP_FLAG_DPAPI_CURRENT_USER,
        dpapi_protect(raw).map_err(StoreError::Io)?,
    );
    #[cfg(not(target_os = "windows"))]
    let (flags, payload) = (LEGACY_BACKUP_FLAG_RESTRICTED_PLAINTEXT, raw.to_vec());

    let digest = Sha256::digest(raw);
    let mut container = Vec::with_capacity(LEGACY_BACKUP_HEADER_BYTES + payload.len());
    container.extend_from_slice(LEGACY_BACKUP_MAGIC);
    container.extend_from_slice(&LEGACY_BACKUP_VERSION.to_le_bytes());
    container.extend_from_slice(&flags.to_le_bytes());
    container.extend_from_slice(&(raw.len() as u64).to_le_bytes());
    container.extend_from_slice(&digest);
    container.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    container.extend_from_slice(&payload);
    Ok(container)
}

fn unprotect_legacy_backup(container: &[u8]) -> StoreResult<Vec<u8>> {
    if container.len() < LEGACY_BACKUP_HEADER_BYTES
        || &container[..LEGACY_BACKUP_MAGIC.len()] != LEGACY_BACKUP_MAGIC
    {
        return Err(StoreError::Integrity(
            "legacy backup magic is invalid".to_string(),
        ));
    }
    let version = u16::from_le_bytes(container[8..10].try_into().unwrap());
    let flags = u16::from_le_bytes(container[10..12].try_into().unwrap());
    let raw_len = u64::from_le_bytes(container[12..20].try_into().unwrap());
    let expected_sha256 = &container[20..52];
    let payload_len = u64::from_le_bytes(container[52..60].try_into().unwrap());
    if version != LEGACY_BACKUP_VERSION
        || payload_len > usize::MAX as u64
        || LEGACY_BACKUP_HEADER_BYTES.saturating_add(payload_len as usize) != container.len()
    {
        return Err(StoreError::Integrity(
            "legacy backup header is invalid".to_string(),
        ));
    }
    let payload = &container[LEGACY_BACKUP_HEADER_BYTES..];
    #[cfg(target_os = "windows")]
    let restored = if flags == LEGACY_BACKUP_FLAG_DPAPI_CURRENT_USER {
        dpapi_unprotect(payload).map_err(StoreError::Io)?
    } else {
        return Err(StoreError::Integrity(
            "legacy backup is not protected for the current Windows user".to_string(),
        ));
    };
    #[cfg(not(target_os = "windows"))]
    let restored = if flags == LEGACY_BACKUP_FLAG_RESTRICTED_PLAINTEXT {
        payload.to_vec()
    } else {
        return Err(StoreError::Integrity(
            "legacy backup protection mode is unsupported".to_string(),
        ));
    };
    if restored.len() as u64 != raw_len || Sha256::digest(&restored).as_slice() != expected_sha256 {
        return Err(StoreError::Integrity(
            "legacy backup length or SHA-256 verification failed".to_string(),
        ));
    }
    Ok(restored)
}

#[cfg(test)]
pub(crate) fn legacy_backup_test_plaintext(path: &Path) -> StoreResult<Vec<u8>> {
    unprotect_legacy_backup(&fs::read(path)?)
}

#[cfg(target_os = "windows")]
fn dpapi_protect(raw: &[u8]) -> io::Result<Vec<u8>> {
    if raw.len() > u32::MAX as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "legacy backup is too large for DPAPI",
        ));
    }
    let mut input = DataBlob {
        cb_data: raw.len() as u32,
        pb_data: raw.as_ptr() as *mut u8,
    };
    let mut output = DataBlob::default();
    let ok = unsafe {
        CryptProtectData(
            &mut input,
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0x1,
            &mut output,
        )
    };
    dpapi_output(ok, output)
}

#[cfg(target_os = "windows")]
fn dpapi_unprotect(raw: &[u8]) -> io::Result<Vec<u8>> {
    if raw.len() > u32::MAX as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "encrypted legacy backup is too large for DPAPI",
        ));
    }
    let mut input = DataBlob {
        cb_data: raw.len() as u32,
        pb_data: raw.as_ptr() as *mut u8,
    };
    let mut output = DataBlob::default();
    let ok = unsafe {
        CryptUnprotectData(
            &mut input,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0x1,
            &mut output,
        )
    };
    dpapi_output(ok, output)
}

#[cfg(target_os = "windows")]
fn dpapi_output(ok: i32, output: DataBlob) -> io::Result<Vec<u8>> {
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    if output.pb_data.is_null() && output.cb_data != 0 {
        return Err(io::Error::other("DPAPI returned an invalid output buffer"));
    }
    let bytes = if output.cb_data == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(output.pb_data, output.cb_data as usize) }.to_vec()
    };
    unsafe {
        let _ = LocalFree(output.pb_data as *mut std::ffi::c_void);
    }
    Ok(bytes)
}

#[cfg(target_os = "windows")]
#[repr(C)]
#[derive(Default)]
struct DataBlob {
    cb_data: u32,
    pb_data: *mut u8,
}

#[cfg(target_os = "windows")]
#[link(name = "crypt32")]
extern "system" {
    fn CryptProtectData(
        p_data_in: *mut DataBlob,
        sz_data_descr: *const u16,
        p_optional_entropy: *mut DataBlob,
        pv_reserved: *mut std::ffi::c_void,
        p_prompt_struct: *mut std::ffi::c_void,
        dw_flags: u32,
        p_data_out: *mut DataBlob,
    ) -> i32;
    fn CryptUnprotectData(
        p_data_in: *mut DataBlob,
        ppsz_data_descr: *mut *mut u16,
        p_optional_entropy: *mut DataBlob,
        pv_reserved: *mut std::ffi::c_void,
        p_prompt_struct: *mut std::ffi::c_void,
        dw_flags: u32,
        p_data_out: *mut DataBlob,
    ) -> i32;
}

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
extern "system" {
    fn LocalFree(h_mem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
}

fn absolute_path(path: &Path) -> StoreResult<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn unique_pre_schema_backup_path(
    database_path: &Path,
    from_version: i64,
    to_version: i64,
    timestamp: i64,
) -> StoreResult<PathBuf> {
    let parent = database_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = database_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("server_store");
    for sequence in 0..10_000_u32 {
        let suffix = if sequence == 0 {
            String::new()
        } else {
            format!("_{sequence}")
        };
        let candidate = parent.join(format!(
            "{stem}_pre_schema_v{from_version}_to_v{to_version}_{timestamp}{suffix}.sqlite3"
        ));
        if !candidate.exists()
            && !sqlite_sidecar_path(&candidate, "-wal").exists()
            && !sqlite_sidecar_path(&candidate, "-shm").exists()
        {
            return Ok(candidate);
        }
    }
    Err(StoreError::Integrity(
        "could not allocate a unique pre-schema-migration backup name".to_string(),
    ))
}

fn create_verified_sqlite_backup(
    source: &Connection,
    source_path: &Path,
    destination: &Path,
    now_epoch_millis: i64,
    expected_schema_version: Option<i64>,
    reusable_pre_schema_versions: Option<(i64, i64)>,
) -> StoreResult<VerifiedBackupReport> {
    if destination.exists()
        || sqlite_sidecar_path(destination, "-wal").exists()
        || sqlite_sidecar_path(destination, "-shm").exists()
        || sqlite_sidecar_path(destination, "-journal").exists()
    {
        return Err(StoreError::Integrity(format!(
            "backup destination or SQLite sidecar already exists: {}",
            destination.display()
        )));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    if absolute_path(source_path)? == absolute_path(destination)? {
        return Err(StoreError::Integrity(
            "backup destination cannot be the live database".to_string(),
        ));
    }
    verify_quick_check(source)?;
    verify_integrity_check(source)?;
    verify_foreign_keys(source)?;
    if let Some(version @ 15..=17) = expected_schema_version {
        verify_required_schema_at_version(source, version)?;
        verify_semantic_storage_integrity(source)?;
    }

    let temp_path = allocate_backup_temp_path(destination, now_epoch_millis)?;
    let mut owns_temp_path = false;
    let backup_result = (|| -> StoreResult<VerifiedBackupReport> {
        let reserved_temp = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        owns_temp_path = true;
        reserved_temp.sync_all()?;
        drop(reserved_temp);
        let mut target = Connection::open_with_flags(
            &temp_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
        )?;
        {
            let backup = Backup::new(source, &mut target)?;
            backup.run_to_completion(128, Duration::from_millis(5), None)?;
        }
        target.pragma_update(None, "journal_mode", "DELETE")?;
        target.pragma_update(None, "foreign_keys", "ON")?;
        target.pragma_update(None, "trusted_schema", "OFF")?;
        verify_quick_check(&target)?;
        verify_integrity_check(&target)?;
        verify_foreign_keys(&target)?;
        if let Some(expected) = expected_schema_version {
            let actual = current_schema_version(&target)?;
            if actual != expected {
                return Err(StoreError::Integrity(format!(
                    "backup schema version changed during copy: expected {expected}, found {actual}"
                )));
            }
            if matches!(expected, 15..=17) {
                verify_required_schema_at_version(&target, expected)?;
                verify_semantic_storage_integrity(&target)?;
            }
        }
        drop(target);
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(&temp_path)?
            .sync_all()?;
        let size_bytes = fs::metadata(&temp_path)?.len();
        if size_bytes == 0 {
            return Err(StoreError::Integrity(
                "SQLite backup is empty after copy".to_string(),
            ));
        }
        let sha256 = sha256_file(&temp_path)?;
        let server_instance_id = if matches!(expected_schema_version, Some(15..=17)) {
            let target = Connection::open_with_flags(
                &temp_path,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
            )?;
            target.query_row(
                "SELECT server_instance_id FROM server_identity WHERE singleton = 1",
                [],
                |row| row.get::<_, String>(0),
            )?
        } else {
            String::new()
        };
        if let Some((from_version, to_version)) = reusable_pre_schema_versions {
            if expected_schema_version != Some(from_version) {
                return Err(StoreError::Integrity(
                    "pre-schema backup reuse version does not match the copied schema".to_string(),
                ));
            }
            if let Some(report) = find_reusable_verified_pre_schema_backup(
                source_path,
                destination,
                from_version,
                to_version,
                size_bytes,
                &sha256,
                &server_instance_id,
            )? {
                remove_owned_backup_temp(&temp_path);
                if temp_path.exists()
                    || sqlite_sidecar_path(&temp_path, "-wal").exists()
                    || sqlite_sidecar_path(&temp_path, "-shm").exists()
                    || sqlite_sidecar_path(&temp_path, "-journal").exists()
                {
                    return Err(StoreError::Integrity(format!(
                        "could not remove verified duplicate backup temporary file: {}",
                        temp_path.display()
                    )));
                }
                owns_temp_path = false;
                return Ok(report);
            }
        }
        fs::rename(&temp_path, destination)?;
        Ok(VerifiedBackupReport {
            destination: destination.to_path_buf(),
            size_bytes,
            sha256,
            created_at_epoch_millis: now_epoch_millis,
            server_instance_id,
        })
    })();
    if backup_result.is_err() && owns_temp_path {
        remove_owned_backup_temp(&temp_path);
    }
    backup_result
}

fn verify_published_backup_identity(report: &VerifiedBackupReport) -> StoreResult<()> {
    if sqlite_sidecar_path(&report.destination, "-wal").exists()
        || sqlite_sidecar_path(&report.destination, "-shm").exists()
        || sqlite_sidecar_path(&report.destination, "-journal").exists()
    {
        return Err(StoreError::Integrity(format!(
            "verified pre-schema backup has an unexpected SQLite sidecar: {}",
            report.destination.display()
        )));
    }
    let size_bytes = fs::metadata(&report.destination)?.len();
    let sha256 = sha256_file(&report.destination)?;
    if size_bytes != report.size_bytes || sha256 != report.sha256 {
        return Err(StoreError::Integrity(format!(
            "verified pre-schema backup identity changed before migration: {}",
            report.destination.display()
        )));
    }
    Ok(())
}

fn controlled_backup_file_name(path: &Path) -> StoreResult<String> {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            StoreError::Integrity(format!(
                "verified pre-schema backup has no UTF-8 file name: {}",
                path.display()
            ))
        })?;
    let mut components = Path::new(file_name).components();
    let is_single_normal_component = matches!(
        components.next(),
        Some(std::path::Component::Normal(component)) if component == std::ffi::OsStr::new(file_name)
    ) && components.next().is_none();
    if file_name.is_empty()
        || file_name == "."
        || file_name == ".."
        || file_name.contains('/')
        || file_name.contains('\\')
        || file_name.len() > 1_024
        || !is_single_normal_component
    {
        return Err(StoreError::Integrity(
            "verified pre-schema backup file name is not a single ordinary file name".to_string(),
        ));
    }
    Ok(file_name.to_string())
}

fn read_verified_pre_schema_snapshot_manifest(
    report: &VerifiedBackupReport,
    expected_schema_version: i64,
) -> StoreResult<VerifiedPreSchemaSnapshotManifest> {
    if !matches!(expected_schema_version, 12 | 13) {
        return Err(StoreError::Integrity(format!(
            "snapshot repair manifest does not support backup schema v{expected_schema_version}"
        )));
    }
    verify_published_backup_identity(report)?;
    let backup = Connection::open_with_flags(
        &report.destination,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )?;
    backup.busy_timeout(DEFAULT_BUSY_TIMEOUT)?;
    backup.pragma_update(None, "foreign_keys", "ON")?;
    backup.pragma_update(None, "trusted_schema", "OFF")?;
    verify_quick_check(&backup)?;
    verify_integrity_check(&backup)?;
    verify_foreign_keys(&backup)?;
    let schema_version = current_schema_version(&backup)?;
    if schema_version != expected_schema_version {
        return Err(StoreError::Integrity(format!(
            "snapshot repair manifest requires a verified schema v{expected_schema_version} backup, found schema v{schema_version}"
        )));
    }
    let sources = legacy_snapshot_repair_manifest(&backup)?;
    drop(backup);
    verify_published_backup_identity(report)?;
    let size_bytes = i64::try_from(report.size_bytes)
        .map_err(|_| StoreError::Integrity("pre-schema backup size overflow".to_string()))?;
    if size_bytes <= 0 || report.created_at_epoch_millis < 0 {
        return Err(StoreError::Integrity(
            "verified pre-schema backup identity metadata is invalid".to_string(),
        ));
    }
    Ok(VerifiedPreSchemaSnapshotManifest {
        backup: LegacySnapshotRepairBackupIdentity {
            file_name: controlled_backup_file_name(&report.destination)?,
            size_bytes,
            sha256: report.sha256.clone(),
            schema_version,
            created_at_epoch_millis: report.created_at_epoch_millis,
        },
        sources,
    })
}

fn find_reusable_verified_pre_schema_backup(
    source_path: &Path,
    destination: &Path,
    from_version: i64,
    to_version: i64,
    copied_size_bytes: u64,
    copied_sha256: &str,
    copied_server_instance_id: &str,
) -> StoreResult<Option<VerifiedBackupReport>> {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let mut candidates = fs::read_dir(parent)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if !entry.file_type().ok()?.is_file() || path == destination {
                return None;
            }
            let created_at_epoch_millis = generated_pre_schema_backup_timestamp(
                source_path,
                &path,
                from_version,
                to_version,
            )?;
            Some((path, created_at_epoch_millis))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.0.cmp(&right.0));

    for (candidate, created_at_epoch_millis) in candidates {
        if sqlite_sidecar_path(&candidate, "-wal").exists()
            || sqlite_sidecar_path(&candidate, "-shm").exists()
            || sqlite_sidecar_path(&candidate, "-journal").exists()
            || absolute_path(&candidate)? == absolute_path(source_path)?
        {
            continue;
        }
        if fs::metadata(&candidate).map(|metadata| metadata.len()).ok() != Some(copied_size_bytes)
            || sha256_file(&candidate).ok().as_deref() != Some(copied_sha256)
        {
            continue;
        }

        let verified = (|| -> StoreResult<String> {
            let existing = Connection::open_with_flags(
                &candidate,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
            )?;
            existing.pragma_update(None, "foreign_keys", "ON")?;
            existing.pragma_update(None, "trusted_schema", "OFF")?;
            verify_quick_check(&existing)?;
            verify_integrity_check(&existing)?;
            verify_foreign_keys(&existing)?;
            let actual_schema_version = current_schema_version(&existing)?;
            if actual_schema_version != from_version {
                return Err(StoreError::Integrity(format!(
                    "reusable pre-schema backup has schema {actual_schema_version}, expected {from_version}"
                )));
            }
            if matches!(from_version, 15..=17) {
                verify_required_schema_at_version(&existing, from_version)?;
                verify_semantic_storage_integrity(&existing)?;
                existing
                    .query_row(
                        "SELECT server_instance_id FROM server_identity WHERE singleton = 1",
                        [],
                        |row| row.get::<_, String>(0),
                    )
                    .map_err(StoreError::from)
            } else {
                Ok(String::new())
            }
        })();
        let Ok(server_instance_id) = verified else {
            continue;
        };
        if server_instance_id != copied_server_instance_id
            || fs::metadata(&candidate).map(|metadata| metadata.len()).ok()
                != Some(copied_size_bytes)
            || sha256_file(&candidate).ok().as_deref() != Some(copied_sha256)
        {
            continue;
        }
        return Ok(Some(VerifiedBackupReport {
            destination: candidate,
            size_bytes: copied_size_bytes,
            sha256: copied_sha256.to_string(),
            created_at_epoch_millis,
            server_instance_id,
        }));
    }
    Ok(None)
}

fn generated_pre_schema_backup_timestamp(
    source_path: &Path,
    candidate: &Path,
    from_version: i64,
    to_version: i64,
) -> Option<i64> {
    let stem = source_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("server_store");
    let prefix = format!("{stem}_pre_schema_v{from_version}_to_v{to_version}_");
    let file_name = candidate.file_name()?.to_str()?;
    let generated_suffix = file_name.strip_prefix(&prefix)?.strip_suffix(".sqlite3")?;
    let mut parts = generated_suffix.split('_');
    let timestamp = parts.next()?.parse::<i64>().ok()?;
    match (parts.next(), parts.next()) {
        (None, None) => Some(timestamp),
        (Some(sequence), None) if sequence.parse::<u32>().ok().is_some_and(|value| value > 0) => {
            Some(timestamp)
        }
        _ => None,
    }
}

fn allocate_backup_temp_path(destination: &Path, timestamp: i64) -> StoreResult<PathBuf> {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let destination_key = sha256_hex(destination.as_os_str().to_string_lossy().as_bytes());
    let short_key = &destination_key[..16];
    for sequence in 0..10_000_u32 {
        let candidate = parent.join(format!(
            ".{short_key}.incomplete_{timestamp}_{sequence}.sqlite3"
        ));
        if !candidate.exists()
            && !sqlite_sidecar_path(&candidate, "-wal").exists()
            && !sqlite_sidecar_path(&candidate, "-shm").exists()
            && !sqlite_sidecar_path(&candidate, "-journal").exists()
        {
            return Ok(candidate);
        }
    }
    Err(StoreError::Integrity(
        "could not allocate a unique backup temporary path".to_string(),
    ))
}

fn remove_owned_backup_temp(temp_path: &Path) {
    let safe_name = temp_path
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.contains(".incomplete_"));
    if !safe_name {
        return;
    }
    let _ = fs::remove_file(temp_path);
    let _ = fs::remove_file(sqlite_sidecar_path(temp_path, "-wal"));
    let _ = fs::remove_file(sqlite_sidecar_path(temp_path, "-shm"));
    let _ = fs::remove_file(sqlite_sidecar_path(temp_path, "-journal"));
}

fn sqlite_sidecar_path(database_path: &Path, suffix: &str) -> PathBuf {
    let mut path = database_path.as_os_str().to_os_string();
    path.push(suffix);
    PathBuf::from(path)
}

fn sha256_file(path: &Path) -> StoreResult<String> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    let bytes = digest.finalize();
    let mut encoded = String::with_capacity(64);
    for byte in bytes {
        use fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    Ok(encoded)
}

fn system_time_epoch_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

#[cfg(test)]
mod tests {
    include!("server_media_restore_tests.rs");
    include!("server_privacy_audit_tests.rs");
    include!("server_privacy_journal_tests.rs");
    include!("server_storage_format_tests.rs");
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::SystemTime;

    #[test]
    fn deleted_historical_media_are_not_reported_missing_but_newer_media_are() {
        let mut data = json!({"notes":[{"id":"note-a","revisions":[{"attachments":[
            {"id":"deleted-image","updatedAtEpochMillis":10},
            {"id":"still-missing","updatedAtEpochMillis":10}
        ]}]}],"tombstones":[
            {"entityType":"noteMedia","entityId":"deleted-image","deletedAtEpochMillis":20}
        ]});
        assert_eq!(
            1,
            unresolved_media_metadata_count(&data.to_string()).unwrap()
        );
        data["notes"][0]["revisions"][0]["attachments"][0]["updatedAtEpochMillis"] = json!(21);
        assert_eq!(
            2,
            unresolved_media_metadata_count(&data.to_string()).unwrap()
        );
        data["notes"][0]["revisions"][0]["attachments"][0]["updatedAtEpochMillis"] = json!(20);
        assert_eq!(
            1,
            unresolved_media_metadata_count(&data.to_string()).unwrap()
        );
        data["tombstones"][0]["entityType"] = json!("note");
        assert_eq!(
            2,
            unresolved_media_metadata_count(&data.to_string()).unwrap()
        );
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let unique = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "gridtimer_server_store_test_{label}_{}_{nanos}_{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let safe_name = self
                .0
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("gridtimer_server_store_test_"));
            if safe_name && self.0.starts_with(std::env::temp_dir()) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn legacy_media_gap_preserves_snapshot_and_allows_timer_sync_and_replay() {
        let directory = TestDirectory::new("legacy-media-sync");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let mut current = json!({"notes":[{"id":"old-note","attachments":[{
            "id":"old-image", "sha256":"", "mimeType":"image/png",
            "sizeBytes":125, "updatedAtEpochMillis":10
        }]}], "slots":[{"id":1,"accumulatedMillis":0}]});
        let raw = current.to_string();
        seed_grandfathered_current_snapshot(&store, "user-a", 0, &raw, 100);
        current["slots"][0]["accumulatedMillis"] = json!(5000);
        let updated = current.to_string();
        assert_eq!(1, unresolved_media_metadata_count(&updated).unwrap());
        let applied = store
            .apply_sync_request(
                "user-a",
                "legacy-gap-sync",
                "{}",
                1,
                &updated,
                "{\"ok\":true}",
                200,
            )
            .unwrap();
        assert!(matches!(applied, SyncRequestOutcome::Applied(_)));
        assert_eq!(updated, store.read_account("user-a").unwrap().app_data_json);
        assert!(matches!(
            store
                .apply_sync_request(
                    "user-a",
                    "legacy-gap-sync",
                    "{}",
                    1,
                    &updated,
                    "{\"ok\":true}",
                    201
                )
                .unwrap(),
            SyncRequestOutcome::Replayed(_)
        ));
        let connection = store.open_connection(false).unwrap();
        let history = store
            .list_snapshot_history("user-a", 10)
            .unwrap()
            .into_iter()
            .find(|entry| entry.revision == 1)
            .unwrap();
        assert_eq!(raw, history.app_data_json);
        let incomplete: (i64, i64) = connection.query_row(
            "SELECT media_snapshot_complete, (SELECT COUNT(*) FROM account_snapshot_media_history
              WHERE user_id='user-a' AND account_revision=1 AND content_sha256 IS NULL
              AND length(missing_reason)>0) FROM account_snapshot_history
              WHERE user_id='user-a' AND revision=1", [], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!((0, 1), incomplete);
        assert_eq!(
            0,
            connection
                .query_row("SELECT COUNT(*) FROM note_media", [], |r| r
                    .get::<_, i64>(0))
                .unwrap()
        );
        assert!(store.restore_account_snapshot("user-a", 1, 2, 300).is_err());
        current["slots"][0]["accumulatedMillis"] = json!(6000);
        store
            .compare_and_swap_account("user-a", 2, &current.to_string(), 300)
            .unwrap();
    }

    #[test]
    fn legacy_media_manifest_is_account_scoped_and_restore_guarded() {
        let directory = TestDirectory::new("legacy-media-manifest");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        store
            .create_user(new_user("user-b", "b@example.test"))
            .unwrap();
        let token_a = store
            .issue_token("user-a", "token-a", "phone-a", 10, 10000)
            .unwrap();
        let token_b = store
            .issue_token("user-b", "token-b", "phone-b", 10, 10000)
            .unwrap();
        let legacy = json!({"notes":[{"id":"note-a","attachments":[{"id":"legacy-image","sha256":"","mimeType":"image/png","sizeBytes":125}]}]}).to_string();
        seed_grandfathered_current_snapshot(&store, "user-a", 0, &legacy, 100);
        assert!(matches!(
            store.media_manifest_for_generation("user-a", token_a.id, 0, "", true),
            Err(StoreError::RestoreReceiptRequired { .. })
        ));
        let (_, receipt_a) = store
            .restored_account_with_receipt("user-a", token_a.id)
            .unwrap();
        let (_, receipt_b) = store
            .restored_account_with_receipt("user-b", token_b.id)
            .unwrap();
        let (blobs, gaps) = store
            .media_manifest_for_generation("user-a", token_a.id, 0, &receipt_a.receipt, true)
            .unwrap();
        assert!(blobs.is_empty());
        assert_eq!(1, gaps.len());
        assert_eq!("legacy-image", gaps[0].attachment_id);
        assert_eq!("image/png", gaps[0].mime_type);
        assert_eq!(125, gaps[0].size_bytes);
        assert!(store
            .media_manifest_for_generation("user-b", token_b.id, 0, &receipt_b.receipt, true)
            .unwrap()
            .1
            .is_empty());
        assert!(store
            .media_manifest_for_generation("user-a", token_b.id, 0, "", true)
            .is_err());
        assert!(store
            .media_manifest_for_generation("user-a", token_a.id, 1, "", true)
            .is_err());
        assert!(
            legacy_unhashed_media_references(&json_with_attachment("new-complete", b"abc"))
                .unwrap()
                .is_empty()
        );
        let malformed = legacy.replace("image/png", "bad mime");
        assert!(legacy_unhashed_media_references(&malformed).map_or(true, |gaps| gaps.is_empty()));
        assert_eq!(legacy, store.read_account("user-a").unwrap().app_data_json);
    }

    #[test]
    fn legacy_media_gap_rejects_new_changed_weakened_and_other_account_references() {
        let directory = TestDirectory::new("legacy-media-guards");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        store
            .create_user(new_user("user-b", "b@example.test"))
            .unwrap();
        let legacy = json!({"notes":[{"id":"old-note","attachments":[{
            "id":"old-image", "sha256":"", "mimeType":"image/png", "sizeBytes":125
        }]}]});
        seed_grandfathered_current_snapshot(&store, "user-a", 0, &legacy.to_string(), 100);
        let before = store.read_account("user-a").unwrap();
        let mut changed_size = legacy.clone();
        changed_size["notes"][0]["attachments"][0]["sizeBytes"] = json!(126);
        let mut changed_type = legacy.clone();
        changed_type["notes"][0]["attachments"][0]["mimeType"] = json!("image/jpeg");
        let mut new_id = legacy.clone();
        new_id["notes"][0]["attachments"][0]["id"] = json!("new-image");
        let mut missing_size = legacy.clone();
        missing_size["notes"][0]["attachments"][0]
            .as_object_mut()
            .unwrap()
            .remove("sizeBytes");
        for (index, incoming) in [changed_size, changed_type, new_id, missing_size]
            .iter()
            .enumerate()
        {
            assert!(store
                .apply_sync_request(
                    "user-a",
                    &format!("bad-{index}"),
                    "{}",
                    1,
                    &incoming.to_string(),
                    "{}",
                    200
                )
                .is_err());
            assert_eq!(before, store.read_account("user-a").unwrap());
        }
        assert!(store
            .compare_and_swap_account("user-b", 0, &legacy.to_string(), 200)
            .is_err());
        let complete = json_with_attachment("old-image", b"verified");
        assert!(validate_app_data_media_transition(&complete, &legacy.to_string()).is_err());
        assert_eq!(
            0,
            store
                .open_connection(false)
                .unwrap()
                .query_row("SELECT COUNT(*) FROM request_dedup", [], |r| r
                    .get::<_, i64>(0))
                .unwrap()
        );
    }

    #[test]
    fn legacy_media_gap_allows_repair_but_rejects_conflicting_history_and_invalid_identifiers() {
        let legacy = json!({"notes":[{"id":"n","attachments":[{
            "id":"old-image", "sha256":"", "mimeType":"application/octet-stream", "sizeBytes":3
        }]}]})
        .to_string();
        let repaired = json_with_attachment("old-image", b"abc");
        validate_app_data_media_transition(&legacy, &repaired).unwrap();
        assert_eq!(0, unresolved_media_metadata_count(&repaired).unwrap());
        validate_app_data_media_transition(&legacy, "{\"notes\":[]}").unwrap();
        let mut conflicted: serde_json::Value = serde_json::from_str(&repaired).unwrap();
        let mut alternate = conflicted["notes"][0]["attachments"][0].clone();
        alternate["sha256"] = json!("1".repeat(64));
        conflicted["notes"][0]["revisions"] = json!([{"attachments":[alternate]}]);
        assert!(validate_app_data_media_transition(&legacy, &conflicted.to_string()).is_err());
        let invalid = legacy.replace("old-image", "../image");
        assert!(validate_app_data_media_transition(&invalid, &invalid).is_err());
        assert!(validate_app_data_media_transition("{}", &legacy).is_err());
    }

    fn new_user(id: &str, email: &str) -> NewStoredUser {
        NewStoredUser {
            id: id.to_string(),
            email: email.to_string(),
            password_salt: format!("salt-{id}"),
            password_hash: format!("hash-{id}"),
            password_scheme: "legacy_sha256".to_string(),
            created_at_epoch_millis: 10,
            updated_at_epoch_millis: 10,
            app_data_json: "{}".to_string(),
            account_revision: 0,
        }
    }

    fn seed_grandfathered_current_snapshot(
        store: &SqliteServerStore,
        user_id: &str,
        expected_revision: i64,
        app_data_json: &str,
        updated_at_epoch_millis: i64,
    ) {
        let mut connection = store.open_connection(false).unwrap();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let revision = next_account_revision(expected_revision).unwrap();
        let restore_generation = restore_generation_in_transaction(&transaction, user_id).unwrap();
        let content_sha256 = sha256_hex(app_data_json.as_bytes());
        let envelope_sha256 = account_snapshot_envelope_sha256(
            user_id,
            app_data_json,
            revision,
            updated_at_epoch_millis,
            restore_generation,
        );
        assert_eq!(
            1,
            transaction
                .execute(
                    "UPDATE account_snapshots
                     SET app_data_json = ?1, revision = ?2, updated_at_epoch_millis = ?3,
                         content_sha256 = ?4, envelope_sha256 = ?5
                     WHERE user_id = ?6 AND revision = ?7",
                    params![
                        app_data_json,
                        revision,
                        updated_at_epoch_millis,
                        content_sha256,
                        envelope_sha256,
                        user_id,
                        expected_revision,
                    ],
                )
                .unwrap()
        );
        replace_current_snapshot_media_identities_tolerant(&transaction, user_id, app_data_json)
            .unwrap();
        transaction.commit().unwrap();
    }

    fn json_with_attachment(id: &str, content: &[u8]) -> String {
        format!(
            "{{\"notes\":[{{\"id\":\"note-1\",\"attachments\":[{{\"id\":\"{id}\",\"sha256\":\"{}\",\"mimeType\":\"application/octet-stream\",\"sizeBytes\":{},\"updatedAtEpochMillis\":100}}]}}]}}",
            sha256_hex(content),
            content.len()
        )
    }

    fn json_with_attachment_id_count(count: usize) -> String {
        let sha256 = "0".repeat(64);
        let attachments = (0..count)
            .map(|index| {
                format!(
                    "{{\"id\":\"attachment-{index}\",\"sha256\":\"{sha256}\",\"mimeType\":\"application/octet-stream\",\"sizeBytes\":0,\"updatedAtEpochMillis\":100}}"
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!("{{\"notes\":[{{\"id\":\"note-1\",\"attachments\":[{attachments}]}}]}}")
    }

    fn json_with_attachment_id_count_and_padding(count: usize, padding_bytes: usize) -> String {
        let mut value =
            serde_json::from_str::<serde_json::Value>(&json_with_attachment_id_count(count))
                .unwrap();
        value["padding"] = serde_json::Value::String("x".repeat(padding_bytes));
        serde_json::to_string(&value).unwrap()
    }

    fn pending_registration_user(id: &str, email: &str, created_at: i64) -> NewStoredUser {
        NewStoredUser {
            id: id.to_string(),
            email: email.to_string(),
            password_salt: String::new(),
            password_hash: format!("argon-hash-{id}"),
            password_scheme: "argon2id_phc".to_string(),
            created_at_epoch_millis: created_at,
            updated_at_epoch_millis: created_at,
            app_data_json: String::new(),
            account_revision: 0,
        }
    }

    fn open_empty(directory: &TestDirectory) -> SqliteServerStore {
        SqliteServerStore::open(directory.0.join("server_store.sqlite3"), None).unwrap()
    }

    fn downgrade_to_schema_twelve_without_v13_triggers(connection: &Connection) {
        connection
            .execute_batch(
                "DROP TABLE IF EXISTS account_snapshot_media_identities;
                 DROP TABLE IF EXISTS legacy_snapshot_repair_allowances;
                 DROP INDEX IF EXISTS account_snapshot_media_history_identity_index;
                 DROP TRIGGER IF EXISTS note_media_metadata_insert_guard;
                 DROP TRIGGER IF EXISTS note_media_metadata_update_guard;
                 DROP TRIGGER IF EXISTS note_media_tombstone_insert_guard;
                 DROP TRIGGER IF EXISTS note_media_tombstone_update_guard;
                 DROP TRIGGER IF EXISTS media_history_metadata_insert_guard;
                 DROP TRIGGER IF EXISTS media_history_metadata_update_guard;
                 DROP TABLE IF EXISTS account_note_privacy;
                 DROP TABLE IF EXISTS note_privacy_commit_witnesses;
                 DELETE FROM schema_migrations WHERE version >= 13;
                 PRAGMA user_version = 12;",
            )
            .unwrap();
    }

    fn open_schema_twelve_snapshot_repair_fixture(
        label: &str,
        now_epoch_millis: i64,
    ) -> (
        TestDirectory,
        PathBuf,
        OpenedServerStore,
        AccountSnapshot,
        String,
    ) {
        let directory = TestDirectory::new(label);
        let database_path = directory.0.join("server_store.sqlite3");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let legacy_source = json_with_attachment_id_count_and_padding(2, 128);
        let source = store
            .compare_and_swap_account("user-a", 0, &legacy_source, 100)
            .unwrap();
        let connection = store.open_connection(false).unwrap();
        downgrade_to_schema_twelve_without_v13_triggers(&connection);
        drop(connection);
        drop(store);
        let opened = SqliteServerStore::open_with_options(ServerStoreOpenOptions {
            database_path: database_path.clone(),
            legacy_json_path: None,
            now_epoch_millis,
            legacy_token_ttl_millis: DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
        })
        .unwrap();
        let replacement = json_with_attachment_id_count_and_padding(1, 0);
        assert!(replacement.len() < source.app_data_json.len());
        (directory, database_path, opened, source, replacement)
    }

    #[test]
    fn connection_durability_pragmas_are_read_back_and_wal_fallback_fails_closed() {
        let directory = TestDirectory::new("connection_pragmas");
        let store = open_empty(&directory);
        let connection = store.open_connection(false).unwrap();
        verify_connection_durability_settings(&connection).unwrap();
        assert_eq!(
            1,
            connection
                .pragma_query_value(None, "foreign_keys", |row| row.get::<_, i64>(0))
                .unwrap()
        );
        assert_eq!(
            "wal",
            connection
                .pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))
                .unwrap()
                .to_ascii_lowercase()
        );
        assert_eq!(
            2,
            connection
                .pragma_query_value(None, "synchronous", |row| row.get::<_, i64>(0))
                .unwrap()
        );

        let memory = Connection::open_in_memory().unwrap();
        assert!(matches!(
            configure_connection(&memory),
            Err(StoreError::Integrity(message))
                if message.contains("journal_mode WAL was not applied")
        ));
    }

    fn schema_eleven_version_only_history_fixture(
        directory: &TestDirectory,
        keep_live_media: bool,
    ) -> (PathBuf, String, Vec<u8>, String) {
        let database_path = directory.0.join("server_store.sqlite3");
        let store = open_empty(directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let content = b"schema-eleven-version-only-media".to_vec();
        let sha256 = sha256_hex(&content);
        let historical_json = json!({
            "notes": [{
                "id": "note-a",
                "attachments": [],
                "revisions": [],
                "versions": [{
                    "id": "version-a",
                    "attachments": [{
                        "id": "version-attachment-a",
                        "sha256": sha256.clone(),
                        "mimeType": "image/png",
                        "sizeBytes": content.len(),
                        "updatedAtEpochMillis": 100
                    }]
                }]
            }]
        })
        .to_string();
        store
            .compare_and_swap_account("user-a", 0, &historical_json, 100)
            .unwrap();
        if keep_live_media {
            store
                .upsert_media(
                    "user-a",
                    "version-attachment-a",
                    &sha256,
                    "image/png",
                    content.len() as i64,
                    &content,
                    101,
                )
                .unwrap();
        }
        store
            .compare_and_swap_account("user-a", 1, "{\"notes\":[]}", 102)
            .unwrap();

        let connection = store.open_connection(false).unwrap();
        assert_eq!(
            1,
            connection
                .execute(
                    "DELETE FROM account_snapshot_media_history
                     WHERE user_id = 'user-a' AND account_revision = 1
                       AND attachment_id = 'version-attachment-a'",
                    [],
                )
                .unwrap()
        );
        assert_eq!(
            1,
            connection
                .execute(
                    "UPDATE account_snapshot_history
                     SET media_snapshot_complete = 1
                     WHERE user_id = 'user-a' AND revision = 1",
                    [],
                )
                .unwrap()
        );
        connection
            .execute_batch(
                "DROP TABLE IF EXISTS account_note_privacy; DROP TABLE IF EXISTS note_privacy_commit_witnesses;
                 DELETE FROM schema_migrations WHERE version >= 12;
                 PRAGMA user_version = 11;",
            )
            .unwrap();
        assert_eq!(11, current_schema_version(&connection).unwrap());
        drop(connection);
        drop(store);
        (database_path, historical_json, content, sha256)
    }

    fn legacy_store() -> LegacyServerStore {
        LegacyServerStore {
            users: (0..4)
                .map(|user_index| LegacyServerUser {
                    id: format!("user-{user_index}"),
                    email: format!("user{user_index}@example.test"),
                    password_salt: format!("salt-{user_index}"),
                    password_hash: format!("password-hash-{user_index}"),
                    created_at_epoch_millis: 1_000 + user_index,
                    updated_at_epoch_millis: 2_000 + user_index,
                    app_data_json: format!("{{\"owner\":{user_index}}}"),
                    tokens: (0..=user_index)
                        .map(|token_index| LegacyServerToken {
                            token: format!("raw-secret-{user_index}-{token_index}"),
                            device_name: format!("device-{token_index}"),
                            created_at_epoch_millis: 40_000 + token_index,
                            last_seen_at_epoch_millis: 50_000 + token_index,
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    fn merge_json_objects(existing: &str, incoming: &str, _: i64) -> StoreResult<String> {
        let mut existing =
            serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(existing)?;
        let incoming =
            serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(incoming)?;
        existing.extend(incoming);
        Ok(serde_json::Value::Object(existing).to_string())
    }

    #[test]
    fn create_user_rejects_an_exhausted_initial_account_revision() {
        let directory = TestDirectory::new("new_account_revision_exhausted");
        let store = open_empty(&directory);
        let mut user = new_user("user-a", "a@example.test");
        user.account_revision = i64::MAX;

        assert!(matches!(
            store.create_user(user),
            Err(StoreError::Integrity(message))
                if message.contains("exhausted account revision")
        ));
        assert!(store.find_user_by_id("user-a").unwrap().is_none());
    }

    #[test]
    fn exhausted_account_revision_rejects_mutations_without_changing_the_snapshot() {
        let directory = TestDirectory::new("account_revision_exhausted");
        let store = open_empty(&directory);
        let mut user = new_user("user-a", "a@example.test");
        user.app_data_json = "{\"base\":true}".to_string();
        store.create_user(user).unwrap();
        let exhausted_envelope =
            account_snapshot_envelope_sha256("user-a", "{\"base\":true}", i64::MAX, 10, 0);
        let connection = store.open_connection(false).unwrap();
        assert_eq!(
            1,
            connection
                .execute(
                    "UPDATE account_snapshots
                     SET revision = ?1, envelope_sha256 = ?2
                     WHERE user_id = 'user-a' AND revision = 0",
                    params![i64::MAX, exhausted_envelope],
                )
                .unwrap()
        );
        drop(connection);
        let before = store.read_account("user-a").unwrap();

        for result in [
            store.compare_and_swap_account("user-a", i64::MAX, "{\"cas\":true}", 100),
            store
                .apply_sync_request_with_limits(
                    "user-a",
                    "request-exhausted",
                    "{\"operation\":\"sync\"}",
                    i64::MAX,
                    "{\"sync\":true}",
                    "{\"ok\":true}",
                    101,
                    REQUEST_DEDUP_MAX_PER_USER,
                    REQUEST_DEDUP_MAX_TOTAL_BYTES_PER_USER,
                )
                .map(|outcome| match outcome {
                    SyncRequestOutcome::Applied(receipt)
                    | SyncRequestOutcome::Replayed(receipt) => AccountSnapshot {
                        user_id: "user-a".to_string(),
                        app_data_json: receipt.response_json,
                        revision: receipt.account_revision,
                        updated_at_epoch_millis: 101,
                    },
                }),
        ] {
            assert!(matches!(
                result,
                Err(StoreError::Integrity(message)) if message.contains("revision is exhausted")
            ));
            assert_eq!(before, store.read_account("user-a").unwrap());
        }

        let imported = store.import_account_snapshot_with_merge(
            "user-a",
            "{\"imported\":true}",
            102,
            102,
            merge_json_objects,
        );
        assert!(matches!(
            imported,
            Err(StoreError::Integrity(message)) if message.contains("revision is exhausted")
        ));
        assert_eq!(before, store.read_account("user-a").unwrap());
        assert!(matches!(
            next_account_revision(i64::MAX),
            Err(StoreError::Integrity(message)) if message.contains("revision is exhausted")
        ));
    }

    #[test]
    fn backup_temp_paths_are_short_destination_specific_and_clean_all_sidecars() {
        let directory = TestDirectory::new("backup_temp_path");
        let first_destination = directory
            .0
            .join(format!("{}-first.sqlite3", "long-backup-name-".repeat(8)));
        let second_destination = directory
            .0
            .join(format!("{}-second.sqlite3", "long-backup-name-".repeat(8)));
        let first_temp = allocate_backup_temp_path(&first_destination, 500).unwrap();
        let second_temp = allocate_backup_temp_path(&second_destination, 500).unwrap();

        assert_ne!(first_temp, second_temp);
        let first_name = first_temp.file_name().unwrap().to_string_lossy();
        assert!(first_name.starts_with('.'));
        assert!(first_name.contains(".incomplete_500_0.sqlite3"));
        assert!(
            first_name.len() <= 64,
            "temporary backup basename is unexpectedly long: {first_name}"
        );

        fs::write(sqlite_sidecar_path(&first_temp, "-journal"), b"occupied").unwrap();
        let collision_avoiding = allocate_backup_temp_path(&first_destination, 500).unwrap();
        assert_ne!(first_temp, collision_avoiding);
        assert!(collision_avoiding
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains(".incomplete_500_1.sqlite3"));

        fs::write(&collision_avoiding, b"temporary").unwrap();
        for suffix in ["-wal", "-shm", "-journal"] {
            fs::write(sqlite_sidecar_path(&collision_avoiding, suffix), b"sidecar").unwrap();
        }
        remove_owned_backup_temp(&collision_avoiding);
        assert!(!collision_avoiding.exists());
        for suffix in ["-wal", "-shm", "-journal"] {
            assert!(!sqlite_sidecar_path(&collision_avoiding, suffix).exists());
        }
        fs::remove_file(sqlite_sidecar_path(&first_temp, "-journal")).unwrap();
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn verified_backup_succeeds_when_legacy_temp_journal_would_reach_max_path() {
        use std::os::windows::ffi::OsStrExt;

        fn windows_path_len(path: &Path) -> usize {
            path.as_os_str().encode_wide().count()
        }

        let directory = TestDirectory::new("long_backup_boundary");
        let store = open_empty(&directory);
        store
            .create_user(new_user("long-path-user", "long-path@example.test"))
            .unwrap();
        let backup_directory = directory.0.join("backups");
        fs::create_dir_all(&backup_directory).unwrap();

        let legacy_suffix = ".incomplete_500_0.sqlite3";
        let minimum_file_name_len = 260_usize.saturating_sub(
            windows_path_len(&backup_directory) + 1 + 1 + legacy_suffix.len() + "-journal".len(),
        );
        let file_name_len = minimum_file_name_len.max(96).min(220);
        let backup_name = format!("{}.sqlite3", "b".repeat(file_name_len - ".sqlite3".len()));
        let backup_path = backup_directory.join(&backup_name);
        let legacy_temp = backup_directory.join(format!(".{backup_name}{legacy_suffix}"));
        let legacy_journal = sqlite_sidecar_path(&legacy_temp, "-journal");
        assert!(
            windows_path_len(&legacy_journal) >= 260,
            "regression fixture did not reach MAX_PATH: {}",
            windows_path_len(&legacy_journal)
        );
        assert!(
            windows_path_len(&backup_path) < 260,
            "final backup fixture must remain below MAX_PATH"
        );

        let report = store.create_verified_backup(&backup_path, 500).unwrap();
        assert_eq!(backup_path, report.destination);
        assert!(backup_path.exists());
        assert_eq!(report.sha256, sha256_file(&backup_path).unwrap());
        let leftovers = fs::read_dir(&backup_directory)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".incomplete_"))
            .count();
        assert_eq!(0, leftovers);
    }

    #[test]
    fn migrates_four_users_and_all_tokens_once_with_exact_backup() {
        let directory = TestDirectory::new("migration");
        let legacy_path = directory.0.join("server_store.json");
        let legacy_bytes = serde_json::to_vec_pretty(&legacy_store()).unwrap();
        fs::write(&legacy_path, &legacy_bytes).unwrap();
        let database_path = directory.0.join("server_store.sqlite3");
        let opened = SqliteServerStore::open_with_options(ServerStoreOpenOptions {
            database_path: database_path.clone(),
            legacy_json_path: Some(legacy_path.clone()),
            now_epoch_millis: 50_000,
            legacy_token_ttl_millis: 10_000,
        })
        .unwrap();
        let report = opened.legacy_migration.as_ref().unwrap();
        assert_eq!(4, report.users_imported);
        assert_eq!(10, report.tokens_imported);
        assert_eq!(
            Some("gtlbak"),
            report.backup_path.extension().and_then(|v| v.to_str())
        );
        let encrypted_backup = fs::read(&report.backup_path).unwrap();
        assert_ne!(legacy_bytes, encrypted_backup);
        #[cfg(target_os = "windows")]
        assert!(!encrypted_backup
            .windows(b"raw-secret-0-0".len())
            .any(|window| window == b"raw-secret-0-0"));
        assert_eq!(
            legacy_bytes,
            unprotect_legacy_backup(&encrypted_backup).unwrap()
        );
        assert_eq!(
            StoreStats {
                users: 4,
                tokens: 10,
                snapshots: 4
            },
            opened.store.stats().unwrap()
        );
        assert_eq!(
            TokenAuthentication::Active(AuthenticatedToken {
                token_id: 1,
                token_identifier: token_identifier("raw-secret-0-0"),
                user_id: "user-0".to_string(),
                device_name: "device-0".to_string(),
                expires_at_epoch_millis: 60_000,
                last_seen_restore_generation: 0,
            }),
            opened
                .store
                .authenticate_token("raw-secret-0-0", 55_000)
                .unwrap()
        );

        let token_rows = opened.store.list_token_metadata("user-3").unwrap();
        assert_eq!(4, token_rows.len());
        assert!(token_rows
            .iter()
            .all(|token| token.token_hash.len() == 64 && !token.token_hash.contains("raw-secret")));
        let exported: LegacyServerStore =
            serde_json::from_str(&opened.store.export_compatible_legacy_json().unwrap()).unwrap();
        assert_eq!(4, exported.users.len());
        assert!(exported.users.iter().all(|user| user.tokens.is_empty()));

        drop(opened);
        let reopened = SqliteServerStore::open_with_options(ServerStoreOpenOptions {
            database_path,
            legacy_json_path: Some(legacy_path),
            now_epoch_millis: 60_000,
            legacy_token_ttl_millis: 10_000,
        })
        .unwrap();
        assert!(reopened.legacy_migration.is_none());
        assert_eq!(4, reopened.store.stats().unwrap().users);
        let backup_count = fs::read_dir(&directory.0)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains("_pre_sqlite_migration_")
            })
            .count();
        assert_eq!(1, backup_count);
    }

    #[test]
    fn legacy_token_expiry_respects_last_use_instead_of_extending_every_token() {
        let directory = TestDirectory::new("legacy_token_expiry");
        let legacy_path = directory.0.join("server_store.json");
        let legacy = LegacyServerStore {
            users: vec![LegacyServerUser {
                id: "user-a".to_string(),
                email: "a@example.test".to_string(),
                password_salt: "salt".to_string(),
                password_hash: "hash".to_string(),
                created_at_epoch_millis: 1,
                updated_at_epoch_millis: 1,
                app_data_json: "{}".to_string(),
                tokens: vec![
                    LegacyServerToken {
                        token: "old-token".to_string(),
                        device_name: "old-device".to_string(),
                        created_at_epoch_millis: 1,
                        last_seen_at_epoch_millis: 10,
                    },
                    LegacyServerToken {
                        token: "recent-token".to_string(),
                        device_name: "recent-device".to_string(),
                        created_at_epoch_millis: 900,
                        last_seen_at_epoch_millis: 950,
                    },
                ],
            }],
        };
        fs::write(&legacy_path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let opened = SqliteServerStore::open_with_options(ServerStoreOpenOptions {
            database_path: directory.0.join("server_store.sqlite3"),
            legacy_json_path: Some(legacy_path),
            now_epoch_millis: 1_000,
            legacy_token_ttl_millis: 100,
        })
        .unwrap();

        assert_eq!(
            TokenAuthentication::Expired,
            opened.store.authenticate_token("old-token", 1_000).unwrap()
        );
        assert!(matches!(
            opened
                .store
                .authenticate_token("recent-token", 1_000)
                .unwrap(),
            TokenAuthentication::Active(AuthenticatedToken {
                expires_at_epoch_millis: 1_050,
                ..
            })
        ));
        assert_eq!(
            TokenAuthentication::Expired,
            opened
                .store
                .authenticate_token("recent-token", 1_050)
                .unwrap()
        );
    }

    #[test]
    fn multi_user_insert_rolls_back_on_constraint_failure() {
        let directory = TestDirectory::new("rollback");
        let store = open_empty(&directory);
        let users = vec![
            new_user("user-a", "same@example.test"),
            new_user("user-b", "SAME@example.test"),
        ];
        assert!(store.create_users_atomically(&users).is_err());
        assert_eq!(StoreStats::default(), store.stats().unwrap());
        store.validate_integrity().unwrap();
    }

    #[test]
    fn expired_and_revoked_tokens_are_rejected() {
        let directory = TestDirectory::new("token_status");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let token = store
            .issue_token("user-a", "expires", "phone", 100, 200)
            .unwrap();
        assert_eq!(token_identifier("expires"), token.token_id);
        assert_eq!(
            Some(decode_sha256_hex(&token_fingerprint("expires")).unwrap()),
            store
                .active_token_key_by_id("user-a", &token.token_id, 199)
                .unwrap()
        );
        assert!(matches!(
            store.authenticate_token("expires", 199).unwrap(),
            TokenAuthentication::Active(_)
        ));
        assert_eq!(
            TokenAuthentication::Expired,
            store.authenticate_token("expires", 200).unwrap()
        );
        store
            .issue_token("user-a", "revoked", "tablet", 100, 300)
            .unwrap();
        assert!(store.revoke_token("revoked", 150).unwrap());
        assert_eq!(
            TokenAuthentication::Revoked,
            store.authenticate_token("revoked", 151).unwrap()
        );
        assert_eq!(
            TokenAuthentication::Unknown,
            store.authenticate_token("unknown", 151).unwrap()
        );
        store
            .update_password_hash(
                "user-a",
                "",
                "$argon2id$v=19$m=65536,t=3,p=1$example$hash",
                "argon2id_phc",
                250,
            )
            .unwrap();
        assert_eq!(
            "argon2id_phc",
            store
                .find_user_by_id("user-a")
                .unwrap()
                .unwrap()
                .password_scheme
        );
    }

    #[test]
    fn discovery_token_key_accepts_only_live_unrevoked_pending_or_active_tokens() {
        let directory = TestDirectory::new("discovery_token_key");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();

        let pending = store
            .issue_pending_token("user-a", "pending-token", "phone", 100, 700)
            .unwrap();
        let pending_key = decode_sha256_hex(&token_fingerprint("pending-token")).unwrap();
        assert_eq!(
            Some(pending_key),
            store
                .discovery_token_key_by_id("user-a", &pending.token_id, 200)
                .unwrap()
        );
        assert!(store
            .discovery_token_key_by_id("wrong-user", &pending.token_id, 200)
            .unwrap()
            .is_none());
        assert!(matches!(
            store.authenticate_token("pending-token", 200).unwrap(),
            TokenAuthentication::PendingActivation(_)
        ));
        assert!(store
            .discovery_token_key_by_id("user-a", &pending.token_id, 700)
            .unwrap()
            .is_none());

        let revoked = store
            .issue_pending_token("user-a", "revoked-pending", "phone", 100, 700)
            .unwrap();
        assert!(store.revoke_token("revoked-pending", 150).unwrap());
        assert!(store
            .discovery_token_key_by_id("user-a", &revoked.token_id, 200)
            .unwrap()
            .is_none());

        let active = store
            .issue_token("user-a", "active-token", "tablet", 100, 700)
            .unwrap();
        assert!(store
            .discovery_token_key_by_id("user-a", &active.token_id, 200)
            .unwrap()
            .is_some());
    }

    #[test]
    fn pending_token_is_leased_then_activated_exactly_once() {
        let directory = TestDirectory::new("pending_token_activation");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let metadata = store
            .issue_pending_token("user-a", "pending-token", "phone", 100, 700)
            .unwrap();
        assert!(matches!(
            store.authenticate_token("pending-token", 200).unwrap(),
            TokenAuthentication::PendingActivation(_)
        ));
        assert!(store
            .active_token_key_by_id("user-a", &metadata.token_id, 200)
            .unwrap()
            .is_none());

        let activated = store
            .activate_pending_token("pending-token", 200, 2_000)
            .unwrap();
        assert!(matches!(
            activated,
            TokenAuthentication::Active(AuthenticatedToken {
                expires_at_epoch_millis: 2_000,
                ..
            })
        ));
        let replayed = store
            .activate_pending_token("pending-token", 300, 9_000)
            .unwrap();
        assert!(matches!(
            replayed,
            TokenAuthentication::Active(AuthenticatedToken {
                expires_at_epoch_millis: 2_000,
                ..
            })
        ));
        assert!(store
            .active_token_key_by_id("user-a", &metadata.token_id, 300)
            .unwrap()
            .is_some());
    }

    #[test]
    fn pending_token_activation_is_atomic_with_sync_commit_and_replay_safe() {
        let directory = TestDirectory::new("pending_token_sync_atomic");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let token = store
            .issue_pending_token("user-a", "atomic-sync-token", "phone", 100, 700)
            .unwrap();
        let receipt = store.ensure_restore_receipt("user-a", token.id).unwrap();
        let operation = r#"{"operation":"sync","tokenId":"atomic"}"#;
        let response = r#"{"ok":true}"#;
        let activation = PendingTokenActivation {
            token_id: token.id,
            activated_at_epoch_millis: 200,
            active_expires_at_epoch_millis: 2_000,
            cleanup_recovery_original_expiry_epoch_millis: None,
            cleanup_recovery_original_lease_millis: 0,
            cleanup_recovery_window_millis: 0,
        };

        assert!(matches!(
            store.apply_sync_request_for_generation_with_pending_activation(
                "user-a",
                token.id,
                0,
                &receipt.receipt,
                "atomic-sync-request",
                operation,
                99,
                r#"{"value":1}"#,
                response,
                200,
                Some(activation),
            ),
            Err(StoreError::RevisionConflict { .. })
        ));
        assert!(matches!(
            store.authenticate_token("atomic-sync-token", 201).unwrap(),
            TokenAuthentication::PendingActivation(_)
        ));
        assert!(
            !store
                .restore_barrier_state("user-a", token.id)
                .unwrap()
                .token_restore_acknowledged
        );

        assert!(matches!(
            store
                .apply_sync_request_for_generation_with_pending_activation(
                    "user-a",
                    token.id,
                    0,
                    &receipt.receipt,
                    "atomic-sync-request",
                    operation,
                    0,
                    r#"{"value":1}"#,
                    response,
                    200,
                    Some(activation),
                )
                .unwrap(),
            SyncRequestOutcome::Applied(_)
        ));
        assert!(matches!(
            store.authenticate_token("atomic-sync-token", 201).unwrap(),
            TokenAuthentication::Active(AuthenticatedToken {
                expires_at_epoch_millis: 2_000,
                ..
            })
        ));

        let replay_activation = PendingTokenActivation {
            token_id: token.id,
            activated_at_epoch_millis: 300,
            active_expires_at_epoch_millis: 9_000,
            cleanup_recovery_original_expiry_epoch_millis: None,
            cleanup_recovery_original_lease_millis: 0,
            cleanup_recovery_window_millis: 0,
        };
        assert!(matches!(
            store
                .apply_sync_request_for_generation_with_pending_activation(
                    "user-a",
                    token.id,
                    0,
                    &receipt.receipt,
                    "atomic-sync-request",
                    operation,
                    0,
                    r#"{"value":1}"#,
                    response,
                    300,
                    Some(replay_activation),
                )
                .unwrap(),
            SyncRequestOutcome::Replayed(_)
        ));
        assert!(matches!(
            store.authenticate_token("atomic-sync-token", 301).unwrap(),
            TokenAuthentication::Active(AuthenticatedToken {
                expires_at_epoch_millis: 2_000,
                ..
            })
        ));
    }

    #[test]
    fn pending_activation_trigger_failure_rolls_back_entire_sync_transaction() {
        let directory = TestDirectory::new("pending_activation_trigger_rollback");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let token = store
            .issue_pending_token("user-a", "trigger-token", "phone", 100, 700)
            .unwrap();
        let receipt = store.ensure_restore_receipt("user-a", token.id).unwrap();
        let before = store.read_account("user-a").unwrap();
        let connection = store.open_connection(false).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER fail_pending_activation
                 BEFORE UPDATE OF activation_state ON tokens
                 WHEN OLD.activation_state = 0 AND NEW.activation_state = 1
                 BEGIN
                     SELECT RAISE(ABORT, 'injected pending activation failure');
                 END;",
            )
            .unwrap();
        drop(connection);

        let outcome = store.apply_sync_request_for_generation_with_pending_activation(
            "user-a",
            token.id,
            0,
            &receipt.receipt,
            "trigger-rollback-request",
            r#"{"operation":"sync","tokenId":"trigger"}"#,
            0,
            r#"{"value":1}"#,
            r#"{"ok":true}"#,
            200,
            Some(PendingTokenActivation {
                token_id: token.id,
                activated_at_epoch_millis: 200,
                active_expires_at_epoch_millis: 2_000,
                cleanup_recovery_original_expiry_epoch_millis: None,
                cleanup_recovery_original_lease_millis: 0,
                cleanup_recovery_window_millis: 0,
            }),
        );
        assert!(outcome.is_err());
        assert_eq!(before, store.read_account("user-a").unwrap());
        assert!(
            !store
                .restore_barrier_state("user-a", token.id)
                .unwrap()
                .token_restore_acknowledged
        );
        assert!(matches!(
            store.authenticate_token("trigger-token", 201).unwrap(),
            TokenAuthentication::PendingActivation(_)
        ));
        let connection = store.open_connection(false).unwrap();
        assert_eq!(
            0,
            connection
                .query_row(
                    "SELECT COUNT(*) FROM request_dedup WHERE request_id = ?1",
                    params!["trigger-rollback-request"],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
        );
        connection
            .execute_batch("DROP TRIGGER fail_pending_activation;")
            .unwrap();
    }

    #[test]
    fn expired_pending_token_cannot_be_activated_and_logout_is_idempotent() {
        let directory = TestDirectory::new("pending_token_expiry_logout");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        store
            .issue_pending_token("user-a", "expired-pending", "phone", 100, 200)
            .unwrap();
        assert_eq!(
            TokenAuthentication::Expired,
            store.authenticate_token("expired-pending", 200).unwrap()
        );
        assert_eq!(
            TokenAuthentication::Expired,
            store
                .activate_pending_token("expired-pending", 200, 2_000)
                .unwrap()
        );
        assert_eq!(1, store.cleanup_expired_pending_tokens(250).unwrap());
        assert_eq!(0, store.cleanup_expired_pending_tokens(250).unwrap());
        assert_eq!(
            TokenAuthentication::Revoked,
            store.authenticate_token("expired-pending", 251).unwrap()
        );
        assert_eq!(
            Some("user-a".to_string()),
            store
                .revoke_token_for_logout("expired-pending", 300)
                .unwrap()
        );
        assert_eq!(
            Some("user-a".to_string()),
            store
                .revoke_token_for_logout("expired-pending", 400)
                .unwrap()
        );
        assert_eq!(
            TokenAuthentication::Revoked,
            store.authenticate_token("expired-pending", 401).unwrap()
        );
        let connection = store.open_connection(false).unwrap();
        assert_eq!(
            Some(300),
            connection
                .query_row(
                    "SELECT revoked_at_epoch_millis FROM tokens WHERE token_hash = ?1",
                    params![token_fingerprint("expired-pending")],
                    |row| row.get::<_, Option<i64>>(0),
                )
                .unwrap()
        );
        drop(connection);
        assert_eq!(
            1,
            store
                .cleanup_expired_pending_tokens(200 + PENDING_TOKEN_TOMBSTONE_RETENTION_MILLIS + 1)
                .unwrap()
        );
        assert_eq!(
            TokenAuthentication::Unknown,
            store.authenticate_token("expired-pending", 1_000).unwrap()
        );
    }

    #[test]
    fn pending_token_activation_is_concurrent_and_crash_atomic() {
        let directory = TestDirectory::new("pending_token_activation_atomic");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        store
            .issue_pending_token("user-a", "atomic-pending", "phone", 100, 900)
            .unwrap();

        let connection = store.open_connection(false).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER fail_pending_activation
                 BEFORE UPDATE OF activation_state ON tokens
                 WHEN OLD.activation_state = 0 AND NEW.activation_state = 1
                 BEGIN
                     SELECT RAISE(ABORT, 'simulated activation commit failure');
                 END;",
            )
            .unwrap();
        assert!(store
            .activate_pending_token("atomic-pending", 200, 2_000)
            .is_err());
        assert!(matches!(
            store.authenticate_token("atomic-pending", 201).unwrap(),
            TokenAuthentication::PendingActivation(_)
        ));
        connection
            .execute_batch("DROP TRIGGER fail_pending_activation;")
            .unwrap();
        drop(connection);

        let barrier = Arc::new(Barrier::new(3));
        let mut workers = Vec::new();
        for (now, expiry) in [(300, 3_000), (301, 4_000)] {
            let store = store.clone();
            let barrier = Arc::clone(&barrier);
            workers.push(thread::spawn(move || {
                barrier.wait();
                store
                    .activate_pending_token("atomic-pending", now, expiry)
                    .unwrap()
            }));
        }
        barrier.wait();
        let results = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        let expiries = results
            .iter()
            .map(|result| match result {
                TokenAuthentication::Active(token) => token.expires_at_epoch_millis,
                other => panic!("unexpected activation result: {other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(expiries[0], expiries[1]);
        assert!([3_000, 4_000].contains(&expiries[0]));
    }

    #[test]
    fn schema_ten_active_tokens_migrate_without_becoming_pending() {
        let directory = TestDirectory::new("token_activation_schema_migration");
        let database_path = directory.0.join("server_store.sqlite3");
        {
            let store = open_empty(&directory);
            store
                .create_user(new_user("user-a", "a@example.test"))
                .unwrap();
            store
                .issue_token(
                    "user-a",
                    "pre-activation-schema",
                    "phone",
                    100,
                    i64::MAX / 2,
                )
                .unwrap();
            let connection = store.open_connection(false).unwrap();
            connection
                .execute_batch(
                    "DROP INDEX tokens_pending_activation_index;
                     ALTER TABLE tokens DROP COLUMN activated_at_epoch_millis;
                     ALTER TABLE tokens DROP COLUMN activation_state;
                     DROP TABLE IF EXISTS account_note_privacy; DROP TABLE IF EXISTS note_privacy_commit_witnesses;
                 DELETE FROM schema_migrations WHERE version >= 11;
                     PRAGMA user_version = 10;",
                )
                .unwrap();
        }
        let reopened = SqliteServerStore::open(&database_path, None).unwrap();
        assert!(matches!(
            reopened
                .authenticate_token("pre-activation-schema", 200)
                .unwrap(),
            TokenAuthentication::Active(_)
        ));
        reopened.validate_integrity().unwrap();
    }

    #[test]
    fn schema_eleven_complete_version_media_is_recovered_from_live_content() {
        let directory = TestDirectory::new("schema_v11_version_media_recoverable");
        let (database_path, historical_json, content, sha256) =
            schema_eleven_version_only_history_fixture(&directory, true);

        let opened = SqliteServerStore::open_with_options(ServerStoreOpenOptions {
            database_path: database_path.clone(),
            legacy_json_path: None,
            now_epoch_millis: 500,
            legacy_token_ttl_millis: DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
        })
        .unwrap();
        assert!(opened.schema_migrated);
        assert!(opened.pre_schema_migration_backup.as_ref().is_some());
        let connection = opened.store.open_connection(false).unwrap();
        assert_eq!(SCHEMA_VERSION, current_schema_version(&connection).unwrap());
        assert_eq!(
            (1_i64, Some(sha256.clone()), String::new()),
            connection
                .query_row(
                    "SELECT h.media_snapshot_complete, m.content_sha256, m.missing_reason
                     FROM account_snapshot_history h
                     JOIN account_snapshot_media_history m
                       ON m.user_id = h.user_id AND m.account_revision = h.revision
                     WHERE h.user_id = 'user-a' AND h.revision = 1
                       AND m.attachment_id = 'version-attachment-a'",
                    [],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .unwrap()
        );
        drop(connection);

        let restored = opened
            .store
            .restore_account_snapshot("user-a", 1, 2, 600)
            .unwrap();
        assert_eq!(historical_json, restored.app_data_json);
        let restored_media = opened
            .store
            .read_media("user-a", "version-attachment-a")
            .unwrap()
            .unwrap();
        assert_eq!(content, restored_media.content);
        assert_eq!(sha256, restored_media.metadata.sha256);
        assert_eq!(100, restored_media.metadata.updated_at_epoch_millis);
        opened.store.verify_snapshot_storage_integrity().unwrap();
    }

    #[test]
    fn schema_eleven_missing_version_media_is_preserved_as_incomplete_evidence() {
        let directory = TestDirectory::new("schema_v11_version_media_missing");
        let (database_path, historical_json, _content, sha256) =
            schema_eleven_version_only_history_fixture(&directory, false);

        let opened = SqliteServerStore::open_with_options(ServerStoreOpenOptions {
            database_path: database_path.clone(),
            legacy_json_path: None,
            now_epoch_millis: 500,
            legacy_token_ttl_millis: DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
        })
        .unwrap();
        assert!(opened.schema_migrated);
        assert!(opened.pre_schema_migration_backup.as_ref().is_some());
        let connection = opened.store.open_connection(false).unwrap();
        assert_eq!(SCHEMA_VERSION, current_schema_version(&connection).unwrap());
        assert_eq!(
            (
                0_i64,
                None::<String>,
                sha256.clone(),
                "content_unavailable_at_snapshot".to_string(),
            ),
            connection
                .query_row(
                    "SELECT h.media_snapshot_complete, m.content_sha256,
                            m.declared_sha256, m.missing_reason
                     FROM account_snapshot_history h
                     JOIN account_snapshot_media_history m
                       ON m.user_id = h.user_id AND m.account_revision = h.revision
                     WHERE h.user_id = 'user-a' AND h.revision = 1
                       AND m.attachment_id = 'version-attachment-a'",
                    [],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    },
                )
                .unwrap()
        );
        drop(connection);
        assert!(opened
            .store
            .list_snapshot_history("user-a", 10)
            .unwrap()
            .iter()
            .any(|history| history.revision == 1 && history.app_data_json == historical_json));
        let before = opened.store.read_account("user-a").unwrap();
        assert!(matches!(
            opened
                .store
                .restore_account_snapshot("user-a", 1, 2, 600),
            Err(StoreError::Integrity(message)) if message.contains("media snapshot is incomplete")
        ));
        assert_eq!(before, opened.store.read_account("user-a").unwrap());
        opened.store.verify_snapshot_storage_integrity().unwrap();
    }

    #[test]
    fn schema_eleven_incomplete_history_reconciles_versions_before_delayed_repair() {
        let directory = TestDirectory::new("schema_v11_incomplete_version_media");
        let database_path = directory.0.join("server_store.sqlite3");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let current_content = b"current-media-uploaded-later";
        let current_sha256 = sha256_hex(current_content);
        let version_content = b"version-media-already-live";
        let version_sha256 = sha256_hex(version_content);
        let historical_json = json!({
            "notes": [{
                "id": "note-a",
                "attachments": [{
                    "id": "current-attachment-a",
                    "sha256": current_sha256.clone(),
                    "mimeType": "image/png",
                    "sizeBytes": current_content.len(),
                    "updatedAtEpochMillis": 100
                }],
                "revisions": [],
                "versions": [{
                    "id": "version-a",
                    "attachments": [{
                        "id": "version-attachment-a",
                        "sha256": version_sha256.clone(),
                        "mimeType": "image/png",
                        "sizeBytes": version_content.len(),
                        "updatedAtEpochMillis": 90
                    }]
                }]
            }]
        })
        .to_string();
        store
            .compare_and_swap_account("user-a", 0, &historical_json, 100)
            .unwrap();
        store
            .upsert_media(
                "user-a",
                "version-attachment-a",
                &version_sha256,
                "image/png",
                version_content.len() as i64,
                version_content,
                101,
            )
            .unwrap();
        store
            .compare_and_swap_account("user-a", 1, "{\"notes\":[]}", 102)
            .unwrap();
        let connection = store.open_connection(false).unwrap();
        assert_eq!(
            1,
            connection
                .execute(
                    "DELETE FROM account_snapshot_media_history
                     WHERE user_id = 'user-a' AND account_revision = 1
                       AND attachment_id = 'version-attachment-a'",
                    [],
                )
                .unwrap()
        );
        assert_eq!(
            0_i64,
            connection
                .query_row(
                    "SELECT media_snapshot_complete FROM account_snapshot_history
                     WHERE user_id = 'user-a' AND revision = 1",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
        );
        connection
            .execute_batch(
                "DROP TABLE IF EXISTS account_note_privacy; DROP TABLE IF EXISTS note_privacy_commit_witnesses;
                 DELETE FROM schema_migrations WHERE version >= 12;
                 PRAGMA user_version = 11;",
            )
            .unwrap();
        drop(connection);
        drop(store);

        let opened = SqliteServerStore::open_with_options(ServerStoreOpenOptions {
            database_path,
            legacy_json_path: None,
            now_epoch_millis: 500,
            legacy_token_ttl_millis: DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
        })
        .unwrap();
        let connection = opened.store.open_connection(false).unwrap();
        assert_eq!(
            (0_i64, 2_i64, 1_i64),
            connection
                .query_row(
                    "SELECT h.media_snapshot_complete, COUNT(m.attachment_id),
                            SUM(CASE WHEN m.content_sha256 IS NULL THEN 1 ELSE 0 END)
                     FROM account_snapshot_history h
                     JOIN account_snapshot_media_history m
                       ON m.user_id = h.user_id AND m.account_revision = h.revision
                     WHERE h.user_id = 'user-a' AND h.revision = 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap()
        );
        assert_eq!(
            Some(version_sha256.clone()),
            connection
                .query_row(
                    "SELECT content_sha256 FROM account_snapshot_media_history
                     WHERE user_id = 'user-a' AND account_revision = 1
                       AND attachment_id = 'version-attachment-a'",
                    [],
                    |row| row.get::<_, Option<String>>(0),
                )
                .unwrap()
        );
        drop(connection);

        opened
            .store
            .upsert_media(
                "user-a",
                "current-attachment-a",
                &current_sha256,
                "image/png",
                current_content.len() as i64,
                current_content,
                700,
            )
            .unwrap();
        let connection = opened.store.open_connection(false).unwrap();
        assert_eq!(
            1_i64,
            connection
                .query_row(
                    "SELECT media_snapshot_complete FROM account_snapshot_history
                     WHERE user_id = 'user-a' AND revision = 1",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
        );
        drop(connection);
        let restored = opened
            .store
            .restore_account_snapshot("user-a", 1, 2, 800)
            .unwrap();
        assert_eq!(historical_json, restored.app_data_json);
        assert_eq!(
            version_content,
            opened
                .store
                .read_media("user-a", "version-attachment-a")
                .unwrap()
                .unwrap()
                .content
                .as_slice()
        );
        opened.store.verify_snapshot_storage_integrity().unwrap();
    }

    #[test]
    fn schema_eleven_existing_manifest_adopts_version_timestamp_without_downgrade() {
        let directory = TestDirectory::new("schema_v11_version_timestamp");
        let database_path = directory.0.join("server_store.sqlite3");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let content = b"same-media-in-revision-and-version";
        let sha256 = sha256_hex(content);
        let historical_json = json!({
            "notes": [{
                "id": "note-a",
                "attachments": [],
                "revisions": [{
                    "id": "revision-a",
                    "attachments": [{
                        "id": "shared-attachment-a",
                        "sha256": sha256.clone(),
                        "mimeType": "image/png",
                        "sizeBytes": content.len(),
                        "updatedAtEpochMillis": 100
                    }]
                }],
                "versions": [{
                    "id": "version-a",
                    "attachments": [{
                        "id": "shared-attachment-a",
                        "sha256": sha256.clone(),
                        "mimeType": "image/png",
                        "sizeBytes": content.len(),
                        "updatedAtEpochMillis": 300
                    }]
                }]
            }]
        })
        .to_string();
        store
            .compare_and_swap_account("user-a", 0, &historical_json, 100)
            .unwrap();
        store
            .upsert_media(
                "user-a",
                "shared-attachment-a",
                &sha256,
                "image/png",
                content.len() as i64,
                content,
                301,
            )
            .unwrap();
        store
            .compare_and_swap_account("user-a", 1, "{\"notes\":[]}", 302)
            .unwrap();
        let connection = store.open_connection(false).unwrap();
        assert_eq!(
            1,
            connection
                .execute(
                    "UPDATE account_snapshot_media_history
                     SET updated_at_epoch_millis = 100
                     WHERE user_id = 'user-a' AND account_revision = 1
                       AND attachment_id = 'shared-attachment-a'",
                    [],
                )
                .unwrap()
        );
        connection
            .execute_batch(
                "DROP TABLE IF EXISTS account_note_privacy; DROP TABLE IF EXISTS note_privacy_commit_witnesses;
                 DELETE FROM schema_migrations WHERE version >= 12;
                 PRAGMA user_version = 11;",
            )
            .unwrap();
        drop(connection);
        drop(store);

        let opened = SqliteServerStore::open_with_options(ServerStoreOpenOptions {
            database_path,
            legacy_json_path: None,
            now_epoch_millis: 500,
            legacy_token_ttl_millis: DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
        })
        .unwrap();
        let connection = opened.store.open_connection(false).unwrap();
        assert_eq!(
            (1_i64, 300_i64),
            connection
                .query_row(
                    "SELECT h.media_snapshot_complete, m.updated_at_epoch_millis
                     FROM account_snapshot_history h
                     JOIN account_snapshot_media_history m
                       ON m.user_id = h.user_id AND m.account_revision = h.revision
                     WHERE h.user_id = 'user-a' AND h.revision = 1
                       AND m.attachment_id = 'shared-attachment-a'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap()
        );
        drop(connection);
        let restored = opened
            .store
            .restore_account_snapshot("user-a", 1, 2, 600)
            .unwrap();
        assert_eq!(historical_json, restored.app_data_json);
        assert_eq!(
            300,
            opened
                .store
                .read_media("user-a", "shared-attachment-a")
                .unwrap()
                .unwrap()
                .metadata
                .updated_at_epoch_millis
        );
        opened.store.verify_snapshot_storage_integrity().unwrap();
    }

    #[test]
    fn schema_eleven_existing_null_manifest_is_repaired_and_promoted_atomically() {
        let directory = TestDirectory::new("schema_v11_null_manifest_repair");
        let database_path = directory.0.join("server_store.sqlite3");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let sha256 = sha256_hex(b"");
        let historical_json = json!({
            "notes": [{
                "id": "note-a",
                "attachments": [],
                "revisions": [],
                "versions": [{
                    "id": "version-a",
                    "attachments": [{
                        "id": "zero-byte-attachment-a",
                        "sha256": sha256.clone(),
                        "mimeType": "application/octet-stream",
                        "sizeBytes": 0,
                        "updatedAtEpochMillis": 100
                    }]
                }]
            }]
        })
        .to_string();
        store
            .compare_and_swap_account("user-a", 0, &historical_json, 100)
            .unwrap();
        store
            .upsert_media(
                "user-a",
                "zero-byte-attachment-a",
                &sha256,
                "application/octet-stream",
                0,
                b"",
                101,
            )
            .unwrap();
        store
            .compare_and_swap_account("user-a", 1, "{\"notes\":[]}", 102)
            .unwrap();
        let connection = store.open_connection(false).unwrap();
        assert_eq!(
            1,
            connection
                .execute(
                    "UPDATE account_snapshot_media_history
                     SET content_sha256 = NULL,
                         missing_reason = 'reference_metadata_unavailable_at_snapshot'
                     WHERE user_id = 'user-a' AND account_revision = 1
                       AND attachment_id = 'zero-byte-attachment-a'",
                    [],
                )
                .unwrap()
        );
        assert_eq!(
            1,
            connection
                .execute(
                    "UPDATE account_snapshot_history
                     SET media_snapshot_complete = 0
                     WHERE user_id = 'user-a' AND revision = 1",
                    [],
                )
                .unwrap()
        );
        connection
            .execute_batch(
                "DROP TABLE IF EXISTS account_note_privacy; DROP TABLE IF EXISTS note_privacy_commit_witnesses;
                 DELETE FROM schema_migrations WHERE version >= 12;
                 PRAGMA user_version = 11;",
            )
            .unwrap();
        drop(connection);
        drop(store);

        let opened = SqliteServerStore::open_with_options(ServerStoreOpenOptions {
            database_path,
            legacy_json_path: None,
            now_epoch_millis: 500,
            legacy_token_ttl_millis: DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
        })
        .unwrap();
        let connection = opened.store.open_connection(false).unwrap();
        assert_eq!(
            (1_i64, Some(sha256.clone()), String::new(), 1_i64),
            connection
                .query_row(
                    "SELECT h.media_snapshot_complete, m.content_sha256,
                            m.missing_reason, c.reference_count
                     FROM account_snapshot_history h
                     JOIN account_snapshot_media_history m
                       ON m.user_id = h.user_id AND m.account_revision = h.revision
                     JOIN media_snapshot_contents c ON c.sha256 = m.content_sha256
                     WHERE h.user_id = 'user-a' AND h.revision = 1
                       AND m.attachment_id = 'zero-byte-attachment-a'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .unwrap()
        );
        drop(connection);
        let restored = opened
            .store
            .restore_account_snapshot("user-a", 1, 2, 600)
            .unwrap();
        assert_eq!(historical_json, restored.app_data_json);
        assert!(opened
            .store
            .read_media("user-a", "zero-byte-attachment-a")
            .unwrap()
            .unwrap()
            .content
            .is_empty());
        opened.store.verify_snapshot_storage_integrity().unwrap();
    }

    #[test]
    fn request_dedup_replays_only_the_same_operation_payload() {
        let directory = TestDirectory::new("request_dedup");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let first = store
            .apply_sync_request(
                "user-a",
                "request-1",
                "{\"appData\":{\"value\":1},\"forceUpload\":false}",
                0,
                "{\"value\":1}",
                "{\"ok\":true}",
                100,
            )
            .unwrap();
        assert!(matches!(first, SyncRequestOutcome::Applied(_)));
        let replay = store
            .apply_sync_request(
                "user-a",
                "request-1",
                "{ \"forceUpload\" : false, \"appData\" : { \"value\" : 1 } }",
                1,
                "{\"value\":1}",
                "{\"ok\":false}",
                101,
            )
            .unwrap();
        assert_eq!(
            SyncRequestOutcome::Replayed(SyncRequestReceipt {
                response_json: "{\"ok\":true}".to_string(),
                account_revision: 1,
            }),
            replay
        );
        assert_eq!(
            "{\"value\":1}",
            store.read_account("user-a").unwrap().app_data_json
        );

        assert!(matches!(
            store.apply_sync_request(
                "user-a",
                "request-1",
                "{\"appData\":{\"value\":999},\"forceUpload\":false}",
                1,
                "{\"value\":999}",
                "{\"ok\":false}",
                102,
            ),
            Err(StoreError::Integrity(message)) if message.contains("different operation payload")
        ));
        assert_eq!(
            "{\"value\":1}",
            store.read_account("user-a").unwrap().app_data_json
        );
    }

    #[test]
    fn snapshot_history_restores_without_reusing_revision() {
        let directory = TestDirectory::new("history");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        store
            .compare_and_swap_account("user-a", 0, "{\"value\":1}", 100)
            .unwrap();
        store
            .compare_and_swap_account("user-a", 1, "{\"value\":2}", 200)
            .unwrap();
        let restored = store.restore_account_snapshot("user-a", 0, 2, 300).unwrap();
        assert_eq!(3, restored.revision);
        assert_eq!("{}", restored.app_data_json);
        assert_eq!(3, store.snapshot_history_count("user-a").unwrap());
        let storage_after_first_restore = store.snapshot_storage_stats("user-a").unwrap();

        let restored_again = store.restore_account_snapshot("user-a", 1, 3, 400).unwrap();
        assert_eq!(4, restored_again.revision);
        assert_eq!("{\"value\":1}", restored_again.app_data_json);
        let restored_third = store.restore_account_snapshot("user-a", 0, 4, 500).unwrap();
        assert_eq!(5, restored_third.revision);
        assert_eq!("{}", restored_third.app_data_json);
        assert_eq!(
            storage_after_first_restore,
            store.snapshot_storage_stats("user-a").unwrap()
        );
    }

    #[test]
    fn restore_refuses_to_discard_an_incomplete_current_media_snapshot() {
        let directory = TestDirectory::new("restore_preserves_incomplete_current");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        store
            .compare_and_swap_account("user-a", 0, "{\"value\":1}", 100)
            .unwrap();
        let current_json =
            "{\"notes\":[{\"id\":\"note-a\",\"attachmentIds\":[\"missing-media\"]}]}";
        seed_grandfathered_current_snapshot(&store, "user-a", 1, current_json, 200);
        let before = store.read_account("user-a").unwrap();
        let storage_before = store.snapshot_storage_stats("user-a").unwrap();

        assert!(matches!(
            store.restore_account_snapshot("user-a", 0, 2, 300),
            Err(StoreError::Integrity(message))
                if message.contains("could not be archived completely")
                    && message.contains("restore was not applied")
        ));
        assert_eq!(before, store.read_account("user-a").unwrap());
        assert_eq!(
            storage_before,
            store.snapshot_storage_stats("user-a").unwrap()
        );
    }

    #[test]
    fn media_round_trip_checks_hash_and_soft_deletes() {
        let directory = TestDirectory::new("media");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let bytes = b"image-bytes";
        let hash = sha256_hex(bytes);
        assert!(matches!(
            store
                .upsert_media(
                    "user-a",
                    "attachment-1",
                    &hash,
                    "image/png",
                    bytes.len() as i64,
                    bytes,
                    100,
                )
                .unwrap(),
            MediaUpsertOutcome::Stored(_)
        ));
        assert_eq!(
            bytes,
            store
                .read_media("user-a", "attachment-1")
                .unwrap()
                .unwrap()
                .content
                .as_slice()
        );
        assert_eq!(
            bytes.len() as i64,
            store.media_usage_bytes("user-a", false).unwrap()
        );
        let deleted = store
            .delete_media("user-a", "attachment-1", 200, 1_000)
            .unwrap();
        assert_eq!(Some(200), deleted.deleted_at_epoch_millis);
        assert!(store
            .read_media("user-a", "attachment-1")
            .unwrap()
            .is_none());
        assert_eq!(0, store.media_usage_bytes("user-a", false).unwrap());
        assert_eq!(1, store.list_media_metadata("user-a", true).unwrap().len());

        assert!(matches!(
            store
                .upsert_media(
                    "user-a",
                    "attachment-1",
                    &hash,
                    "image/png",
                    bytes.len() as i64,
                    bytes,
                    100,
                )
                .unwrap(),
            MediaUpsertOutcome::RejectedByTombstone(MediaMetadata {
                deleted_at_epoch_millis: Some(200),
                ..
            })
        ));
        assert!(store
            .read_media("user-a", "attachment-1")
            .unwrap()
            .is_none());

        assert!(matches!(
            store
                .upsert_media_with_restore(
                    "user-a",
                    "attachment-1",
                    &hash,
                    "image/png",
                    bytes.len() as i64,
                    bytes,
                    201,
                    true,
                )
                .unwrap(),
            MediaUpsertOutcome::Stored(MediaMetadata {
                updated_at_epoch_millis: 201,
                deleted_at_epoch_millis: None,
                ..
            })
        ));
        assert_eq!(
            bytes.len() as i64,
            store.media_usage_bytes("user-a", false).unwrap()
        );
    }

    #[test]
    fn corrupt_live_media_is_rejected_by_reads_audits_and_backups_then_idempotently_repaired() {
        let directory = TestDirectory::new("live_media_corruption");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let content = b"verified-media";
        let sha256 = sha256_hex(content);
        store
            .upsert_media(
                "user-a",
                "attachment-a",
                &sha256,
                "application/octet-stream",
                content.len() as i64,
                content,
                100,
            )
            .unwrap();
        let corrupt = b"corrupted-medi";
        assert_eq!(content.len(), corrupt.len());
        let connection = store.open_connection(false).unwrap();
        connection
            .execute(
                "UPDATE note_media SET content = ?1
                 WHERE user_id = 'user-a' AND attachment_id = 'attachment-a'",
                params![corrupt.as_slice()],
            )
            .unwrap();
        drop(connection);

        assert!(matches!(
            store.read_media("user-a", "attachment-a"),
            Err(StoreError::Integrity(message)) if message.contains("live media content is corrupt")
        ));
        assert!(matches!(
            store.validate_integrity(),
            Err(StoreError::Integrity(message)) if message.contains("live media content is corrupt")
        ));
        let rejected_backup = directory.0.join("corrupt-backup.sqlite3");
        assert!(matches!(
            store.create_verified_backup(&rejected_backup, 200),
            Err(StoreError::Integrity(message)) if message.contains("live media content is corrupt")
        ));
        assert!(!rejected_backup.exists());

        // A same-revision retry carries independently verified bytes and heals
        // only the corrupt BLOB without advancing the logical media revision.
        let repaired = store
            .upsert_media(
                "user-a",
                "attachment-a",
                &sha256,
                "application/octet-stream",
                content.len() as i64,
                content,
                100,
            )
            .unwrap();
        assert!(matches!(
            repaired,
            MediaUpsertOutcome::Stored(MediaMetadata {
                updated_at_epoch_millis: 100,
                ..
            })
        ));
        assert_eq!(
            content.as_slice(),
            store
                .read_media("user-a", "attachment-a")
                .unwrap()
                .unwrap()
                .content
                .as_slice()
        );
        store.validate_integrity().unwrap();
        store
            .create_verified_backup(directory.0.join("repaired-backup.sqlite3"), 201)
            .unwrap();
    }

    #[test]
    fn deleted_media_blob_gc_keeps_tombstone_and_blocks_stale_revival() {
        let directory = TestDirectory::new("media_gc");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let bytes = b"retained-image";
        let hash = sha256_hex(bytes);
        store
            .upsert_media(
                "user-a",
                "attachment-1",
                &hash,
                "image/png",
                bytes.len() as i64,
                bytes,
                100,
            )
            .unwrap();
        store
            .delete_media("user-a", "attachment-1", 200, 1_000)
            .unwrap();
        assert_eq!(
            0,
            store
                .prune_deleted_media_content(1_000 + DELETED_MEDIA_CONTENT_RETENTION_MILLIS)
                .unwrap()
        );
        assert_eq!(
            bytes.len() as i64,
            store.media_usage_bytes("user-a", true).unwrap()
        );
        assert_eq!(
            1,
            store
                .prune_deleted_media_content(1_001 + DELETED_MEDIA_CONTENT_RETENTION_MILLIS)
                .unwrap()
        );
        assert_eq!(0, store.media_usage_bytes("user-a", true).unwrap());
        let manifest = store.list_media_metadata("user-a", true).unwrap();
        assert_eq!(1, manifest.len());
        assert_eq!(Some(200), manifest[0].deleted_at_epoch_millis);
        assert!(matches!(
            store
                .upsert_media(
                    "user-a",
                    "attachment-1",
                    &hash,
                    "image/png",
                    bytes.len() as i64,
                    bytes,
                    100,
                )
                .unwrap(),
            MediaUpsertOutcome::RejectedByTombstone(_)
        ));
    }

    #[test]
    fn deleting_missing_media_records_tombstone_and_blocks_fast_clock_upload() {
        let directory = TestDirectory::new("missing_media_delete");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let deleted = store
            .delete_media("user-a", "attachment-never-uploaded", 200, 1_000)
            .unwrap();
        assert_eq!(Some(200), deleted.deleted_at_epoch_millis);
        assert_eq!(0, deleted.size_bytes);
        assert_eq!(
            vec![deleted],
            store.list_media_metadata("user-a", true).unwrap()
        );

        let content = b"offline-device-content";
        assert!(matches!(
            store
                .upsert_media(
                    "user-a",
                    "attachment-never-uploaded",
                    &sha256_hex(content),
                    "application/octet-stream",
                    content.len() as i64,
                    content,
                    system_time_epoch_millis(),
                )
                .unwrap(),
            MediaUpsertOutcome::RejectedByTombstone(MediaMetadata {
                deleted_at_epoch_millis: Some(200),
                ..
            })
        ));
        assert_eq!(0, store.media_usage_bytes("user-a", false).unwrap());
    }

    #[test]
    fn non_positive_media_revisions_are_rejected_without_poisoning_attachment_id() {
        let directory = TestDirectory::new("invalid_media_revision");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let content = b"future-content";
        assert!(matches!(
            store.upsert_media(
                "user-a",
                "attachment-1",
                &sha256_hex(content),
                "application/octet-stream",
                content.len() as i64,
                content,
                0,
            ),
            Err(StoreError::Integrity(message)) if message.contains("must be positive")
        ));
        assert!(matches!(
            store.delete_media("user-a", "attachment-1", 0, 1_000),
            Err(StoreError::Integrity(message)) if message.contains("positive delete revision")
        ));
        assert!(store
            .list_media_metadata("user-a", true)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn batch_import_matches_normalized_email_and_online_backup_verifies() {
        let directory = TestDirectory::new("batch_backup");
        let store = open_empty(&directory);
        let first = LegacyServerStore {
            users: vec![LegacyServerUser {
                id: "canonical-id".to_string(),
                email: "Person@Example.test".to_string(),
                password_salt: "salt-1".to_string(),
                password_hash: "hash-1".to_string(),
                created_at_epoch_millis: 1,
                updated_at_epoch_millis: 10,
                app_data_json: "{\"old\":1}".to_string(),
                tokens: vec![LegacyServerToken {
                    token: "token-1".to_string(),
                    device_name: "phone".to_string(),
                    created_at_epoch_millis: 1,
                    last_seen_at_epoch_millis: 2,
                }],
            }],
        };
        let mut second = first.clone();
        second.users[0].email = "person@example.test".to_string();
        second.users[0].updated_at_epoch_millis = 20;
        second.users[0].app_data_json = "{\"new\":2}".to_string();
        second.users[0].tokens[0].token = "token-2".to_string();
        let first_path = directory.0.join("server_store.json");
        let second_path = directory.0.join("server_store_before_reset.json");
        fs::write(&first_path, serde_json::to_vec(&first).unwrap()).unwrap();
        fs::write(&second_path, serde_json::to_vec(&second).unwrap()).unwrap();
        fs::write(
            directory.0.join("server_store_startup_state_v1.json"),
            br#"{"backupFileName":"server_store_startup_1.sqlite3"}"#,
        )
        .unwrap();
        let paths = SqliteServerStore::discover_legacy_json_files(&directory.0).unwrap();
        assert_eq!(2, paths.len());
        assert!(paths.contains(&first_path));
        assert!(paths.contains(&second_path));
        let report = store
            .import_legacy_files_with_merge(&paths, 100, 1_000, merge_json_objects)
            .unwrap();
        assert_eq!(2, report.files.len());
        assert_eq!(1, store.stats().unwrap().users);
        assert_eq!(2, store.stats().unwrap().tokens);
        let snapshot = store.read_account("canonical-id").unwrap();
        assert!(snapshot.app_data_json.contains("\"old\":1"));
        assert!(snapshot.app_data_json.contains("\"new\":2"));

        let backup_path = directory.0.join("verified.sqlite3");
        let backup = store.create_verified_backup(&backup_path, 200).unwrap();
        assert_eq!(backup.sha256, sha256_file(&backup_path).unwrap());
        let backup_store = SqliteServerStore::open(&backup_path, None).unwrap();
        assert_eq!(store.stats().unwrap(), backup_store.stats().unwrap());
        assert_eq!(snapshot, backup_store.read_account("canonical-id").unwrap());
    }

    #[test]
    fn schema_upgrade_creates_verified_backup_before_any_migration_ddl() {
        let directory = TestDirectory::new("pre_schema_backup");
        let database_path = directory.0.join("server_store.sqlite3");
        let store = SqliteServerStore::open(&database_path, None).unwrap();
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let content = b"pre-migration-media";
        store
            .upsert_media(
                "user-a",
                "attachment-1",
                &sha256_hex(content),
                "application/octet-stream",
                content.len() as i64,
                content,
                100,
            )
            .unwrap();
        drop(store);

        let pre_upgrade = Connection::open(&database_path).unwrap();
        pre_upgrade
            .execute_batch(
                "DROP TABLE IF EXISTS account_note_privacy; DROP TABLE IF EXISTS note_privacy_commit_witnesses;
                 DELETE FROM schema_migrations WHERE version >= 6;
                 PRAGMA user_version = 5;",
            )
            .unwrap();
        assert_eq!(5, current_schema_version(&pre_upgrade).unwrap());
        drop(pre_upgrade);

        let opened = SqliteServerStore::open_with_options(ServerStoreOpenOptions {
            database_path: database_path.clone(),
            legacy_json_path: None,
            now_epoch_millis: 777,
            legacy_token_ttl_millis: DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
        })
        .unwrap();
        assert!(opened.schema_migrated);
        let report = opened.pre_schema_migration_backup.as_ref().unwrap();
        assert!(report.destination.exists());
        assert_eq!(
            report.size_bytes,
            fs::metadata(&report.destination).unwrap().len()
        );
        assert_eq!(report.sha256, sha256_file(&report.destination).unwrap());
        assert!(report
            .destination
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains(&format!("_pre_schema_v5_to_v{SCHEMA_VERSION}_777")));

        let backup = Connection::open_with_flags(
            &report.destination,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
        )
        .unwrap();
        assert_eq!(5, current_schema_version(&backup).unwrap());
        assert_eq!(
            1,
            backup
                .query_row("SELECT COUNT(*) FROM users", [], |row| row.get::<_, i64>(0))
                .unwrap()
        );
        assert_eq!(
            content.as_slice(),
            backup
                .query_row(
                    "SELECT content FROM note_media WHERE attachment_id = 'attachment-1'",
                    [],
                    |row| row.get::<_, Vec<u8>>(0),
                )
                .unwrap()
                .as_slice()
        );
        assert_eq!(
            0,
            backup
                .query_row(
                    "SELECT COUNT(*) FROM schema_migrations WHERE version >= 6",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
        );
        drop(backup);

        let migrated = opened.store.open_connection(false).unwrap();
        assert_eq!(SCHEMA_VERSION, current_schema_version(&migrated).unwrap());
        assert_eq!(
            4,
            migrated
                .query_row(
                    "SELECT COUNT(*) FROM schema_migrations WHERE version IN (6, 7, 8, 9)",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
        );
    }

    #[test]
    fn current_schema_fails_closed_when_token_activation_column_is_missing() {
        let directory = TestDirectory::new("missing_token_activation_column");
        let database_path = directory.0.join("server_store.sqlite3");
        let store = SqliteServerStore::open(&database_path, None).unwrap();
        drop(store);

        let connection = Connection::open(&database_path).unwrap();
        connection
            .execute_batch("ALTER TABLE tokens DROP COLUMN activated_at_epoch_millis;")
            .unwrap();
        drop(connection);

        let error = SqliteServerStore::open(&database_path, None).unwrap_err();
        assert!(error
            .to_string()
            .contains("required token activation timestamp column is missing"));
    }

    #[test]
    fn schema_v3_history_migrates_losslessly_to_compressed_content_objects() {
        let directory = TestDirectory::new("snapshot_v3_migration");
        let database_path = directory.0.join("server_store.sqlite3");
        let store = SqliteServerStore::open(&database_path, None).unwrap();
        let mut user = new_user("user-a", "a@example.test");
        user.app_data_json = "{\"value\":\"a\"}".to_string();
        store.create_user(user).unwrap();
        store
            .compare_and_swap_account("user-a", 0, "{\"value\":\"b\"}", 100)
            .unwrap();
        store.restore_account_snapshot("user-a", 0, 1, 200).unwrap();
        store
            .compare_and_swap_account("user-a", 2, "{\"value\":\"c\"}", 300)
            .unwrap();
        let expected = store.list_snapshot_history("user-a", 10).unwrap();
        drop(store);

        let mut connection = Connection::open(&database_path).unwrap();
        let transaction = connection.transaction().unwrap();
        transaction
            .execute_batch(
                "CREATE TABLE account_snapshot_history_v3 (
                     user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                     revision INTEGER NOT NULL CHECK(revision >= 0),
                     app_data_json TEXT NOT NULL,
                     created_at_epoch_millis INTEGER NOT NULL,
                     sha256 TEXT NOT NULL CHECK(length(sha256) = 64),
                     PRIMARY KEY(user_id, revision),
                     CHECK(length(trim(app_data_json)) = 0 OR json_valid(app_data_json))
                 ) STRICT;",
            )
            .unwrap();
        for snapshot in &expected {
            transaction
                .execute(
                    "INSERT INTO account_snapshot_history_v3(
                         user_id, revision, app_data_json, created_at_epoch_millis, sha256
                     ) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        snapshot.user_id,
                        snapshot.revision,
                        snapshot.app_data_json,
                        snapshot.created_at_epoch_millis,
                        snapshot.sha256,
                    ],
                )
                .unwrap();
        }
        transaction
            .execute_batch(
                "DROP TRIGGER account_snapshot_history_insert_ref;
                 DROP TRIGGER account_snapshot_history_delete_ref;
                 DROP TRIGGER account_snapshot_history_update_ref;
                 DROP TRIGGER account_snapshot_media_history_insert_ref;
                 DROP TRIGGER account_snapshot_media_history_delete_ref;
                 DROP TABLE account_snapshot_media_history;
                 DROP TABLE media_snapshot_contents;
                 DROP TABLE account_snapshot_history;
                 DROP TABLE snapshot_contents;
                 DROP TABLE snapshot_history_prune_audit;
                 ALTER TABLE account_snapshot_history_v3 RENAME TO account_snapshot_history;
                 CREATE INDEX account_snapshot_history_created_index
                     ON account_snapshot_history(user_id, created_at_epoch_millis DESC);
                 DROP TABLE IF EXISTS account_note_privacy; DROP TABLE IF EXISTS note_privacy_commit_witnesses;
                 DELETE FROM schema_migrations WHERE version >= 4;
                 PRAGMA user_version = 3;",
            )
            .unwrap();
        transaction.commit().unwrap();
        drop(connection);

        let migrated = SqliteServerStore::open_with_options(ServerStoreOpenOptions {
            database_path,
            legacy_json_path: None,
            now_epoch_millis: 400,
            legacy_token_ttl_millis: DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
        })
        .unwrap();
        assert_eq!(
            expected,
            migrated.store.list_snapshot_history("user-a", 10).unwrap()
        );
        let connection = migrated.store.open_connection(false).unwrap();
        let (objects, references) = connection
            .query_row(
                "SELECT COUNT(*), SUM(reference_count) FROM snapshot_contents",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .unwrap();
        assert_eq!(2, objects);
        assert_eq!(3, references);
        migrated.store.verify_snapshot_storage_integrity().unwrap();
    }

    #[test]
    fn legacy_identity_collisions_fail_closed_without_snapshot_or_token_merge() {
        for (case, incoming_id, incoming_email) in [
            ("id_points_to_a_email_to_b", "user-a", "b@example.test"),
            (
                "email_points_to_a_id_differs",
                "attacker-id",
                "a@example.test",
            ),
        ] {
            let directory = TestDirectory::new(case);
            let store = open_empty(&directory);
            let mut user_a = new_user("user-a", "a@example.test");
            user_a.app_data_json = "{\"owner\":\"a\"}".to_string();
            let mut user_b = new_user("user-b", "b@example.test");
            user_b.app_data_json = "{\"owner\":\"b\"}".to_string();
            store.create_users_atomically(&[user_a, user_b]).unwrap();
            let before_a = store.find_user_by_id("user-a").unwrap().unwrap();
            let before_b = store.find_user_by_id("user-b").unwrap().unwrap();
            let legacy = LegacyServerStore {
                users: vec![LegacyServerUser {
                    id: incoming_id.to_string(),
                    email: incoming_email.to_string(),
                    password_salt: "attacker-salt".to_string(),
                    password_hash: "attacker-hash".to_string(),
                    created_at_epoch_millis: 1,
                    updated_at_epoch_millis: 999_999,
                    app_data_json: "{\"owner\":\"attacker\"}".to_string(),
                    tokens: vec![LegacyServerToken {
                        token: format!("attacker-token-{case}"),
                        device_name: "attacker-device".to_string(),
                        created_at_epoch_millis: 10,
                        last_seen_at_epoch_millis: 20,
                    }],
                }],
            };
            let source = directory.0.join("server_store_collision.json");
            fs::write(&source, serde_json::to_vec(&legacy).unwrap()).unwrap();

            let result = store.import_legacy_files_with_merge(
                std::slice::from_ref(&source),
                1_000_000,
                1_000,
                merge_json_objects,
            );
            assert!(matches!(
                result,
                Err(StoreError::Integrity(message)) if message.contains("identity collision")
            ));
            assert_eq!(before_a, store.find_user_by_id("user-a").unwrap().unwrap());
            assert_eq!(before_b, store.find_user_by_id("user-b").unwrap().unwrap());
            assert!(store.find_user_by_id("attacker-id").unwrap().is_none());
            assert!(store.list_token_metadata("user-a").unwrap().is_empty());
            assert!(store.list_token_metadata("user-b").unwrap().is_empty());
        }
    }

    #[test]
    fn concurrent_cas_allows_exactly_one_writer() {
        let directory = TestDirectory::new("cas");
        let store = Arc::new(open_empty(&directory));
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let barrier = Arc::new(Barrier::new(3));
        let mut handles = Vec::new();
        for writer in 1..=2 {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                store.compare_and_swap_account(
                    "user-a",
                    0,
                    &format!("{{\"writer\":{writer}}}"),
                    100 + writer,
                )
            }));
        }
        barrier.wait();
        let results = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(1, results.iter().filter(|result| result.is_ok()).count());
        assert_eq!(
            1,
            results
                .iter()
                .filter(|result| matches!(result, Err(StoreError::RevisionConflict { .. })))
                .count()
        );
        let snapshot = store.read_account("user-a").unwrap();
        assert_eq!(1, snapshot.revision);
        assert!(
            snapshot.app_data_json == "{\"writer\":1}"
                || snapshot.app_data_json == "{\"writer\":2}"
        );
    }

    #[test]
    fn corrupted_existing_database_fails_closed() {
        let directory = TestDirectory::new("corrupt");
        let database_path = directory.0.join("server_store.sqlite3");
        fs::write(&database_path, b"this is not sqlite").unwrap();
        let result = SqliteServerStore::open(&database_path, None);
        assert!(result.is_err());
        assert_eq!(
            b"this is not sqlite",
            fs::read(&database_path).unwrap().as_slice()
        );
    }

    #[test]
    fn malformed_legacy_json_fails_without_backup_or_import() {
        let directory = TestDirectory::new("bad_legacy");
        let legacy_path = directory.0.join("server_store.json");
        fs::write(&legacy_path, b"{not-json").unwrap();
        let database_path = directory.0.join("server_store.sqlite3");
        let result = SqliteServerStore::open_with_options(ServerStoreOpenOptions {
            database_path,
            legacy_json_path: Some(legacy_path),
            now_epoch_millis: 100,
            legacy_token_ttl_millis: 100,
        });
        assert!(matches!(result, Err(StoreError::Json(_))));
        assert_eq!(
            0,
            fs::read_dir(&directory.0)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .contains("_pre_sqlite_migration_"))
                .count()
        );
    }

    #[test]
    fn future_schema_legacy_import_preserves_source_and_existing_snapshot() {
        let directory = TestDirectory::new("future_legacy");
        let store = open_empty(&directory);
        let mut existing = new_user("user-a", "a@example.test");
        existing.app_data_json = format!(
            "{{\"schemaVersion\":{},\"marker\":\"kept\"}}",
            APP_DATA_SCHEMA_VERSION
        );
        store.create_user(existing).unwrap();
        let before = store.read_account("user-a").unwrap();
        let legacy = LegacyServerStore {
            users: vec![LegacyServerUser {
                id: "user-a".to_string(),
                email: "a@example.test".to_string(),
                password_salt: "salt-a".to_string(),
                password_hash: "hash-a".to_string(),
                created_at_epoch_millis: 1,
                updated_at_epoch_millis: 20,
                app_data_json: format!(
                    "{{\"schemaVersion\":{},\"futureNested\":{{\"opaque\":true}}}}",
                    APP_DATA_SCHEMA_VERSION + 1
                ),
                tokens: Vec::new(),
            }],
        };
        let source_path = directory.0.join("server_store_future.json");
        let source_bytes = serde_json::to_vec_pretty(&legacy).unwrap();
        fs::write(&source_path, &source_bytes).unwrap();

        let result = store.import_legacy_files_with_merge(
            std::slice::from_ref(&source_path),
            100,
            1_000,
            merge_json_objects,
        );
        assert!(matches!(
            result,
            Err(StoreError::Integrity(message))
                if message.contains("newer than supported") && message.contains("upgrade required")
        ));
        assert_eq!(source_bytes, fs::read(&source_path).unwrap());
        assert_eq!(before, store.read_account("user-a").unwrap());
        assert_eq!(
            0,
            fs::read_dir(&directory.0)
                .unwrap()
                .filter_map(Result::ok)
                .filter(
                    |entry| entry.path().extension().and_then(|value| value.to_str())
                        == Some("gtlbak")
                )
                .count()
        );
    }

    #[test]
    fn snapshot_history_rejects_same_revision_with_divergent_content() {
        let directory = TestDirectory::new("history_divergence");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let current = store.read_account("user-a").unwrap();
        let mut connection = store.open_connection(false).unwrap();
        let transaction = connection.transaction().unwrap();
        insert_snapshot_history(&transaction, &current, 20)
            .expect("identical history replay should be accepted");
        let divergent = AccountSnapshot {
            app_data_json: "{\"fork\":true}".to_string(),
            ..current
        };
        assert!(matches!(
            insert_snapshot_history(&transaction, &divergent, 21),
            Err(StoreError::Integrity(message)) if message.contains("snapshot history diverged")
        ));
        transaction.rollback().unwrap();
    }

    #[test]
    fn snapshot_history_list_and_restore_reject_valid_json_with_wrong_sha256() {
        let directory = TestDirectory::new("history_sha_tamper");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        store
            .compare_and_swap_account("user-a", 0, "{\"value\":1}", 100)
            .unwrap();
        store
            .compare_and_swap_account("user-a", 1, "{\"value\":2}", 200)
            .unwrap();
        let before = store.read_account("user-a").unwrap();
        let connection = store.open_connection(false).unwrap();
        let tampered = compress_snapshot_content(b"{\"value\":999}").unwrap();
        connection
            .execute(
                "UPDATE snapshot_contents
                 SET content = ?1, uncompressed_size_bytes = ?2,
                     compressed_size_bytes = ?3
                 WHERE sha256 = (
                     SELECT content_sha256 FROM account_snapshot_history
                     WHERE user_id = 'user-a' AND revision = 1
                 )",
                params![
                    tampered,
                    b"{\"value\":999}".len() as i64,
                    tampered.len() as i64
                ],
            )
            .unwrap();
        drop(connection);

        assert!(matches!(
            store.list_snapshot_history("user-a", 10),
            Err(StoreError::Integrity(message)) if message.contains("SHA-256 mismatch")
        ));
        assert!(matches!(
            store.restore_account_snapshot("user-a", 1, 2, 300),
            Err(StoreError::Integrity(message)) if message.contains("SHA-256 mismatch")
        ));
        assert_eq!(before, store.read_account("user-a").unwrap());
    }

    #[test]
    fn valid_json_corruption_in_current_snapshot_is_rejected_by_reads_audits_and_backups() {
        let directory = TestDirectory::new("current_snapshot_content_hash");
        let store = open_empty(&directory);
        let mut user = new_user("user-a", "a@example.test");
        user.app_data_json = "{\"value\":1}".to_string();
        store.create_user(user).unwrap();

        let connection = store.open_connection(false).unwrap();
        connection
            .execute(
                "UPDATE account_snapshots SET app_data_json = '{\"value\":2}' \
                 WHERE user_id = 'user-a'",
                [],
            )
            .unwrap();
        verify_quick_check(&connection).unwrap();
        verify_integrity_check(&connection).unwrap();
        drop(connection);

        for result in [
            store.read_account("user-a").map(|_| ()),
            store.find_user_by_id("user-a").map(|_| ()),
            store.list_account_snapshots().map(|_| ()),
            store.validate_integrity(),
        ] {
            assert!(matches!(
                result,
                Err(StoreError::Integrity(message))
                    if message.contains("current account snapshot SHA-256 mismatch")
            ));
        }
        let backup_path = directory.0.join("corrupt-backup.sqlite3");
        assert!(matches!(
            store.create_verified_backup(&backup_path, 500),
            Err(StoreError::Integrity(message))
                if message.contains("current account snapshot SHA-256 mismatch")
        ));
        assert!(!backup_path.exists());

        drop(store);
        assert!(matches!(
            SqliteServerStore::open(directory.0.join("server_store.sqlite3"), None),
            Err(StoreError::Integrity(message))
                if message.contains("current account snapshot SHA-256 mismatch")
        ));
    }

    #[test]
    fn current_snapshot_metadata_corruption_is_rejected_by_envelope_hash() {
        let directory = TestDirectory::new("current_snapshot_envelope_hash");
        let store = open_empty(&directory);
        let mut user = new_user("user-a", "a@example.test");
        user.app_data_json = "{\"value\":1}".to_string();
        store.create_user(user).unwrap();

        let connection = store.open_connection(false).unwrap();
        connection
            .execute(
                "UPDATE account_snapshots SET revision = revision + 1 \
                 WHERE user_id = 'user-a'",
                [],
            )
            .unwrap();
        verify_quick_check(&connection).unwrap();
        verify_integrity_check(&connection).unwrap();
        drop(connection);

        assert!(matches!(
            store.read_account("user-a"),
            Err(StoreError::Integrity(message))
                if message.contains("snapshot envelope SHA-256 mismatch")
        ));
        assert!(matches!(
            store.validate_integrity(),
            Err(StoreError::Integrity(message))
                if message.contains("snapshot envelope SHA-256 mismatch")
        ));
        let backup_path = directory.0.join("metadata-corrupt-backup.sqlite3");
        assert!(matches!(
            store.create_verified_backup(&backup_path, 500),
            Err(StoreError::Integrity(message))
                if message.contains("snapshot envelope SHA-256 mismatch")
        ));
        assert!(!backup_path.exists());
    }

    #[test]
    fn snapshot_contents_are_compressed_deduplicated_and_reference_counted() {
        let directory = TestDirectory::new("snapshot_content_dedup");
        let store = open_empty(&directory);
        let repeated = format!("{{\"payload\":\"{}\"}}", "same-data-".repeat(4_096));
        let mut user_a = new_user("user-a", "a@example.test");
        user_a.app_data_json = repeated.clone();
        let mut user_b = new_user("user-b", "b@example.test");
        user_b.app_data_json = repeated.clone();
        store.create_users_atomically(&[user_a, user_b]).unwrap();

        let connection = store.open_connection(false).unwrap();
        let (objects, references, raw_bytes, compressed_bytes) = connection
            .query_row(
                "SELECT COUNT(*), SUM(reference_count),
                        SUM(uncompressed_size_bytes), SUM(compressed_size_bytes)
                 FROM snapshot_contents",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(1, objects);
        assert_eq!(2, references);
        assert_eq!(repeated.len() as i64, raw_bytes);
        assert!(compressed_bytes < raw_bytes / 4);
        drop(connection);
        assert_eq!(
            repeated,
            store.list_snapshot_history("user-a", 10).unwrap()[0].app_data_json
        );
        store.verify_snapshot_storage_integrity().unwrap();

        let connection = store.open_connection(false).unwrap();
        connection
            .execute("DELETE FROM users WHERE id = 'user-a'", [])
            .unwrap();
        assert_eq!(
            1,
            connection
                .query_row("SELECT reference_count FROM snapshot_contents", [], |row| {
                    row.get::<_, i64>(0)
                },)
                .unwrap()
        );
        connection
            .execute("DELETE FROM users WHERE id = 'user-b'", [])
            .unwrap();
        assert_eq!(
            0,
            connection
                .query_row("SELECT COUNT(*) FROM snapshot_contents", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap()
        );
    }

    #[test]
    fn snapshot_history_is_never_auto_pruned_and_quota_rejection_is_atomic() {
        let directory = TestDirectory::new("snapshot_quota_atomic");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        for revision in 0..3 {
            store
                .compare_and_swap_account(
                    "user-a",
                    revision,
                    &format!("{{\"revision\":{}}}", revision + 1),
                    100 + revision,
                )
                .unwrap();
        }
        let current_before = store.read_account("user-a").unwrap();
        let stats_before = store.snapshot_storage_stats("user-a").unwrap();
        assert_eq!(3, stats_before.history_rows);
        assert_eq!(0, stats_before.prune_audit_rows);

        let mut connection = store.open_connection(false).unwrap();
        let transaction = connection.transaction().unwrap();
        let error = insert_snapshot_history_with_limit(
            &transaction,
            &current_before,
            1_000,
            stats_before.charged_bytes,
            NewUserMediaPolicy::EnforceGrowth,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            StoreError::SnapshotHistoryQuotaExceeded {
                usage_bytes,
                projected_bytes,
                limit_bytes,
            } if usage_bytes == stats_before.charged_bytes
                && projected_bytes > usage_bytes
                && limit_bytes == stats_before.charged_bytes
        ));
        verify_snapshot_content_index(&transaction).unwrap();
        transaction.commit().unwrap();

        assert_eq!(current_before, store.read_account("user-a").unwrap());
        assert_eq!(
            stats_before,
            store.snapshot_storage_stats("user-a").unwrap()
        );

        // An already committed revision is a zero-increment replay. It remains
        // readable and verifiable even when a migrated account is over limit.
        let existing = store.list_snapshot_history("user-a", 10).unwrap()[0].clone();
        let replay = AccountSnapshot {
            user_id: existing.user_id,
            app_data_json: existing.app_data_json,
            revision: existing.revision,
            updated_at_epoch_millis: existing.created_at_epoch_millis,
        };
        let mut connection = store.open_connection(false).unwrap();
        let transaction = connection.transaction().unwrap();
        insert_snapshot_history_with_limit(
            &transaction,
            &replay,
            2_000,
            1,
            NewUserMediaPolicy::EnforceGrowth,
        )
        .unwrap();
        transaction.commit().unwrap();
        assert_eq!(
            stats_before,
            store.snapshot_storage_stats("user-a").unwrap()
        );
    }

    #[test]
    fn snapshot_write_space_guard_preserves_safety_reserve() {
        let payload = 1_024_u64;
        let required = SNAPSHOT_WRITE_MIN_FREE_BYTES + SNAPSHOT_WRITE_OVERHEAD_BYTES + payload * 2;
        assert!(validate_snapshot_write_capacity(payload, Some(required)).is_ok());
        assert!(matches!(
            validate_snapshot_write_capacity(payload, Some(required - 1)),
            Err(StoreError::DiskReserveExceeded {
                available_bytes,
                required_bytes,
            }) if available_bytes == required - 1 && required_bytes == required
        ));
        assert!(validate_snapshot_write_capacity(payload, None).is_ok());
    }

    #[test]
    fn schema_migration_space_guard_accounts_for_backup_and_index_rebuild() {
        let transient_payload = 32 * 1024 * 1024_u64;
        let required =
            SNAPSHOT_WRITE_MIN_FREE_BYTES + SNAPSHOT_WRITE_OVERHEAD_BYTES + transient_payload * 2;
        assert!(validate_schema_migration_capacity(transient_payload, Some(required)).is_ok());
        assert!(matches!(
            validate_schema_migration_capacity(transient_payload, Some(required - 1)),
            Err(StoreError::DiskReserveExceeded {
                available_bytes,
                required_bytes,
            }) if available_bytes == required - 1 && required_bytes == required
        ));
        assert!(validate_schema_migration_capacity(transient_payload, None).is_ok());
    }

    #[test]
    fn app_data_quota_blocks_only_positive_growth_and_preserves_recovery_paths() {
        assert!(matches!(
            validate_app_data_growth_quota_with_limit("1234", "123456", 5),
            Err(StoreError::AppDataQuotaExceeded {
                current_bytes: 4,
                projected_bytes: 6,
                limit_bytes: 5,
            })
        ));
        assert!(validate_app_data_growth_quota_with_limit("123456", "12345", 4).is_ok());
        assert!(validate_app_data_growth_quota_with_limit("123456", "123456", 4).is_ok());
    }

    #[test]
    fn restore_generation_blocks_stale_tokens_until_explicit_acknowledgement() {
        let directory = TestDirectory::new("restore_generation");
        let store = open_empty(&directory);
        let mut user = new_user("user-a", "a@example.test");
        let kept_media = b"kept-across-restore";
        user.app_data_json = format!(
            "{{\"notes\":[{{\"id\":\"note-1\",\"attachments\":[{{\"id\":\"kept-media\",\"sha256\":\"{}\",\"mimeType\":\"application/octet-stream\",\"sizeBytes\":{},\"updatedAtEpochMillis\":50}}]}}]}}",
            sha256_hex(kept_media),
            kept_media.len()
        );
        store.create_user(user).unwrap();
        let token_a = store
            .issue_token("user-a", "token-a", "phone-a", 10, 10_000)
            .unwrap();
        let token_b = store
            .issue_token("user-a", "token-b", "phone-b", 10, 10_000)
            .unwrap();
        store
            .upsert_media(
                "user-a",
                "kept-media",
                &sha256_hex(kept_media),
                "application/octet-stream",
                kept_media.len() as i64,
                kept_media,
                50,
            )
            .unwrap();
        store
            .compare_and_swap_account("user-a", 0, "{\"value\":\"newer\"}", 100)
            .unwrap();
        store.restore_account_snapshot("user-a", 0, 1, 200).unwrap();

        assert_eq!(
            RestoreBarrierState {
                current_generation: 1,
                token_last_seen_generation: 0,
                token_restore_acknowledged: false,
            },
            store.restore_barrier_state("user-a", token_a.id).unwrap()
        );
        let before = store.read_account("user-a").unwrap();
        assert!(matches!(
            store.apply_sync_request_for_generation(
                "user-a",
                token_a.id,
                0,
                "",
                "stale-generation",
                "{\"operation\":\"sync\"}",
                before.revision,
                "{\"value\":\"stale-device\"}",
                "{\"ok\":true}",
                300,
            ),
            Err(StoreError::RestoreGenerationConflict {
                expected_generation: 0,
                actual_generation: 1,
            })
        ));
        assert_eq!(before, store.read_account("user-a").unwrap());
        let stale_media = b"stale-media";
        assert!(matches!(
            store.upsert_media_with_restore_for_generation(
                "user-a",
                token_a.id,
                0,
                "",
                "stale-media",
                &sha256_hex(stale_media),
                "application/octet-stream",
                stale_media.len() as i64,
                stale_media,
                300,
                false,
            ),
            Err(StoreError::RestoreGenerationConflict {
                expected_generation: 0,
                actual_generation: 1,
            })
        ));
        assert!(matches!(
            store.delete_media_for_generation("user-a", token_a.id, 0, "", "kept-media", 301, 301,),
            Err(StoreError::RestoreGenerationConflict {
                expected_generation: 0,
                actual_generation: 1,
            })
        ));
        assert!(matches!(
            store.list_media_metadata_for_generation("user-a", token_a.id, 0, "", true),
            Err(StoreError::RestoreGenerationConflict {
                expected_generation: 0,
                actual_generation: 1,
            })
        ));
        assert!(matches!(
            store.read_media_for_generation("user-a", token_a.id, 0, "", "kept-media"),
            Err(StoreError::RestoreGenerationConflict {
                expected_generation: 0,
                actual_generation: 1,
            })
        ));
        assert!(store.read_media("user-a", "stale-media").unwrap().is_none());
        assert_eq!(
            kept_media,
            store
                .read_media("user-a", "kept-media")
                .unwrap()
                .unwrap()
                .content
                .as_slice()
        );

        let receipt = store.ensure_restore_receipt("user-a", token_a.id).unwrap();
        store
            .acknowledge_restore_generation("user-a", token_a.id, 1, &receipt.receipt)
            .unwrap();
        assert_eq!(
            1,
            store
                .restore_barrier_state("user-a", token_a.id)
                .unwrap()
                .token_last_seen_generation
        );
        assert_eq!(
            0,
            store
                .restore_barrier_state("user-a", token_b.id)
                .unwrap()
                .token_last_seen_generation
        );
        assert_eq!(
            1,
            store
                .list_media_metadata_for_generation("user-a", token_a.id, 1, "", true)
                .unwrap()
                .len()
        );
        assert_eq!(
            kept_media,
            store
                .read_media_for_generation("user-a", token_a.id, 1, "", "kept-media")
                .unwrap()
                .unwrap()
                .content
                .as_slice()
        );
        store
            .apply_sync_request_for_generation(
                "user-a",
                token_a.id,
                1,
                "",
                "acknowledged-generation",
                "{\"operation\":\"sync\"}",
                before.revision,
                &before.app_data_json,
                "{\"ok\":true}",
                301,
            )
            .unwrap();
    }

    #[test]
    fn registration_user_and_initial_token_commit_or_roll_back_together() {
        let directory = TestDirectory::new("atomic_registration");
        let store = open_empty(&directory);
        store
            .create_user(new_user("existing", "existing@example.test"))
            .unwrap();
        store
            .issue_token("existing", "duplicate-raw-token", "phone", 10, 10_000)
            .unwrap();

        let mut failed_user = new_user("failed", "failed@example.test");
        failed_user.app_data_json.clear();
        assert!(store
            .create_user_with_initial_token(
                failed_user,
                "duplicate-raw-token",
                "new phone",
                20,
                10_000,
            )
            .is_err());
        assert!(store.find_user_by_id("failed").unwrap().is_none());
        assert_eq!(1, store.stats().unwrap().users);
        assert_eq!(1, store.stats().unwrap().tokens);

        let mut registered_user = new_user("registered", "registered@example.test");
        registered_user.app_data_json.clear();
        let initial = store
            .create_user_with_initial_token(
                registered_user,
                "unique-registration-token",
                "new phone",
                30,
                10_000,
            )
            .unwrap();
        let barrier = store
            .restore_barrier_state("registered", initial.id)
            .unwrap();
        assert_eq!(0, barrier.current_generation);
        assert_eq!(0, barrier.token_last_seen_generation);
        assert!(barrier.token_restore_acknowledged);
        assert_eq!(2, store.stats().unwrap().users);
        assert_eq!(2, store.stats().unwrap().tokens);
    }

    #[test]
    fn independent_servers_isolate_same_user_id_with_stable_account_namespaces() {
        let first_directory = TestDirectory::new("server_namespace_first");
        let second_directory = TestDirectory::new("server_namespace_second");
        let first = open_empty(&first_directory);
        let second = open_empty(&second_directory);
        first
            .create_user(new_user("shared-user", "first@example.test"))
            .unwrap();
        second
            .create_user(new_user("shared-user", "second@example.test"))
            .unwrap();

        let first_identity = first.server_account_identity("shared-user").unwrap();
        let second_identity = second.server_account_identity("shared-user").unwrap();
        assert!(valid_sha256_hex(&first_identity.server_instance_id));
        assert!(valid_sha256_hex(&first_identity.account_namespace));
        assert_ne!(
            first_identity.server_instance_id,
            second_identity.server_instance_id
        );
        assert_ne!(
            first_identity.account_namespace,
            second_identity.account_namespace
        );

        let backup_path = first_directory.0.join("identity-backup.sqlite3");
        first.create_verified_backup(&backup_path, 500).unwrap();
        let backup = SqliteServerStore::open(&backup_path, None).unwrap();
        assert_eq!(
            first_identity,
            backup.server_account_identity("shared-user").unwrap()
        );
        drop(backup);
        drop(first);
        let reopened =
            SqliteServerStore::open(first_directory.0.join("server_store.sqlite3"), None).unwrap();
        assert_eq!(
            first_identity,
            reopened.server_account_identity("shared-user").unwrap()
        );
    }

    #[test]
    fn workspace_capability_hmac_matches_standard_sha256_hmac() {
        assert_eq!(
            "ca63de87f5430d8308b416d8edabab87fa747ead59566759ffb16e1aee709f7e",
            workspace_capability_hmac(
                &"01".repeat(32),
                &"02".repeat(32),
                "user-a",
                &"03".repeat(32),
                &"04".repeat(32),
                7,
            )
            .unwrap()
        );
    }

    #[test]
    fn workspace_capability_survives_token_replay_restart_and_verified_backup() {
        let directory = TestDirectory::new("workspace_capability_durability");
        let database_path = directory.0.join("server_store.sqlite3");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        store
            .create_user(new_user("user-b", "b@example.test"))
            .unwrap();
        let workspace_id = "ab".repeat(32);
        let proof = store
            .workspace_capability_proof("user-a", &workspace_id, 0)
            .unwrap();
        assert_eq!(64, proof.len());
        assert!(store
            .verify_workspace_capability("user-a", &workspace_id, 0, &proof)
            .unwrap());
        assert!(!store
            .verify_workspace_capability("user-b", &workspace_id, 0, &proof)
            .unwrap());

        let token = store
            .issue_token("user-a", "workspace-token", "phone", 20, 20_000)
            .unwrap();
        let response_json = json!({
            "ok": true,
            "workspaceId": &workspace_id,
            "workspaceProof": &proof
        })
        .to_string();
        let applied = store
            .apply_workspace_bound_sync_request_for_generation(
                "user-a",
                token.id,
                0,
                "workspace-dedup",
                "{\"operation\":\"workspace-sync\"}",
                0,
                "{\"value\":1}",
                &response_json,
                30,
            )
            .unwrap();
        assert!(matches!(applied, SyncRequestOutcome::Applied(_)));
        let replay = store
            .apply_sync_request_for_generation(
                "user-a",
                token.id,
                0,
                "",
                "workspace-dedup",
                "{\"operation\":\"workspace-sync\"}",
                1,
                "{\"value\":999}",
                "{\"ok\":false}",
                31,
            )
            .unwrap();
        assert!(matches!(replay, SyncRequestOutcome::Replayed(_)));

        let backup_path = directory.0.join("workspace-capability-backup.sqlite3");
        store.create_verified_backup(&backup_path, 40).unwrap();
        drop(store);

        let reopened = SqliteServerStore::open(&database_path, None).unwrap();
        assert!(reopened
            .verify_workspace_capability("user-a", &workspace_id, 0, &proof)
            .unwrap());
        let replay_after_restart = reopened
            .apply_sync_request_for_generation(
                "user-a",
                token.id,
                0,
                "",
                "workspace-dedup",
                "{\"operation\":\"workspace-sync\"}",
                1,
                "{\"value\":999}",
                "{\"ok\":false}",
                41,
            )
            .unwrap();
        assert_eq!(replay, replay_after_restart);
        drop(reopened);

        let backup = SqliteServerStore::open(&backup_path, None).unwrap();
        assert!(backup
            .verify_workspace_capability("user-a", &workspace_id, 0, &proof)
            .unwrap());
        let replay_from_backup = backup
            .apply_sync_request_for_generation(
                "user-a",
                token.id,
                0,
                "",
                "workspace-dedup",
                "{\"operation\":\"workspace-sync\"}",
                1,
                "{\"value\":999}",
                "{\"ok\":false}",
                42,
            )
            .unwrap();
        assert_eq!(replay, replay_from_backup);
    }

    #[test]
    fn new_token_requires_bound_receipt_and_server_generation_rollback_never_writes() {
        let directory = TestDirectory::new("restore_receipt_and_rollback");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let token = store
            .issue_token("user-a", "token-a", "phone", 10, 10_000)
            .unwrap();
        let initial = store.restore_barrier_state("user-a", token.id).unwrap();
        assert_eq!(0, initial.current_generation);
        assert!(!initial.token_restore_acknowledged);
        assert!(matches!(
            store.list_media_metadata_for_generation("user-a", token.id, 0, "", true),
            Err(StoreError::RestoreReceiptRequired {
                actual_generation: 0
            })
        ));
        assert!(matches!(
            store.read_media_for_generation("user-a", token.id, 0, "", "missing-media"),
            Err(StoreError::RestoreReceiptRequired {
                actual_generation: 0
            })
        ));
        let before = store.read_account("user-a").unwrap();
        assert!(matches!(
            store.apply_sync_request_for_generation(
                "user-a",
                token.id,
                0,
                "",
                "missing-receipt",
                "{\"operation\":\"sync\"}",
                before.revision,
                "{\"value\":1}",
                "{\"ok\":true}",
                20,
            ),
            Err(StoreError::RestoreReceiptRequired {
                actual_generation: 0
            })
        ));
        assert_eq!(before, store.read_account("user-a").unwrap());

        let receipt = store.ensure_restore_receipt("user-a", token.id).unwrap();
        assert_eq!(64, receipt.receipt.len());
        assert!(matches!(
            store.acknowledge_restore_generation("user-a", token.id, 0, "wrong"),
            Err(StoreError::RestoreReceiptRequired {
                actual_generation: 0
            })
        ));
        store
            .acknowledge_restore_generation("user-a", token.id, 0, &receipt.receipt)
            .unwrap();
        // A retry after a crash while clearing the local receipt is idempotent.
        store
            .acknowledge_restore_generation("user-a", token.id, 0, &receipt.receipt)
            .unwrap();

        let media = b"rollback-guard";
        store
            .upsert_media(
                "user-a",
                "media-a",
                &sha256_hex(media),
                "application/octet-stream",
                media.len() as i64,
                media,
                30,
            )
            .unwrap();
        let account_for_envelope = store.read_account("user-a").unwrap();
        let envelope_sha256 = account_snapshot_envelope_sha256(
            "user-a",
            &account_for_envelope.app_data_json,
            account_for_envelope.revision,
            account_for_envelope.updated_at_epoch_millis,
            3,
        );
        let connection = store.open_connection(false).unwrap();
        connection
            .execute(
                "UPDATE account_snapshots \
                 SET restore_generation = 3, envelope_sha256 = ?1 \
                 WHERE user_id = 'user-a'",
                params![envelope_sha256],
            )
            .unwrap();
        let token_before = connection
            .query_row(
                "SELECT last_seen_restore_generation, restore_acknowledged,
                        pending_restore_generation, pending_restore_receipt
                 FROM tokens WHERE id = ?1",
                params![token.id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .unwrap();
        drop(connection);
        let account_before = store.read_account("user-a").unwrap();
        assert!(matches!(
            store.apply_sync_request_for_generation(
                "user-a",
                token.id,
                5,
                &receipt.receipt,
                "server-rollback-sync",
                "{\"operation\":\"sync\"}",
                account_before.revision,
                "{\"value\":5}",
                "{\"ok\":true}",
                40,
            ),
            Err(StoreError::ServerGenerationRollback {
                client_generation: 5,
                server_generation: 3
            })
        ));
        assert!(matches!(
            store.upsert_media_with_restore_for_generation(
                "user-a",
                token.id,
                5,
                &receipt.receipt,
                "media-b",
                &sha256_hex(b"blocked"),
                "application/octet-stream",
                7,
                b"blocked",
                41,
                false,
            ),
            Err(StoreError::ServerGenerationRollback {
                client_generation: 5,
                server_generation: 3
            })
        ));
        assert!(matches!(
            store.delete_media_for_generation(
                "user-a",
                token.id,
                5,
                &receipt.receipt,
                "media-a",
                42,
                42,
            ),
            Err(StoreError::ServerGenerationRollback {
                client_generation: 5,
                server_generation: 3
            })
        ));
        assert!(matches!(
            store
                .list_media_metadata_for_generation("user-a", token.id, 5, &receipt.receipt, true,),
            Err(StoreError::ServerGenerationRollback {
                client_generation: 5,
                server_generation: 3
            })
        ));
        assert!(matches!(
            store.read_media_for_generation("user-a", token.id, 5, &receipt.receipt, "media-a",),
            Err(StoreError::ServerGenerationRollback {
                client_generation: 5,
                server_generation: 3
            })
        ));
        assert!(matches!(
            store.acknowledge_restore_generation("user-a", token.id, 5, &receipt.receipt),
            Err(StoreError::ServerGenerationRollback {
                client_generation: 5,
                server_generation: 3
            })
        ));
        assert_eq!(account_before, store.read_account("user-a").unwrap());
        assert!(store.read_media("user-a", "media-b").unwrap().is_none());
        assert_eq!(
            media.as_slice(),
            store
                .read_media("user-a", "media-a")
                .unwrap()
                .unwrap()
                .content
                .as_slice()
        );
        let connection = store.open_connection(false).unwrap();
        let token_after = connection
            .query_row(
                "SELECT last_seen_restore_generation, restore_acknowledged,
                        pending_restore_generation, pending_restore_receipt
                 FROM tokens WHERE id = ?1",
                params![token.id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(token_before, token_after);
    }

    #[test]
    fn media_history_restores_json_manifest_and_blob_after_live_content_gc() {
        let directory = TestDirectory::new("media_history_restore");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let content = b"historical-media-content";
        let historical_json = json_with_attachment("attachment-a", content);

        // Real client ordering: JSON first, media second, then a later JSON edit and delete.
        store
            .compare_and_swap_account("user-a", 0, &historical_json, 100)
            .unwrap();
        store
            .upsert_media(
                "user-a",
                "attachment-a",
                &sha256_hex(content),
                "application/octet-stream",
                content.len() as i64,
                content,
                101,
            )
            .unwrap();
        store
            .compare_and_swap_account("user-a", 1, "{\"notes\":[]}", 102)
            .unwrap();
        store
            .delete_media("user-a", "attachment-a", 103, 103)
            .unwrap();
        store
            .prune_deleted_media_content(103 + DELETED_MEDIA_CONTENT_RETENTION_MILLIS + 1)
            .unwrap();
        assert!(store
            .read_media("user-a", "attachment-a")
            .unwrap()
            .is_none());

        let restored = store.restore_account_snapshot("user-a", 1, 2, 200).unwrap();
        assert_eq!(historical_json, restored.app_data_json);
        let restored_media = store.read_media("user-a", "attachment-a").unwrap().unwrap();
        assert_eq!(content, restored_media.content.as_slice());
        assert_eq!(sha256_hex(content), restored_media.metadata.sha256);
        assert_eq!(100, restored_media.metadata.updated_at_epoch_millis);
        assert!(restored_media.metadata.deleted_at_epoch_millis.is_none());
        assert_eq!(
            vec!["attachment-a".to_string()],
            store
                .list_media_metadata("user-a", false)
                .unwrap()
                .into_iter()
                .map(|item| item.attachment_id)
                .collect::<Vec<_>>()
        );
        store.verify_snapshot_storage_integrity().unwrap();
    }

    #[test]
    fn media_history_restores_version_only_attachment_after_live_content_gc() {
        let directory = TestDirectory::new("media_history_version_only_restore");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let content = b"version-only-historical-media";
        let sha256 = sha256_hex(content);
        let historical_json = json!({
            "notes": [{
                "id": "note-a",
                "attachments": [],
                "revisions": [],
                "versions": [{
                    "id": "version-a",
                    "attachments": [{
                        "id": "version-attachment-a",
                        "sha256": sha256.clone(),
                        "mimeType": "image/png",
                        "sizeBytes": content.len(),
                        "updatedAtEpochMillis": 100
                    }]
                }]
            }]
        })
        .to_string();

        store
            .compare_and_swap_account("user-a", 0, &historical_json, 100)
            .unwrap();
        store
            .upsert_media(
                "user-a",
                "version-attachment-a",
                &sha256,
                "image/png",
                content.len() as i64,
                content,
                101,
            )
            .unwrap();
        store
            .compare_and_swap_account("user-a", 1, "{\"notes\":[]}", 102)
            .unwrap();
        store
            .delete_media("user-a", "version-attachment-a", 103, 103)
            .unwrap();
        store
            .prune_deleted_media_content(103 + DELETED_MEDIA_CONTENT_RETENTION_MILLIS + 1)
            .unwrap();
        assert!(store
            .read_media("user-a", "version-attachment-a")
            .unwrap()
            .is_none());

        let restored = store.restore_account_snapshot("user-a", 1, 2, 200).unwrap();
        assert_eq!(historical_json, restored.app_data_json);
        let restored_media = store
            .read_media("user-a", "version-attachment-a")
            .unwrap()
            .unwrap();
        assert_eq!(content, restored_media.content.as_slice());
        assert_eq!(sha256, restored_media.metadata.sha256);
        assert_eq!(100, restored_media.metadata.updated_at_epoch_millis);
        assert!(restored_media.metadata.deleted_at_epoch_millis.is_none());
        store.verify_snapshot_storage_integrity().unwrap();
    }

    #[test]
    fn media_history_restores_explicit_zero_byte_version_attachment() {
        let directory = TestDirectory::new("media_history_zero_byte_version_restore");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let content = b"";
        let sha256 = sha256_hex(content);
        let historical_json = json!({
            "notes": [{
                "id": "note-a",
                "attachments": [],
                "revisions": [],
                "versions": [{
                    "id": "version-a",
                    "attachments": [{
                        "id": "zero-byte-attachment-a",
                        "sha256": sha256.clone(),
                        "mimeType": "application/octet-stream",
                        "sizeBytes": 0,
                        "updatedAtEpochMillis": 100
                    }]
                }]
            }]
        })
        .to_string();
        store
            .compare_and_swap_account("user-a", 0, &historical_json, 100)
            .unwrap();
        store
            .upsert_media(
                "user-a",
                "zero-byte-attachment-a",
                &sha256,
                "application/octet-stream",
                0,
                content,
                101,
            )
            .unwrap();
        store
            .compare_and_swap_account("user-a", 1, "{\"notes\":[]}", 102)
            .unwrap();
        store
            .delete_media("user-a", "zero-byte-attachment-a", 103, 103)
            .unwrap();
        store
            .prune_deleted_media_content(103 + DELETED_MEDIA_CONTENT_RETENTION_MILLIS + 1)
            .unwrap();
        assert!(store
            .read_media("user-a", "zero-byte-attachment-a")
            .unwrap()
            .is_none());

        let restored = store.restore_account_snapshot("user-a", 1, 2, 200).unwrap();
        assert_eq!(historical_json, restored.app_data_json);
        let restored_media = store
            .read_media("user-a", "zero-byte-attachment-a")
            .unwrap()
            .unwrap();
        assert!(restored_media.content.is_empty());
        assert_eq!(sha256, restored_media.metadata.sha256);
        assert_eq!(0, restored_media.metadata.size_bytes);
        store.verify_snapshot_storage_integrity().unwrap();
    }

    #[test]
    fn missing_size_metadata_is_not_treated_as_an_explicit_zero_byte_attachment() {
        let directory = TestDirectory::new("media_history_missing_zero_size_metadata");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let sha256 = sha256_hex(b"");
        let historical_json = json!({
            "notes": [{
                "id": "note-a",
                "attachments": [{
                    "id": "attachment-a",
                    "sha256": sha256.clone(),
                    "mimeType": "application/octet-stream",
                    "updatedAtEpochMillis": 100
                }]
            }]
        })
        .to_string();
        seed_grandfathered_current_snapshot(&store, "user-a", 0, &historical_json, 100);
        store
            .upsert_media(
                "user-a",
                "attachment-a",
                &sha256,
                "application/octet-stream",
                0,
                b"",
                101,
            )
            .unwrap();
        store
            .compare_and_swap_account("user-a", 1, "{\"notes\":[]}", 102)
            .unwrap();
        let connection = store.open_connection(false).unwrap();
        assert_eq!(
            (
                0_i64,
                None::<String>,
                "reference_metadata_unavailable_at_snapshot".to_string(),
            ),
            connection
                .query_row(
                    "SELECT h.media_snapshot_complete, m.content_sha256, m.missing_reason
                     FROM account_snapshot_history h
                     JOIN account_snapshot_media_history m
                       ON m.user_id = h.user_id AND m.account_revision = h.revision
                     WHERE h.user_id = 'user-a' AND h.revision = 1
                       AND m.attachment_id = 'attachment-a'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap()
        );
        store.verify_snapshot_storage_integrity().unwrap();
    }

    #[test]
    fn verified_backup_rejects_a_complete_history_with_a_missing_manifest_row() {
        let directory = TestDirectory::new("media_history_missing_manifest_row");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let content = b"manifest-row-content";
        let historical_json = json_with_attachment("attachment-a", content);
        store
            .compare_and_swap_account("user-a", 0, &historical_json, 100)
            .unwrap();
        store
            .upsert_media(
                "user-a",
                "attachment-a",
                &sha256_hex(content),
                "application/octet-stream",
                content.len() as i64,
                content,
                101,
            )
            .unwrap();
        store
            .compare_and_swap_account("user-a", 1, "{\"notes\":[]}", 102)
            .unwrap();
        store.verify_snapshot_storage_integrity().unwrap();

        let connection = store.open_connection(false).unwrap();
        assert_eq!(
            1,
            connection
                .execute(
                    "DELETE FROM account_snapshot_media_history
                     WHERE user_id = 'user-a' AND account_revision = 1
                       AND attachment_id = 'attachment-a'",
                    [],
                )
                .unwrap()
        );
        drop(connection);

        for result in [
            store.validate_integrity(),
            store.verify_snapshot_storage_integrity(),
        ] {
            assert!(
                matches!(&result, Err(StoreError::Integrity(_))),
                "missing manifest row was not rejected as an integrity failure: {result:?}"
            );
        }
        let destination = directory.0.join("missing-manifest-backup.sqlite3");
        let backup_result = store.create_verified_backup(&destination, 200);
        assert!(
            matches!(&backup_result, Err(StoreError::Integrity(_))),
            "missing manifest row did not prevent verified backup: {backup_result:?}"
        );
        assert!(!destination.exists());
    }

    #[test]
    fn media_history_allows_same_content_with_current_and_revision_timestamps() {
        let directory = TestDirectory::new("media_history_revision_timestamp");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let content = b"same-content-across-note-revision";
        let sha256 = sha256_hex(content);
        let historical_json = json!({
            "notes": [{
                "id": "note-a",
                "attachments": [{
                    "id": "attachment-a",
                    "sha256": sha256.clone(),
                    "mimeType": "image/png",
                    "sizeBytes": content.len(),
                    "updatedAtEpochMillis": 200
                }],
                "revisions": [{
                    "id": "revision-a",
                    "attachments": [{
                        "id": "attachment-a",
                        "sha256": sha256,
                        "mimeType": "image/png",
                        "sizeBytes": content.len(),
                        "updatedAtEpochMillis": 100
                    }],
                    "attachmentIds": ["attachment-a"],
                    "capturedAtEpochMillis": 100
                }]
            }]
        })
        .to_string();

        store
            .compare_and_swap_account("user-a", 0, &historical_json, 200)
            .unwrap();
        store
            .upsert_media(
                "user-a",
                "attachment-a",
                &sha256,
                "image/png",
                content.len() as i64,
                content,
                201,
            )
            .unwrap();
        store
            .compare_and_swap_account("user-a", 1, "{\"notes\":[]}", 300)
            .unwrap();

        let restored = store.restore_account_snapshot("user-a", 1, 2, 400).unwrap();
        assert_eq!(historical_json, restored.app_data_json);
        let restored_media = store.read_media("user-a", "attachment-a").unwrap().unwrap();
        assert_eq!(content, restored_media.content.as_slice());
        assert_eq!(200, restored_media.metadata.updated_at_epoch_millis);
        store.verify_snapshot_storage_integrity().unwrap();
    }

    #[test]
    fn delayed_media_upload_repairs_matching_history_refcount_but_not_reused_id() {
        let directory = TestDirectory::new("media_history_delayed_repair");
        let store = open_empty(&directory);
        let expected = b"expected-content";
        let different = b"different-content";
        let mut user = new_user("user-a", "a@example.test");
        user.app_data_json = json_with_attachment("attachment-a", expected);
        store.create_user(user).unwrap();
        store
            .compare_and_swap_account("user-a", 0, "{\"notes\":[]}", 100)
            .unwrap();
        let connection = store.open_connection(false).unwrap();
        assert_eq!(
            (0_i64, 1_i64),
            connection
                .query_row(
                    "SELECT h.media_snapshot_complete, COUNT(m.attachment_id)
                     FROM account_snapshot_history h
                     JOIN account_snapshot_media_history m
                       ON m.user_id = h.user_id AND m.account_revision = h.revision
                     WHERE h.user_id = 'user-a' AND h.revision = 0",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap()
        );
        drop(connection);

        store
            .upsert_media(
                "user-a",
                "attachment-a",
                &sha256_hex(different),
                "application/octet-stream",
                different.len() as i64,
                different,
                101,
            )
            .unwrap();
        assert!(matches!(
            store.restore_account_snapshot("user-a", 0, 1, 150),
            Err(StoreError::Integrity(message)) if message.contains("media snapshot is incomplete")
        ));
        let connection = store.open_connection(false).unwrap();
        assert_eq!(
            (0_i64, None::<String>),
            connection
                .query_row(
                    "SELECT h.media_snapshot_complete, m.content_sha256
                     FROM account_snapshot_history h
                     JOIN account_snapshot_media_history m
                       ON m.user_id = h.user_id AND m.account_revision = h.revision
                     WHERE h.user_id = 'user-a' AND h.revision = 0",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap()
        );
        drop(connection);

        let mut user = new_user("user-b", "b@example.test");
        user.app_data_json = json_with_attachment("attachment-b", expected);
        store.create_user(user).unwrap();
        store
            .compare_and_swap_account("user-b", 0, "{\"notes\":[]}", 200)
            .unwrap();
        store
            .upsert_media(
                "user-b",
                "attachment-b",
                &sha256_hex(expected),
                "application/octet-stream",
                expected.len() as i64,
                expected,
                201,
            )
            .unwrap();
        let connection = store.open_connection(false).unwrap();
        assert_eq!(
            (1_i64, 1_i64),
            connection
                .query_row(
                    "SELECT h.media_snapshot_complete, c.reference_count
                     FROM account_snapshot_history h
                     JOIN account_snapshot_media_history m
                       ON m.user_id = h.user_id AND m.account_revision = h.revision
                     JOIN media_snapshot_contents c ON c.sha256 = m.content_sha256
                     WHERE h.user_id = 'user-b' AND h.revision = 0",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap()
        );
        drop(connection);
        let restored = store.restore_account_snapshot("user-b", 0, 1, 250).unwrap();
        assert_eq!(
            json_with_attachment("attachment-b", expected),
            restored.app_data_json
        );
        assert_eq!(
            100,
            store
                .read_media("user-b", "attachment-b")
                .unwrap()
                .unwrap()
                .metadata
                .updated_at_epoch_millis
        );
        store.verify_snapshot_storage_integrity().unwrap();
    }

    #[test]
    fn conflict_payload_attachment_like_fields_do_not_create_fake_media_history() {
        let directory = TestDirectory::new("media_history_scoped_references");
        let store = open_empty(&directory);
        let ghost_json = serde_json::json!({
            "notes": [],
            "syncConflictHistory": [{
                "id": "conflict-1",
                "payload": {
                    "attachmentId": "ghost-a",
                    "attachmentIds": ["ghost-b"],
                    "attachments": [{
                        "id": "ghost-c",
                        "sha256": sha256_hex(b"ghost"),
                        "mimeType": "application/octet-stream",
                        "sizeBytes": 5,
                        "updatedAtEpochMillis": 99
                    }]
                }
            }]
        })
        .to_string();
        let mut user = new_user("user-a", "a@example.test");
        user.app_data_json = ghost_json.clone();
        store.create_user(user).unwrap();
        store
            .compare_and_swap_account("user-a", 0, "{\"notes\":[]}", 100)
            .unwrap();
        let connection = store.open_connection(false).unwrap();
        assert_eq!(
            (1_i64, 0_i64),
            connection
                .query_row(
                    "SELECT h.media_snapshot_complete, COUNT(m.attachment_id)
                     FROM account_snapshot_history h
                     LEFT JOIN account_snapshot_media_history m
                       ON m.user_id = h.user_id AND m.account_revision = h.revision
                     WHERE h.user_id = 'user-a' AND h.revision = 0",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap()
        );
        drop(connection);
        let restored = store.restore_account_snapshot("user-a", 0, 1, 200).unwrap();
        assert_eq!(ghost_json, restored.app_data_json);
        assert!(store
            .list_media_metadata("user-a", true)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn media_logical_revision_accepts_large_future_values() {
        let directory = TestDirectory::new("media_future_revision");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let content = b"future-logical-revision";
        let future_revision =
            system_time_epoch_millis().saturating_add(31_i64 * 24 * 60 * 60 * 1_000);
        store
            .upsert_media(
                "user-a",
                "future-media",
                &sha256_hex(content),
                "application/octet-stream",
                content.len() as i64,
                content,
                future_revision,
            )
            .unwrap();
        assert_eq!(
            future_revision,
            store
                .read_media("user-a", "future-media")
                .unwrap()
                .unwrap()
                .metadata
                .updated_at_epoch_millis
        );
        store
            .delete_media(
                "user-a",
                "future-media",
                future_revision.saturating_add(1),
                1,
            )
            .unwrap();
    }

    #[test]
    fn unchanged_sync_only_records_bounded_dedup_receipts() {
        let directory = TestDirectory::new("dedup_cap");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let initial_history_count = store.snapshot_history_count("user-a").unwrap();
        for index in 0..=REQUEST_DEDUP_MAX_PER_USER {
            let outcome = store
                .apply_sync_request(
                    "user-a",
                    &format!("request-{index}"),
                    &format!("{{\"index\":{index}}}"),
                    0,
                    "{}",
                    &format!("{{\"ok\":true,\"index\":{index}}}"),
                    1_000 + index,
                )
                .unwrap();
            assert!(matches!(
                outcome,
                SyncRequestOutcome::Applied(SyncRequestReceipt {
                    account_revision: 0,
                    ..
                })
            ));
        }
        assert_eq!(0, store.read_account("user-a").unwrap().revision);
        assert_eq!(
            initial_history_count,
            store.snapshot_history_count("user-a").unwrap()
        );
        let connection = store.open_connection(false).unwrap();
        let receipt_count = connection
            .query_row(
                "SELECT COUNT(*) FROM request_dedup WHERE user_id = 'user-a'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(REQUEST_DEDUP_MAX_PER_USER, receipt_count);
        let oversized_response = format!(
            "{{\"payload\":\"{}\"}}",
            "x".repeat(REQUEST_DEDUP_MAX_RESPONSE_BYTES)
        );
        assert!(matches!(
            store.apply_sync_request(
                "user-a",
                "oversized-response",
                "{\"operation\":\"oversized\"}",
                0,
                "{}",
                &oversized_response,
                2_000
            ),
            Err(StoreError::Integrity(message)) if message.contains("deduplication limit")
        ));
        assert!(store
            .apply_sync_request(
                "user-a",
                &"r".repeat(257),
                "{\"operation\":\"invalid-id\"}",
                0,
                "{}",
                "{\"ok\":true}",
                2_001,
            )
            .is_err());
    }

    #[test]
    fn dedup_accepts_transport_sized_response_above_two_megabytes() {
        let directory = TestDirectory::new("dedup_large_response");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let response = format!(
            "{{\"payload\":\"{}\"}}",
            "x".repeat(2 * 1024 * 1024 + 4_096)
        );

        let outcome = store
            .apply_sync_request(
                "user-a",
                "large-response",
                "{\"operation\":\"large-response\"}",
                0,
                "{}",
                &response,
                1_000,
            )
            .unwrap();

        assert!(matches!(outcome, SyncRequestOutcome::Applied(_)));
        assert_eq!(0, store.read_account("user-a").unwrap().revision);
        let connection = store.open_connection(false).unwrap();
        let stored_bytes = connection
            .query_row(
                "SELECT length(CAST(response_json AS BLOB)) FROM request_dedup \
                 WHERE request_id = 'large-response'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(response.len() as i64, stored_bytes);
        assert!(response.len() < REQUEST_DEDUP_MAX_RESPONSE_BYTES);
    }

    #[test]
    fn dedup_total_byte_budget_evicts_oldest_without_changing_snapshot() {
        let directory = TestDirectory::new("dedup_byte_budget");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let response = format!("{{\"payload\":\"{}\"}}", "x".repeat(96));
        let byte_budget = (response.len() * 2 + 8) as i64;
        for index in 0..4 {
            store
                .apply_sync_request_with_limits(
                    "user-a",
                    &format!("budget-{index}"),
                    &format!("{{\"index\":{index}}}"),
                    0,
                    "{}",
                    &response,
                    1_000 + index,
                    100,
                    byte_budget,
                )
                .unwrap();
        }

        let account = store.read_account("user-a").unwrap();
        assert_eq!(0, account.revision);
        assert_eq!("{}", account.app_data_json);
        let connection = store.open_connection(false).unwrap();
        let (count, total_bytes, oldest_remaining, newest_remaining) = connection
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(length(CAST(response_json AS BLOB))), 0), \
                        MIN(request_id), MAX(request_id) \
                 FROM request_dedup WHERE user_id = 'user-a'",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(2, count);
        assert!(total_bytes <= byte_budget);
        assert_eq!("budget-2", oldest_remaining);
        assert_eq!("budget-3", newest_remaining);
    }

    #[test]
    fn token_last_seen_write_is_throttled_for_active_tokens() {
        let directory = TestDirectory::new("token_last_seen");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        store
            .issue_token("user-a", "active-token", "phone", 100, 2_000_000)
            .unwrap();
        assert!(matches!(
            store
                .authenticate_token("active-token", 100 + TOKEN_LAST_SEEN_WRITE_INTERVAL_MILLIS)
                .unwrap(),
            TokenAuthentication::Active(_)
        ));
        assert_eq!(
            100,
            store.list_token_metadata("user-a").unwrap()[0].last_seen_at_epoch_millis
        );
        let refreshed_at = 101 + TOKEN_LAST_SEEN_WRITE_INTERVAL_MILLIS;
        assert!(matches!(
            store
                .authenticate_token("active-token", refreshed_at)
                .unwrap(),
            TokenAuthentication::Active(_)
        ));
        assert_eq!(
            refreshed_at,
            store.list_token_metadata("user-a").unwrap()[0].last_seen_at_epoch_millis
        );
    }

    #[test]
    fn concurrent_media_hash_conflict_keeps_exactly_one_blob() {
        let directory = TestDirectory::new("media_conflict");
        let store = Arc::new(open_empty(&directory));
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let barrier = Arc::new(Barrier::new(3));
        let mut workers = Vec::new();
        for content in [b"first".as_slice(), b"second".as_slice()] {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            let content = content.to_vec();
            workers.push(thread::spawn(move || {
                let hash = sha256_hex(&content);
                barrier.wait();
                store.upsert_media(
                    "user-a",
                    "shared-id",
                    &hash,
                    "application/octet-stream",
                    content.len() as i64,
                    &content,
                    100,
                )
            }));
        }
        barrier.wait();
        let outcomes = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(1, outcomes.iter().filter(|outcome| outcome.is_ok()).count());
        assert_eq!(
            1,
            outcomes
                .iter()
                .filter(|outcome| matches!(
                    outcome,
                    Err(StoreError::Integrity(message))
                        if message == "attachment id already belongs to different content"
                ))
                .count()
        );
        let stored = store.read_media("user-a", "shared-id").unwrap().unwrap();
        assert!(stored.content == b"first" || stored.content == b"second");
        assert_eq!(sha256_hex(&stored.content), stored.metadata.sha256);
    }

    #[test]
    fn media_quota_accounts_for_new_items_and_net_update_size() {
        assert!(validate_media_quota(MAX_MEDIA_ITEMS_PER_ACCOUNT, 0, None, 1).is_err());
        assert!(validate_media_quota(1, MAX_MEDIA_ACCOUNT_BYTES, Some(1), 1).is_ok());
        assert!(validate_media_quota(1, MAX_MEDIA_ACCOUNT_BYTES, Some(1), 2).is_err());

        assert!(validate_media_retained_quota(MAX_MEDIA_ACCOUNT_BYTES, None, 1).is_err());
        assert!(validate_media_retained_quota(MAX_MEDIA_ACCOUNT_BYTES, Some(1), 1).is_ok());
        assert!(validate_media_retained_quota(MAX_MEDIA_ACCOUNT_BYTES - 1, None, 1).is_ok());
    }

    #[test]
    fn media_identity_and_metadata_limits_fail_closed_without_creating_tombstones() {
        let maximum_id = format!("a{}", "b".repeat(MAX_MEDIA_ATTACHMENT_ID_BYTES - 1));
        assert!(valid_media_attachment_id(&maximum_id));
        for invalid in [
            "".to_string(),
            format!("a{}", "b".repeat(MAX_MEDIA_ATTACHMENT_ID_BYTES)),
            "-leading".to_string(),
            "contains.dot".to_string(),
            "contains/slash".to_string(),
            "contains\\slash".to_string(),
            "contains space".to_string(),
            "unicode-附件".to_string(),
            "control\nchar".to_string(),
        ] {
            assert!(!valid_media_attachment_id(&invalid), "accepted {invalid:?}");
        }

        assert!(valid_media_mime_type("a"));
        assert!(valid_media_mime_type(
            &"x".repeat(MAX_MEDIA_MIME_TYPE_BYTES)
        ));
        assert!(!valid_media_mime_type(""));
        assert!(!valid_media_mime_type(
            &"x".repeat(MAX_MEDIA_MIME_TYPE_BYTES + 1)
        ));
        assert!(!valid_media_mime_type("image/png; charset=utf-8"));
        assert!(!valid_media_mime_type("image/png\n"));

        assert!(validate_media_identity_count(MAX_MEDIA_IDENTITIES_PER_ACCOUNT - 1, false).is_ok());
        assert!(validate_media_identity_count(MAX_MEDIA_IDENTITIES_PER_ACCOUNT, false).is_err());
        assert!(validate_media_identity_count(MAX_MEDIA_IDENTITIES_PER_ACCOUNT, true).is_ok());

        let directory = TestDirectory::new("invalid_media_identity");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        for invalid in ["-leading", "contains.dot", "unicode-附件"] {
            assert!(matches!(
                store.delete_media("user-a", invalid, 100, 100),
                Err(StoreError::Integrity(_))
            ));
        }
        let content = b"x";
        assert!(matches!(
            store.upsert_media(
                "user-a",
                &format!("a{}", "b".repeat(MAX_MEDIA_ATTACHMENT_ID_BYTES)),
                &sha256_hex(content),
                "image/png",
                1,
                content,
                100,
            ),
            Err(StoreError::Integrity(_))
        ));
        assert!(matches!(
            store.upsert_media(
                "user-a",
                "valid-id",
                &sha256_hex(content),
                &"x".repeat(MAX_MEDIA_MIME_TYPE_BYTES + 1),
                1,
                content,
                100,
            ),
            Err(StoreError::Integrity(_))
        ));
        assert!(store
            .list_media_metadata("user-a", true)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn legacy_snapshot_repair_allowance_is_exact_bounded_and_migration_only() {
        let directory = TestDirectory::new("legacy_snapshot_repair_allowance");
        let database_path = directory.0.join("server_store.sqlite3");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let legacy_source = json_with_attachment_id_count_and_padding(2, 128);
        let source_snapshot = store
            .compare_and_swap_account("user-a", 0, &legacy_source, 100)
            .unwrap();
        assert_eq!(1, source_snapshot.revision);
        let connection = store.open_connection(false).unwrap();
        downgrade_to_schema_twelve_without_v13_triggers(&connection);
        drop(connection);
        drop(store);

        let opened = SqliteServerStore::open_with_options(ServerStoreOpenOptions {
            database_path: database_path.clone(),
            legacy_json_path: None,
            now_epoch_millis: 1_318,
            legacy_token_ttl_millis: DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
        })
        .unwrap();
        let migration_backup = opened.pre_schema_migration_backup.clone().unwrap();
        assert!(opened
            .pre_schema_migration_backup
            .as_ref()
            .is_some_and(
                |backup| backup.destination.file_name().is_some_and(|name| name
                    .to_string_lossy()
                    .contains(&format!("_pre_schema_v12_to_v{SCHEMA_VERSION}_1318")))
            ));
        let migrated_source = opened.store.read_account("user-a").unwrap();
        assert_eq!(source_snapshot, migrated_source);
        let connection = opened.store.open_connection(false).unwrap();
        let seeded = connection
            .query_row(
                "SELECT source_revision, source_content_sha256, seeded_at_epoch_millis,
                        backup_file_name, backup_size_bytes, backup_sha256,
                        backup_schema_version, backup_created_at_epoch_millis
                 FROM legacy_snapshot_repair_allowances WHERE user_id = 'user-a'",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(migrated_source.revision, seeded.0);
        assert_eq!(
            sha256_hex(migrated_source.app_data_json.as_bytes()),
            seeded.1
        );
        assert_eq!(1_318, seeded.2);
        assert_eq!(
            migration_backup
                .destination
                .file_name()
                .unwrap()
                .to_string_lossy(),
            seeded.3
        );
        assert_eq!(
            i64::try_from(migration_backup.size_bytes).unwrap(),
            seeded.4
        );
        assert_eq!(migration_backup.sha256, seeded.5);
        assert_eq!(12, seeded.6);
        assert_eq!(migration_backup.created_at_epoch_millis, seeded.7);
        drop(connection);

        let same_ids_shorter = json_with_attachment_id_count_and_padding(2, 0);
        let fewer_ids_longer =
            json_with_attachment_id_count_and_padding(1, legacy_source.len() + 64);
        let bounded_replacement = json_with_attachment_id_count_and_padding(1, 0);
        assert!(same_ids_shorter.len() < legacy_source.len());
        assert!(fewer_ids_longer.len() > legacy_source.len());
        assert!(bounded_replacement.len() < legacy_source.len());
        let mut connection = opened.store.open_connection(false).unwrap();
        let transaction = connection.transaction().unwrap();
        assert!(legacy_snapshot_repair_authorization(
            &transaction,
            &database_path,
            &migrated_source,
            &same_ids_shorter,
        )
        .unwrap()
        .is_none());
        assert!(legacy_snapshot_repair_authorization(
            &transaction,
            &database_path,
            &migrated_source,
            &fewer_ids_longer,
        )
        .unwrap()
        .is_none());
        assert!(legacy_snapshot_repair_authorization(
            &transaction,
            &database_path,
            &migrated_source,
            &bounded_replacement,
        )
        .unwrap()
        .is_some());
        let mut wrong_revision = migrated_source.clone();
        wrong_revision.revision += 1;
        assert!(legacy_snapshot_repair_authorization(
            &transaction,
            &database_path,
            &wrong_revision,
            &bounded_replacement,
        )
        .unwrap()
        .is_none());
        let mut wrong_content = migrated_source.clone();
        wrong_content.app_data_json.push(' ');
        assert!(legacy_snapshot_repair_authorization(
            &transaction,
            &database_path,
            &wrong_content,
            &bounded_replacement,
        )
        .unwrap()
        .is_none());

        let history_rows_before = transaction
            .query_row(
                "SELECT COUNT(*) FROM account_snapshot_history WHERE user_id = 'user-a'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        insert_snapshot_history_for_account_mutation_with_limit(
            &transaction,
            &database_path,
            &migrated_source,
            &bounded_replacement,
            1_400,
            1,
        )
        .unwrap();
        assert_eq!(
            history_rows_before,
            transaction
                .query_row(
                    "SELECT COUNT(*) FROM account_snapshot_history WHERE user_id = 'user-a'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
        );
        assert_eq!(
            1,
            transaction
                .query_row(
                    "SELECT COUNT(*) FROM snapshot_history_prune_audit
                     WHERE user_id = 'user-a' AND revision = 1
                       AND reason = 'legacy_repair_source_preserved_in_verified_pre_schema_backup'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
        );
        let audit_details = transaction
            .query_row(
                "SELECT details_json FROM snapshot_history_prune_audit
                 WHERE user_id = 'user-a' AND revision = 1
                   AND reason = 'legacy_repair_source_preserved_in_verified_pre_schema_backup'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        let audit_details = serde_json::from_str::<serde_json::Value>(&audit_details).unwrap();
        assert_eq!(
            migration_backup
                .destination
                .file_name()
                .unwrap()
                .to_string_lossy(),
            audit_details["backupFileName"].as_str().unwrap()
        );
        assert_eq!(
            i64::try_from(migration_backup.size_bytes).unwrap(),
            audit_details["backupSizeBytes"].as_i64().unwrap()
        );
        assert_eq!(
            migration_backup.sha256,
            audit_details["backupSha256"].as_str().unwrap()
        );
        assert_eq!(12, audit_details["backupSchemaVersion"].as_i64().unwrap());
        assert_eq!(
            migration_backup.created_at_epoch_millis,
            audit_details["backupCreatedAtEpochMillis"]
                .as_i64()
                .unwrap()
        );
        assert_eq!(
            0,
            transaction
                .query_row(
                    "SELECT COUNT(*) FROM legacy_snapshot_repair_allowances
                     WHERE user_id = 'user-a'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
        );
        transaction.rollback().unwrap();

        let connection = opened.store.open_connection(false).unwrap();
        assert_eq!(
            (1_i64, 0_i64),
            connection
                .query_row(
                    "SELECT
                         (SELECT COUNT(*) FROM legacy_snapshot_repair_allowances
                          WHERE user_id = 'user-a'),
                         (SELECT COUNT(*) FROM snapshot_history_prune_audit
                          WHERE user_id = 'user-a' AND revision = 1
                            AND reason = 'legacy_repair_source_preserved_in_verified_pre_schema_backup')",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap()
        );
        drop(connection);

        let mut connection = opened.store.open_connection(false).unwrap();
        let transaction = connection.transaction().unwrap();
        insert_snapshot_history_for_account_mutation_with_limit(
            &transaction,
            &database_path,
            &migrated_source,
            &bounded_replacement,
            1_400,
            1,
        )
        .unwrap();
        let replacement_revision = next_account_revision(migrated_source.revision).unwrap();
        validate_prospective_current_archivability(
            &transaction,
            "user-a",
            replacement_revision,
            &bounded_replacement,
            1_400,
        )
        .unwrap();
        let restore_generation = restore_generation_in_transaction(&transaction, "user-a").unwrap();
        let replacement_content_sha256 = sha256_hex(bounded_replacement.as_bytes());
        let replacement_envelope_sha256 = account_snapshot_envelope_sha256(
            "user-a",
            &bounded_replacement,
            replacement_revision,
            1_400,
            restore_generation,
        );
        assert_eq!(
            1,
            transaction
                .execute(
                    "UPDATE account_snapshots
                     SET app_data_json = ?1, revision = ?2, updated_at_epoch_millis = ?3,
                         content_sha256 = ?4, envelope_sha256 = ?5
                     WHERE user_id = 'user-a' AND revision = ?6",
                    params![
                        bounded_replacement,
                        replacement_revision,
                        1_400,
                        replacement_content_sha256,
                        replacement_envelope_sha256,
                        migrated_source.revision,
                    ],
                )
                .unwrap()
        );
        replace_current_snapshot_media_identities(&transaction, "user-a", &bounded_replacement)
            .unwrap();
        transaction.commit().unwrap();

        let repaired = opened.store.read_account("user-a").unwrap();
        assert_eq!(replacement_revision, repaired.revision);
        assert_eq!(bounded_replacement, repaired.app_data_json);
        let mut connection = opened.store.open_connection(false).unwrap();
        let transaction = connection.transaction().unwrap();
        let stale_error = insert_snapshot_history_for_account_mutation_with_limit(
            &transaction,
            &database_path,
            &repaired,
            "{}",
            1_500,
            1,
        )
        .unwrap_err();
        assert!(matches!(
            stale_error,
            StoreError::SnapshotHistoryQuotaExceeded { .. }
        ));
        transaction.rollback().unwrap();

        let archived_repair = opened
            .store
            .compare_and_swap_account("user-a", repaired.revision, "{}", 1_550)
            .unwrap();
        assert_eq!(
            next_account_revision(repaired.revision).unwrap(),
            archived_repair.revision
        );
        let connection = opened.store.open_connection(false).unwrap();
        assert_eq!(
            (1_i64, 0_i64),
            connection
                .query_row(
                    "SELECT
                         (SELECT COUNT(*) FROM account_snapshot_history
                          WHERE user_id = 'user-a' AND revision = ?1),
                         (SELECT COUNT(*) FROM legacy_snapshot_repair_allowances
                          WHERE user_id = 'user-a')",
                    params![repaired.revision],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap()
        );
        drop(connection);

        opened
            .store
            .create_user(new_user("user-b", "b@example.test"))
            .unwrap();
        let post_migration = opened
            .store
            .compare_and_swap_account("user-b", 0, &legacy_source, 1_600)
            .unwrap();
        let mut connection = opened.store.open_connection(false).unwrap();
        let transaction = connection.transaction().unwrap();
        assert_eq!(
            0,
            transaction
                .query_row(
                    "SELECT COUNT(*) FROM legacy_snapshot_repair_allowances
                     WHERE user_id = 'user-b'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
        );
        let post_migration_error = insert_snapshot_history_for_account_mutation_with_limit(
            &transaction,
            &database_path,
            &post_migration,
            &bounded_replacement,
            1_700,
            1,
        )
        .unwrap_err();
        assert!(matches!(
            post_migration_error,
            StoreError::SnapshotHistoryQuotaExceeded { .. }
        ));
        transaction.rollback().unwrap();
        opened.store.validate_integrity().unwrap();
    }

    #[test]
    fn legacy_snapshot_repair_rejects_missing_changed_or_sidecar_backup() {
        for (index, damage) in ["deleted", "changed", "sidecar"].into_iter().enumerate() {
            let (_directory, database_path, opened, source, replacement) =
                open_schema_twelve_snapshot_repair_fixture(
                    &format!("legacy_repair_backup_{damage}"),
                    1_800 + index as i64,
                );
            let backup_path = opened
                .pre_schema_migration_backup
                .as_ref()
                .unwrap()
                .destination
                .clone();
            match damage {
                "deleted" => fs::remove_file(&backup_path).unwrap(),
                "changed" => {
                    OpenOptions::new()
                        .append(true)
                        .open(&backup_path)
                        .unwrap()
                        .write_all(b"changed")
                        .unwrap();
                }
                "sidecar" => {
                    fs::write(sqlite_sidecar_path(&backup_path, "-wal"), b"unexpected").unwrap();
                }
                _ => unreachable!(),
            }

            let connection = opened.store.open_connection(false).unwrap();
            let history_rows_before = connection
                .query_row(
                    "SELECT COUNT(*) FROM account_snapshot_history WHERE user_id = 'user-a'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap();
            drop(connection);
            let mut connection = opened.store.open_connection(false).unwrap();
            let transaction = connection.transaction().unwrap();
            let error = insert_snapshot_history_for_account_mutation_with_limit(
                &transaction,
                &database_path,
                &source,
                &replacement,
                1_900 + index as i64,
                1,
            )
            .unwrap_err();
            assert!(matches!(
                error,
                StoreError::SnapshotHistoryQuotaExceeded { limit_bytes: 1, .. }
            ));
            transaction.rollback().unwrap();

            let connection = opened.store.open_connection(false).unwrap();
            assert_eq!(
                (1_i64, 0_i64, history_rows_before),
                connection
                    .query_row(
                        "SELECT
                             (SELECT COUNT(*) FROM legacy_snapshot_repair_allowances
                              WHERE user_id = 'user-a'),
                             (SELECT COUNT(*) FROM snapshot_history_prune_audit
                              WHERE user_id = 'user-a'
                                AND reason = 'legacy_repair_source_preserved_in_verified_pre_schema_backup'),
                             (SELECT COUNT(*) FROM account_snapshot_history
                              WHERE user_id = 'user-a')",
                        [],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .unwrap(),
                "backup damage case {damage} changed transactional state"
            );
        }
    }

    #[test]
    fn schema_thirteen_old_allowances_are_filtered_and_bound_to_the_v14_backup() {
        let directory = TestDirectory::new("schema_13_old_repair_allowances");
        let database_path = directory.0.join("server_store.sqlite3");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        store
            .create_user(new_user("user-b", "b@example.test"))
            .unwrap();
        let source_json = json_with_attachment_id_count_and_padding(2, 128);
        let source_a = store
            .compare_and_swap_account("user-a", 0, &source_json, 100)
            .unwrap();
        let source_b = store
            .compare_and_swap_account("user-b", 0, &source_json, 101)
            .unwrap();
        let connection = store.open_connection(false).unwrap();
        connection
            .execute_batch(&format!(
                "DROP TABLE legacy_snapshot_repair_allowances;\n{}",
                legacy_snapshot_repair_allowance_v13_table_sql()
            ))
            .unwrap();
        assert_eq!(
            1,
            connection
                .execute(
                    "INSERT INTO legacy_snapshot_repair_allowances(
                         user_id, source_revision, source_content_sha256,
                         seeded_at_epoch_millis
                     ) VALUES (?1, ?2, ?3, ?4)",
                    params![
                        source_a.user_id,
                        source_a.revision,
                        sha256_hex(source_a.app_data_json.as_bytes()),
                        1_000,
                    ],
                )
                .unwrap()
        );
        assert_eq!(
            1,
            connection
                .execute(
                    "INSERT INTO legacy_snapshot_repair_allowances(
                         user_id, source_revision, source_content_sha256,
                         seeded_at_epoch_millis
                     ) VALUES (?1, ?2, ?3, ?4)",
                    params![
                        source_b.user_id,
                        source_b.revision + 1,
                        sha256_hex(source_b.app_data_json.as_bytes()),
                        1_001,
                    ],
                )
                .unwrap()
        );
        connection
            .execute_batch(
                "DROP TABLE IF EXISTS account_note_privacy; DROP TABLE IF EXISTS note_privacy_commit_witnesses;
                 DELETE FROM schema_migrations WHERE version >= 14;
                 PRAGMA user_version = 13;",
            )
            .unwrap();
        assert_eq!(13, current_schema_version(&connection).unwrap());
        assert_eq!(
            4,
            connection
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('legacy_snapshot_repair_allowances')",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
        );
        drop(connection);
        drop(store);

        let opened = SqliteServerStore::open_with_options(ServerStoreOpenOptions {
            database_path: database_path.clone(),
            legacy_json_path: None,
            now_epoch_millis: 2_000,
            legacy_token_ttl_millis: DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
        })
        .unwrap();
        let backup = opened.pre_schema_migration_backup.as_ref().unwrap();
        assert!(backup
            .destination
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains(&format!("_pre_schema_v13_to_v{SCHEMA_VERSION}_2000")));
        let connection = opened.store.open_connection(false).unwrap();
        assert_eq!(SCHEMA_VERSION, current_schema_version(&connection).unwrap());
        assert_eq!(
            9,
            connection
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('legacy_snapshot_repair_allowances')",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
        );
        let retained = connection
            .query_row(
                "SELECT source_revision, source_content_sha256, seeded_at_epoch_millis,
                        backup_file_name, backup_size_bytes, backup_sha256,
                        backup_schema_version, backup_created_at_epoch_millis
                 FROM legacy_snapshot_repair_allowances WHERE user_id = 'user-a'",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(source_a.revision, retained.0);
        assert_eq!(sha256_hex(source_a.app_data_json.as_bytes()), retained.1);
        assert_eq!(1_000, retained.2);
        assert_eq!(
            backup.destination.file_name().unwrap().to_string_lossy(),
            retained.3
        );
        assert_eq!(i64::try_from(backup.size_bytes).unwrap(), retained.4);
        assert_eq!(backup.sha256, retained.5);
        assert_eq!(13, retained.6);
        assert_eq!(backup.created_at_epoch_millis, retained.7);
        assert_eq!(
            0,
            connection
                .query_row(
                    "SELECT COUNT(*) FROM legacy_snapshot_repair_allowances
                     WHERE user_id = 'user-b'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
        );
        drop(connection);

        let replacement = json_with_attachment_id_count_and_padding(1, 0);
        let mut connection = opened.store.open_connection(false).unwrap();
        let transaction = connection.transaction().unwrap();
        assert!(legacy_snapshot_repair_authorization(
            &transaction,
            &database_path,
            &source_a,
            &replacement,
        )
        .unwrap()
        .is_some());
        transaction.rollback().unwrap();
        opened.store.validate_integrity().unwrap();
    }

    #[test]
    fn schema_thirteen_v14_table_crash_state_is_rebound_to_a_new_backup() {
        let directory = TestDirectory::new("schema_13_v14_allowance_crash_state");
        let database_path = directory.0.join("server_store.sqlite3");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let source_json = json_with_attachment_id_count_and_padding(2, 128);
        let source = store
            .compare_and_swap_account("user-a", 0, &source_json, 100)
            .unwrap();
        let source_sha256 = sha256_hex(source.app_data_json.as_bytes());
        let connection = store.open_connection(false).unwrap();
        assert_eq!(
            1,
            connection
                .execute(
                    "INSERT INTO legacy_snapshot_repair_allowances(
                         user_id, source_revision, source_content_sha256,
                         seeded_at_epoch_millis, backup_file_name, backup_size_bytes,
                         backup_sha256, backup_schema_version,
                         backup_created_at_epoch_millis
                     ) VALUES (?1, ?2, ?3, 1000, 'obsolete.sqlite3', 1, ?4, 12, 999)",
                    params![
                        source.user_id,
                        source.revision,
                        source_sha256,
                        "0".repeat(64)
                    ],
                )
                .unwrap()
        );
        connection
            .execute_batch(
                "DROP TABLE IF EXISTS account_note_privacy; DROP TABLE IF EXISTS note_privacy_commit_witnesses;
                 DELETE FROM schema_migrations WHERE version >= 14;
                 PRAGMA user_version = 13;",
            )
            .unwrap();
        drop(connection);
        drop(store);

        let opened = SqliteServerStore::open_with_options(ServerStoreOpenOptions {
            database_path,
            legacy_json_path: None,
            now_epoch_millis: 2_100,
            legacy_token_ttl_millis: DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
        })
        .unwrap();
        let backup = opened.pre_schema_migration_backup.as_ref().unwrap();
        let connection = opened.store.open_connection(false).unwrap();
        let rebound = connection
            .query_row(
                "SELECT backup_file_name, backup_size_bytes, backup_sha256,
                        backup_schema_version, backup_created_at_epoch_millis
                 FROM legacy_snapshot_repair_allowances WHERE user_id = 'user-a'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            backup.destination.file_name().unwrap().to_string_lossy(),
            rebound.0
        );
        assert_eq!(i64::try_from(backup.size_bytes).unwrap(), rebound.1);
        assert_eq!(backup.sha256, rebound.2);
        assert_eq!(13, rebound.3);
        assert_eq!(backup.created_at_epoch_millis, rebound.4);
        assert_ne!("obsolete.sqlite3", rebound.0);
        opened.store.validate_integrity().unwrap();
    }

    #[test]
    fn schema_twelve_migration_rejects_snapshot_changed_after_verified_backup() {
        let directory = TestDirectory::new("schema_12_backup_manifest_race");
        let database_path = directory.0.join("server_store.sqlite3");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let source = store
            .compare_and_swap_account("user-a", 0, "{\"beforeBackup\":true}", 100)
            .unwrap();
        let mut connection = store.open_connection(false).unwrap();
        downgrade_to_schema_twelve_without_v13_triggers(&connection);

        let backup_path = unique_pre_schema_backup_path(&database_path, 12, 14, 1_318).unwrap();
        let backup = create_verified_sqlite_backup(
            &connection,
            &database_path,
            &backup_path,
            1_318,
            Some(12),
            Some((12, 14)),
        )
        .unwrap();
        let manifest = read_verified_pre_schema_snapshot_manifest(&backup, 12).unwrap();
        assert_eq!(
            vec![LegacySnapshotRepairSource {
                user_id: "user-a".to_string(),
                revision: source.revision,
                content_sha256: sha256_hex(source.app_data_json.as_bytes()),
            }],
            manifest.sources
        );

        let changed_app_data_json = "{\"afterBackup\":true}";
        let changed_revision = next_account_revision(source.revision).unwrap();
        let changed_at_epoch_millis = 1_319;
        let restore_generation = connection
            .query_row(
                "SELECT restore_generation FROM account_snapshots WHERE user_id = 'user-a'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        let changed_content_sha256 = sha256_hex(changed_app_data_json.as_bytes());
        let changed_envelope_sha256 = account_snapshot_envelope_sha256(
            "user-a",
            changed_app_data_json,
            changed_revision,
            changed_at_epoch_millis,
            restore_generation,
        );
        assert_eq!(
            1,
            connection
                .execute(
                    "UPDATE account_snapshots
                     SET app_data_json = ?1, revision = ?2, updated_at_epoch_millis = ?3,
                         content_sha256 = ?4, envelope_sha256 = ?5
                     WHERE user_id = 'user-a' AND revision = ?6",
                    params![
                        changed_app_data_json,
                        changed_revision,
                        changed_at_epoch_millis,
                        changed_content_sha256,
                        changed_envelope_sha256,
                        source.revision,
                    ],
                )
                .unwrap()
        );

        let error = apply_schema_migrations(&mut connection, 1_320, Some(&manifest)).unwrap_err();
        assert!(matches!(
            error,
            StoreError::Integrity(message) if message.contains("diverged from the verified pre-schema backup")
        ));
        assert_eq!(12, current_schema_version(&connection).unwrap());
        assert_eq!(
            0,
            connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master
                     WHERE type = 'table' AND name = 'legacy_snapshot_repair_allowances'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
        );
        assert_eq!(
            0,
            connection
                .query_row(
                    "SELECT COUNT(*) FROM schema_migrations WHERE version = 13",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
        );
    }

    #[test]
    fn clean_schema_twelve_migrates_to_thirteen_after_verified_backup() {
        let directory = TestDirectory::new("schema_13_clean_upgrade");
        let database_path = directory.0.join("server_store.sqlite3");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let content = b"clean-upgrade";
        store
            .upsert_media(
                "user-a",
                "attachment-a",
                &sha256_hex(content),
                "application/octet-stream",
                content.len() as i64,
                content,
                100,
            )
            .unwrap();
        let connection = store.open_connection(false).unwrap();
        downgrade_to_schema_twelve_without_v13_triggers(&connection);
        connection
            .execute_batch(
                "CREATE TABLE account_snapshot_media_identities(user_id TEXT, attachment_id TEXT);
                 INSERT INTO account_snapshot_media_identities(user_id, attachment_id)
                 VALUES ('spoof-user', '-invalid');
                 INSERT INTO account_snapshot_media_history(
                     user_id, account_revision, attachment_id, content_sha256,
                     declared_sha256, mime_type, size_bytes, updated_at_epoch_millis,
                     missing_reason
                 ) VALUES (
                     'user-a', 0, 'legacy-cleanup', NULL, 'NOT-A-SHA', '', 0, 0,
                     'reference_metadata_unavailable_at_snapshot'
                 );
                 UPDATE account_snapshot_history
                 SET media_snapshot_complete = 0
                 WHERE user_id = 'user-a' AND revision = 0;
                 CREATE TRIGGER note_media_metadata_insert_guard AFTER INSERT ON users BEGIN SELECT 1; END;
                 CREATE TRIGGER note_media_metadata_update_guard AFTER INSERT ON users BEGIN SELECT 1; END;
                 CREATE TRIGGER note_media_tombstone_insert_guard AFTER INSERT ON users BEGIN SELECT 1; END;
                 CREATE TRIGGER note_media_tombstone_update_guard AFTER INSERT ON users BEGIN SELECT 1; END;
                 CREATE TRIGGER media_history_metadata_insert_guard AFTER INSERT ON users BEGIN SELECT 1; END;
                 CREATE TRIGGER media_history_metadata_update_guard
                 BEFORE UPDATE OF declared_sha256 ON account_snapshot_media_history
                 BEGIN SELECT RAISE(ABORT, 'spoofed migration blocker'); END;",
            )
            .unwrap();
        drop(connection);
        drop(store);

        let opened = SqliteServerStore::open_with_options(ServerStoreOpenOptions {
            database_path,
            legacy_json_path: None,
            now_epoch_millis: 1_313,
            legacy_token_ttl_millis: DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
        })
        .unwrap();
        assert!(opened.schema_migrated);
        let backup = opened.pre_schema_migration_backup.as_ref().unwrap();
        assert!(backup
            .destination
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains(&format!("_pre_schema_v12_to_v{SCHEMA_VERSION}_1313")));
        let backup_connection = Connection::open(&backup.destination).unwrap();
        assert_eq!(12, current_schema_version(&backup_connection).unwrap());
        assert_eq!(
            "NOT-A-SHA",
            backup_connection
                .query_row(
                    "SELECT declared_sha256 FROM account_snapshot_media_history
                     WHERE user_id = 'user-a' AND account_revision = 0
                       AND attachment_id = 'legacy-cleanup'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap()
        );
        assert_eq!(
            SCHEMA_VERSION,
            current_schema_version(&opened.store.open_connection(false).unwrap()).unwrap()
        );
        opened.store.validate_integrity().unwrap();
        let guarded = opened.store.open_connection(false).unwrap();
        assert!(guarded
            .execute(
                "UPDATE note_media SET mime_type = 'invalid mime' WHERE user_id = 'user-a'",
                [],
            )
            .is_err());
        let trigger_sql = guarded
            .query_row(
                "SELECT sql FROM sqlite_master
                 WHERE type = 'trigger' AND name = 'note_media_metadata_update_guard'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        assert!(trigger_sql.contains("invalid note_media metadata"));
        assert!(!trigger_sql.contains("ON users"));
        assert_eq!(
            0,
            guarded
                .query_row(
                    "SELECT COUNT(*) FROM account_snapshot_media_identities
                     WHERE user_id = 'spoof-user'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
        );
        // Format 16 removes manifest rows that this fully resolved historical
        // JSON never referenced; the verified pre-migration backup above keeps
        // the original evidence.
        assert_eq!(
            0_i64,
            guarded
                .query_row(
                    "SELECT COUNT(*) FROM account_snapshot_media_history
                     WHERE user_id = 'user-a' AND account_revision = 0
                       AND attachment_id = 'legacy-cleanup'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
        );
    }

    #[test]
    fn schema_twelve_reconciliation_drops_spoofed_media_guard_before_writes() {
        let directory = TestDirectory::new("schema_12_spoofed_guard");
        let database_path = directory.0.join("server_store.sqlite3");
        let store = open_empty(&directory);
        let mut user = new_user("user-a", "a@example.test");
        user.app_data_json = json_with_attachment("attachment-a", b"missing-content");
        store.create_user(user).unwrap();
        let connection = store.open_connection(false).unwrap();
        downgrade_to_schema_twelve_without_v13_triggers(&connection);
        connection
            .execute_batch(
                "DELETE FROM account_snapshot_media_history
                 WHERE user_id = 'user-a' AND account_revision = 0;
                 DROP TABLE IF EXISTS account_note_privacy; DROP TABLE IF EXISTS note_privacy_commit_witnesses;
                 DELETE FROM schema_migrations WHERE version >= 12;
                 PRAGMA user_version = 11;
                 CREATE TRIGGER media_history_metadata_insert_guard
                 BEFORE INSERT ON account_snapshot_media_history
                 BEGIN SELECT RAISE(ABORT, 'spoofed reconciliation blocker'); END;",
            )
            .unwrap();
        drop(connection);
        drop(store);

        let opened = SqliteServerStore::open_with_options(ServerStoreOpenOptions {
            database_path,
            legacy_json_path: None,
            now_epoch_millis: 1_317,
            legacy_token_ttl_millis: DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
        })
        .unwrap();
        opened.store.validate_integrity().unwrap();
        let connection = opened.store.open_connection(false).unwrap();
        assert_eq!(
            1,
            connection
                .query_row(
                    "SELECT COUNT(*) FROM account_snapshot_media_history
                     WHERE user_id = 'user-a' AND account_revision = 0
                       AND attachment_id = 'attachment-a'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
        );
        assert_eq!(
            0,
            connection
                .query_row(
                    "SELECT COUNT(*) FROM legacy_snapshot_repair_allowances",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
        );
    }

    #[test]
    fn schema_twelve_reconciliation_marks_global_quota_capture_unavailable() {
        let directory = TestDirectory::new("schema_12_global_overage_reconciliation");
        let store = open_empty(&directory);
        let mut connection = store.open_connection(false).unwrap();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let content = b"grandfathered-overage";
        let capture = ensure_media_snapshot_content_with_limit(
            &transaction,
            &sha256_hex(content),
            content,
            1_313,
            0,
        );
        assert!(matches!(
            &capture,
            Err(StoreError::MediaServerRetainedQuotaExceeded { .. })
        ));
        assert!(!migration_media_snapshot_content_available(capture).unwrap());
        assert_eq!(
            0,
            transaction
                .query_row("SELECT COUNT(*) FROM media_snapshot_contents", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap()
        );
        transaction.rollback().unwrap();
    }

    #[test]
    fn schema_thirteen_reopen_rejects_same_name_noop_trigger() {
        let directory = TestDirectory::new("schema_13_noop_trigger_reopen");
        let database_path = directory.0.join("server_store.sqlite3");
        let store = open_empty(&directory);
        let connection = store.open_connection(false).unwrap();
        connection
            .execute_batch(
                "DROP TRIGGER note_media_metadata_update_guard;
                 CREATE TRIGGER note_media_metadata_update_guard
                 AFTER INSERT ON users BEGIN SELECT 1; END;",
            )
            .unwrap();
        drop(connection);
        drop(store);

        assert!(matches!(
            SqliteServerStore::open_with_options(ServerStoreOpenOptions {
                database_path,
                legacy_json_path: None,
                now_epoch_millis: 1_316,
                legacy_token_ttl_millis: DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
            }),
            Err(StoreError::Integrity(message))
                if message.contains("trigger definition diverged")
        ));
    }

    #[test]
    fn schema_thirteen_reopen_rejects_same_name_unconstrained_identity_table() {
        let directory = TestDirectory::new("schema_13_spoofed_identity_table");
        let database_path = directory.0.join("server_store.sqlite3");
        let store = open_empty(&directory);
        let connection = store.open_connection(false).unwrap();
        connection
            .execute_batch(
                "DROP TABLE account_snapshot_media_identities;
                 CREATE TABLE account_snapshot_media_identities(
                     user_id TEXT,
                     attachment_id TEXT
                 );",
            )
            .unwrap();
        drop(connection);
        drop(store);

        assert!(matches!(
            SqliteServerStore::open_with_options(ServerStoreOpenOptions {
                database_path,
                legacy_json_path: None,
                now_epoch_millis: 1_318,
                legacy_token_ttl_millis: DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
            }),
            Err(StoreError::Integrity(message))
                if message.contains("identity table is missing or divergent")
        ));
    }

    #[test]
    fn current_identity_index_verification_rejects_rows_without_a_snapshot() {
        let directory = TestDirectory::new("orphan_current_identity");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let connection = store.open_connection(false).unwrap();
        connection
            .execute(
                "INSERT INTO account_snapshot_media_identities(user_id, attachment_id)
                 VALUES ('user-a', 'orphan-id')",
                [],
            )
            .unwrap();
        connection
            .execute("DELETE FROM account_snapshots WHERE user_id = 'user-a'", [])
            .unwrap();
        assert!(matches!(
            verify_current_snapshot_media_identity_index(&connection),
            Err(StoreError::Integrity(message))
                if message.contains("without a snapshot")
        ));
    }

    #[test]
    fn invalid_schema_twelve_media_fails_after_preserving_exact_backup() {
        let directory = TestDirectory::new("schema_13_invalid_upgrade");
        let database_path = directory.0.join("server_store.sqlite3");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let content = b"invalid-upgrade";
        store
            .upsert_media(
                "user-a",
                "attachment-a",
                &sha256_hex(content),
                "application/octet-stream",
                content.len() as i64,
                content,
                100,
            )
            .unwrap();
        let connection = store.open_connection(false).unwrap();
        downgrade_to_schema_twelve_without_v13_triggers(&connection);
        assert_eq!(
            1,
            connection
                .execute(
                    "UPDATE note_media SET mime_type = 'invalid mime' WHERE user_id = 'user-a'",
                    [],
                )
                .unwrap()
        );
        drop(connection);
        drop(store);

        let result = SqliteServerStore::open_with_options(ServerStoreOpenOptions {
            database_path: database_path.clone(),
            legacy_json_path: None,
            now_epoch_millis: 1_314,
            legacy_token_ttl_millis: DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
        });
        assert!(matches!(
            result,
            Err(StoreError::Integrity(message))
                if message.contains("note_media metadata")
        ));
        let repeated_result = SqliteServerStore::open_with_options(ServerStoreOpenOptions {
            database_path: database_path.clone(),
            legacy_json_path: None,
            now_epoch_millis: 1_315,
            legacy_token_ttl_millis: DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
        });
        assert!(matches!(
            repeated_result,
            Err(StoreError::Integrity(message))
                if message.contains("note_media metadata")
        ));

        let live = Connection::open(&database_path).unwrap();
        assert_eq!(12, current_schema_version(&live).unwrap());
        assert_eq!(
            "invalid mime",
            live.query_row(
                "SELECT mime_type FROM note_media WHERE user_id = 'user-a'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
        );
        let backups = fs::read_dir(&directory.0)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.contains(&format!("_pre_schema_v12_to_v{SCHEMA_VERSION}_"))
                    })
            })
            .collect::<Vec<_>>();
        assert_eq!(1, backups.len());
        assert!(backups[0]
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains(&format!("_pre_schema_v12_to_v{SCHEMA_VERSION}_1314")));
        let backup = Connection::open(&backups[0]).unwrap();
        assert_eq!(12, current_schema_version(&backup).unwrap());
        verify_quick_check(&backup).unwrap();
        verify_integrity_check(&backup).unwrap();
        verify_foreign_keys(&backup).unwrap();
        assert_eq!(
            "invalid mime",
            backup
                .query_row(
                    "SELECT mime_type FROM note_media WHERE user_id = 'user-a'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap()
        );
    }

    #[test]
    fn schema_thirteen_audits_history_metadata_and_triggers_block_new_corruption() {
        let directory = TestDirectory::new("schema_13_history_metadata");
        let database_path = directory.0.join("server_store.sqlite3");
        let store = open_empty(&directory);
        let content = b"history-metadata";
        let mut user = new_user("user-a", "a@example.test");
        user.app_data_json = json_with_attachment("attachment-a", content);
        store.create_user(user).unwrap();
        let connection = store.open_connection(false).unwrap();
        assert!(connection
            .execute(
                "UPDATE account_snapshot_media_history
                 SET attachment_id = '-invalid'
                 WHERE user_id = 'user-a'",
                [],
            )
            .is_err());
        downgrade_to_schema_twelve_without_v13_triggers(&connection);
        assert_eq!(
            1,
            connection
                .execute(
                    "UPDATE account_snapshot_media_history
                     SET attachment_id = '-invalid'
                     WHERE user_id = 'user-a'",
                    [],
                )
                .unwrap()
        );
        drop(connection);
        drop(store);

        assert!(matches!(
            SqliteServerStore::open_with_options(ServerStoreOpenOptions {
                database_path: database_path.clone(),
                legacy_json_path: None,
                now_epoch_millis: 1_315,
                legacy_token_ttl_millis: DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
            }),
            Err(StoreError::Integrity(message))
                if message.contains("media history metadata")
        ));
        let live = Connection::open(database_path).unwrap();
        assert_eq!(12, current_schema_version(&live).unwrap());
        assert_eq!(
            "-invalid",
            live.query_row(
                "SELECT attachment_id FROM account_snapshot_media_history
                 WHERE user_id = 'user-a'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
        );
    }

    #[test]
    fn global_media_projection_is_exact_and_grandfathers_only_non_growth() {
        assert!(validate_media_server_retained_growth_with_limit(9, None, 1, 10).is_ok());
        assert!(matches!(
            validate_media_server_retained_growth_with_limit(10, None, 1, 10),
            Err(StoreError::MediaServerRetainedQuotaExceeded {
                usage_bytes: 10,
                projected_bytes: 11,
                limit_bytes: 10,
            })
        ));
        assert!(validate_media_server_retained_growth_with_limit(11, Some(1), 1, 10).is_ok());
        assert!(validate_media_server_retained_growth_with_limit(11, Some(2), 1, 10).is_ok());
        assert!(validate_media_retained_quota(MAX_MEDIA_ACCOUNT_BYTES + 1, Some(1), 1).is_ok());

        let required = SNAPSHOT_WRITE_MIN_FREE_BYTES
            + SNAPSHOT_WRITE_OVERHEAD_BYTES
            + 2 * MAX_MEDIA_BYTES as u64;
        assert!(validate_media_write_capacity(MAX_MEDIA_BYTES as u64, Some(required)).is_ok());
        assert!(matches!(
            validate_media_write_capacity(MAX_MEDIA_BYTES as u64, Some(required - 1)),
            Err(StoreError::DiskReserveExceeded {
                available_bytes,
                required_bytes,
            }) if available_bytes == required - 1 && required_bytes == required
        ));
    }

    #[test]
    fn concurrent_cross_account_media_writes_share_one_global_budget() {
        let directory = TestDirectory::new("global_media_concurrency");
        let store = Arc::new(open_empty(&directory));
        store
            .create_users_atomically(&[
                new_user("user-a", "a@example.test"),
                new_user("user-b", "b@example.test"),
            ])
            .unwrap();
        let barrier = Arc::new(Barrier::new(3));
        let mut workers = Vec::new();
        for (user_id, attachment_id) in [("user-a", "attachment-a"), ("user-b", "attachment-b")] {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            workers.push(thread::spawn(move || -> StoreResult<()> {
                barrier.wait();
                let mut connection = store.open_connection(false)?;
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let current = global_media_retained_bytes(&transaction)?;
                validate_media_server_retained_growth_with_limit(current, None, 1, 1)?;
                let content = b"x";
                transaction.execute(
                    "INSERT INTO note_media(
                         user_id, attachment_id, sha256, mime_type, size_bytes, content,
                         updated_at_epoch_millis, deleted_at_epoch_millis
                     ) VALUES (?1, ?2, ?3, 'application/octet-stream', 1, ?4, 100, NULL)",
                    params![user_id, attachment_id, sha256_hex(content), content],
                )?;
                transaction.commit()?;
                Ok(())
            }));
        }
        barrier.wait();
        let outcomes = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(1, outcomes.iter().filter(|outcome| outcome.is_ok()).count());
        assert_eq!(
            1,
            outcomes
                .iter()
                .filter(|outcome| matches!(
                    outcome,
                    Err(StoreError::MediaServerRetainedQuotaExceeded {
                        usage_bytes: 1,
                        projected_bytes: 2,
                        limit_bytes: 1,
                    })
                ))
                .count()
        );
        let connection = store.open_connection(false).unwrap();
        assert_eq!(
            (1_i64, 1_i64),
            connection
                .query_row(
                    "SELECT COUNT(*), COALESCE(SUM(size_bytes), 0) FROM note_media",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap()
        );
    }

    #[test]
    fn media_snapshot_content_and_identity_use_global_limits() {
        let directory = TestDirectory::new("global_media_history_limit");
        let store = open_empty(&directory);
        let mut user_a = new_user("user-a", "a@example.test");
        user_a.app_data_json = json_with_attachment("history-only", b"missing-history-content");
        store
            .create_users_atomically(&[user_a, new_user("user-b", "b@example.test")])
            .unwrap();
        let mut connection = store.open_connection(false).unwrap();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let content = b"history-byte";
        assert!(matches!(
            ensure_media_snapshot_content_with_limit(
                &transaction,
                &sha256_hex(content),
                content,
                100,
                (content.len() - 1) as i64,
            ),
            Err(StoreError::MediaServerRetainedQuotaExceeded { .. })
        ));
        assert_eq!(
            0,
            transaction
                .query_row("SELECT COUNT(*) FROM media_snapshot_contents", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap()
        );
        assert!(matches!(
            validate_media_identity_quota_with_limits(
                &transaction,
                "user-b",
                "attachment-b",
                10,
                1,
            ),
            Err(StoreError::MediaServerIdentityQuotaExceeded {
                usage_items: 1,
                projected_items: 2,
                limit_items: 1,
            })
        ));
        validate_media_identity_quota_with_limits(&transaction, "user-a", "history-only", 1, 1)
            .unwrap();
        assert!(matches!(
            validate_media_identity_batch_quota_with_limits(
                &transaction,
                "user-a",
                &HashSet::from(["history-only".to_string(), "new-history".to_string()]),
                1,
                10,
            ),
            Err(StoreError::MediaAccountIdentityQuotaExceeded {
                usage_items: 1,
                projected_items: 2,
                limit_items: 1,
            })
        ));
        transaction.rollback().unwrap();
    }

    #[test]
    fn incoming_current_snapshot_cannot_defer_an_identity_overage_to_next_sync() {
        let directory = TestDirectory::new("incoming_current_identity_limit");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let oversized =
            json_with_attachment_id_count((MAX_MEDIA_IDENTITIES_PER_ACCOUNT + 1) as usize);
        assert!(matches!(
            store.compare_and_swap_account("user-a", 0, &oversized, 100),
            Err(StoreError::MediaAccountIdentityQuotaExceeded {
                usage_items: 0,
                projected_items,
                limit_items: MAX_MEDIA_IDENTITIES_PER_ACCOUNT,
            }) if projected_items == MAX_MEDIA_IDENTITIES_PER_ACCOUNT + 1
        ));
        let current = store.read_account("user-a").unwrap();
        assert_eq!(0, current.revision);
        assert_eq!("{}", current.app_data_json);
        let connection = store.open_connection(false).unwrap();
        assert_eq!(
            0,
            connection
                .query_row(
                    "SELECT COUNT(*) FROM account_snapshot_media_history
                     WHERE user_id = 'user-a'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
        );
    }

    #[test]
    fn atomic_sync_identity_overage_rolls_back_snapshot_history_and_receipt() {
        let directory = TestDirectory::new("atomic_sync_current_identity_limit");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let oversized =
            json_with_attachment_id_count((MAX_MEDIA_IDENTITIES_PER_ACCOUNT + 1) as usize);

        assert!(matches!(
            store.apply_sync_request(
                "user-a",
                "oversized-current-sync",
                "{\"operation\":\"oversized-current-sync\"}",
                0,
                &oversized,
                "{\"ok\":true}",
                100,
            ),
            Err(StoreError::MediaAccountIdentityQuotaExceeded {
                usage_items: 0,
                projected_items,
                limit_items: MAX_MEDIA_IDENTITIES_PER_ACCOUNT,
            }) if projected_items == MAX_MEDIA_IDENTITIES_PER_ACCOUNT + 1
        ));

        let current = store.read_account("user-a").unwrap();
        assert_eq!(0, current.revision);
        assert_eq!("{}", current.app_data_json);
        let connection = store.open_connection(false).unwrap();
        for (table, count) in [
            ("request_dedup", 0_i64),
            ("account_snapshot_media_identities", 0_i64),
            ("account_snapshot_media_history", 0_i64),
        ] {
            assert_eq!(
                count,
                connection
                    .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .unwrap(),
                "unexpected rows remained in {table}"
            );
        }
    }

    #[test]
    fn schema_thirteen_grandfathers_current_identity_overage_but_rejects_new_growth() {
        let directory = TestDirectory::new("schema_13_current_identity_overage");
        let database_path = directory.0.join("server_store.sqlite3");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();

        let oversized =
            json_with_attachment_id_count((MAX_MEDIA_IDENTITIES_PER_ACCOUNT + 1) as usize);
        let oversized_sha256 = sha256_hex(oversized.as_bytes());
        let oversized_envelope = account_snapshot_envelope_sha256("user-a", &oversized, 1, 20, 0);
        let connection = store.open_connection(false).unwrap();
        downgrade_to_schema_twelve_without_v13_triggers(&connection);
        assert_eq!(
            1,
            connection
                .execute(
                    "UPDATE account_snapshots
                     SET app_data_json = ?1, revision = 1, updated_at_epoch_millis = 20,
                         content_sha256 = ?2, envelope_sha256 = ?3
                     WHERE user_id = 'user-a'",
                    params![oversized, oversized_sha256, oversized_envelope],
                )
                .unwrap()
        );
        drop(connection);
        drop(store);

        let opened = SqliteServerStore::open_with_options(ServerStoreOpenOptions {
            database_path,
            legacy_json_path: None,
            now_epoch_millis: 1_320,
            legacy_token_ttl_millis: DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
        })
        .unwrap();
        assert!(opened.schema_migrated);
        let connection = opened.store.open_connection(false).unwrap();
        assert_eq!(
            MAX_MEDIA_IDENTITIES_PER_ACCOUNT + 1,
            connection
                .query_row(
                    "SELECT COUNT(*) FROM account_snapshot_media_identities
                     WHERE user_id = 'user-a'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
        );
        drop(connection);

        let subset = json_with_attachment_id_count(MAX_MEDIA_IDENTITIES_PER_ACCOUNT as usize);
        let cleaned = opened
            .store
            .compare_and_swap_account("user-a", 1, &subset, 100)
            .unwrap();
        assert_eq!(2, cleaned.revision);
        assert_eq!(subset, cleaned.app_data_json);

        let growth = oversized.replacen(
            &format!("attachment-{MAX_MEDIA_IDENTITIES_PER_ACCOUNT}"),
            "attachment-new",
            1,
        );
        assert_ne!(growth, oversized);
        assert!(matches!(
            opened
                .store
                .compare_and_swap_account("user-a", 2, &growth, 200),
            Err(StoreError::MediaAccountIdentityQuotaExceeded {
                usage_items,
                projected_items,
                limit_items: MAX_MEDIA_IDENTITIES_PER_ACCOUNT,
            }) if usage_items == MAX_MEDIA_IDENTITIES_PER_ACCOUNT + 1
                && projected_items == MAX_MEDIA_IDENTITIES_PER_ACCOUNT + 2
        ));

        let current = opened.store.read_account("user-a").unwrap();
        assert_eq!(2, current.revision);
        assert_eq!(subset, current.app_data_json);
        let connection = opened.store.open_connection(false).unwrap();
        assert_eq!(
            (MAX_MEDIA_IDENTITIES_PER_ACCOUNT, 0_i64, 0_i64),
            connection
                .query_row(
                    "SELECT
                         (SELECT COUNT(*) FROM account_snapshot_media_identities
                          WHERE user_id = 'user-a'),
                         (SELECT COUNT(*) FROM account_snapshot_media_identities
                          WHERE user_id = 'user-a' AND attachment_id = 'attachment-new'),
                         (SELECT COUNT(*) FROM account_snapshot_history
                          WHERE user_id = 'user-a' AND revision = 2)",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap()
        );
    }

    #[test]
    fn concurrent_public_registrations_cannot_cross_account_cap() {
        let directory = TestDirectory::new("registration_cap_concurrency");
        let store = Arc::new(open_empty(&directory));
        let users = (0..MAX_REGISTERED_ACCOUNTS - 1)
            .map(|index| {
                new_user(
                    &format!("prefill-{index}"),
                    &format!("prefill-{index}@example.test"),
                )
            })
            .collect::<Vec<_>>();
        store.create_users_atomically(&users).unwrap();
        let barrier = Arc::new(Barrier::new(3));
        let mut workers = Vec::new();
        for index in 0..2 {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            workers.push(thread::spawn(move || {
                barrier.wait();
                let mut user = new_user(
                    &format!("registrant-{index}"),
                    &format!("registrant-{index}@example.test"),
                );
                user.app_data_json = String::new();
                store.create_user_with_initial_pending_token(
                    user,
                    &format!("registration-token-{index}"),
                    "test-device",
                    100,
                    200,
                )
            }));
        }
        barrier.wait();
        let outcomes = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(1, outcomes.iter().filter(|outcome| outcome.is_ok()).count());
        assert_eq!(
            1,
            outcomes
                .iter()
                .filter(|outcome| matches!(
                    outcome,
                    Err(StoreError::RegisteredAccountQuotaExceeded {
                        current_accounts,
                        limit_accounts,
                    }) if *current_accounts == MAX_REGISTERED_ACCOUNTS
                        && *limit_accounts == MAX_REGISTERED_ACCOUNTS
                ))
                .count()
        );
        assert_eq!(MAX_REGISTERED_ACCOUNTS, store.stats().unwrap().users);
    }

    #[test]
    fn capacity_pressure_never_reclaims_an_explicitly_logged_out_initial_registration() {
        let directory = TestDirectory::new("explicit_logout_registration_reclaim");
        let store = open_empty(&directory);
        let created_at = 10_000_i64;
        let expires_at = created_at.saturating_add(INITIAL_REGISTRATION_ACTIVATION_LEASE_MILLIS);
        store
            .create_user_with_initial_pending_token(
                pending_registration_user(
                    "logged-out-registration",
                    "logged-out-registration@example.test",
                    created_at,
                ),
                "logged-out-registration-token",
                "phone",
                created_at,
                expires_at,
            )
            .unwrap();
        let natural_tombstone_token = store
            .issue_pending_token(
                "logged-out-registration",
                "natural-tombstone-token",
                "phone",
                created_at,
                expires_at,
            )
            .unwrap();
        let connection = store.open_connection(false).unwrap();
        assert_eq!(
            1,
            connection
                .execute(
                    "UPDATE tokens SET restore_acknowledged = 1 WHERE id = ?1",
                    params![natural_tombstone_token.id],
                )
                .unwrap()
        );
        drop(connection);
        assert_eq!(2, store.cleanup_expired_pending_tokens(expires_at).unwrap());
        let logout_at = expires_at.saturating_add(1);
        assert_eq!(
            Some("logged-out-registration".to_string()),
            store
                .revoke_token_for_logout("logged-out-registration-token", logout_at)
                .unwrap()
        );

        let prefill = (0..MAX_REGISTERED_ACCOUNTS - 1)
            .map(|index| {
                new_user(
                    &format!("logout-prefill-{index}"),
                    &format!("logout-prefill-{index}@example.test"),
                )
            })
            .collect::<Vec<_>>();
        store.create_users_atomically(&prefill).unwrap();
        let replacement_created = expires_at
            .saturating_add(INITIAL_REGISTRATION_RECOVERY_WINDOW_MILLIS)
            .saturating_add(ABANDONED_REGISTRATION_SAFETY_MARGIN_MILLIS)
            .saturating_add(1);
        assert!(matches!(
            store.create_user_with_initial_pending_token(
                pending_registration_user(
                    "logout-replacement",
                    "logout-replacement@example.test",
                    replacement_created,
                ),
                "logout-replacement-token",
                "phone",
                replacement_created,
                replacement_created
                    .saturating_add(INITIAL_REGISTRATION_ACTIVATION_LEASE_MILLIS),
            ),
            Err(StoreError::RegisteredAccountQuotaExceeded {
                current_accounts,
                limit_accounts,
            }) if current_accounts == MAX_REGISTERED_ACCOUNTS
                && limit_accounts == MAX_REGISTERED_ACCOUNTS
        ));
        assert!(store
            .find_user_by_id("logged-out-registration")
            .unwrap()
            .is_some());
        assert!(store
            .find_user_by_id("logout-replacement")
            .unwrap()
            .is_none());
        let tokens = store
            .list_token_metadata("logged-out-registration")
            .unwrap();
        assert_eq!(2, tokens.len());
        assert_eq!(Some(logout_at), tokens[0].revoked_at_epoch_millis);
        assert_eq!(Some(expires_at), tokens[1].revoked_at_epoch_millis);
        assert_eq!(MAX_REGISTERED_ACCOUNTS, store.stats().unwrap().users);
    }

    #[test]
    fn abandoned_registration_reclaim_deletes_distinct_users_with_multiple_tokens() {
        let directory = TestDirectory::new("registration_reclaim_distinct_users");
        let store = open_empty(&directory);
        let first_created = 10_000_i64;
        let second_created = first_created.saturating_add(1);
        let first_expiry =
            first_created.saturating_add(INITIAL_REGISTRATION_ACTIVATION_LEASE_MILLIS);
        let second_expiry =
            second_created.saturating_add(INITIAL_REGISTRATION_ACTIVATION_LEASE_MILLIS);
        store
            .create_user_with_initial_pending_token(
                pending_registration_user(
                    "multi-token-first",
                    "multi-token-first@example.test",
                    first_created,
                ),
                "multi-token-first-primary",
                "phone",
                first_created,
                first_expiry,
            )
            .unwrap();
        let duplicate = store
            .issue_pending_token(
                "multi-token-first",
                "multi-token-first-secondary",
                "phone",
                first_created,
                first_expiry,
            )
            .unwrap();
        let connection = store.open_connection(false).unwrap();
        assert_eq!(
            1,
            connection
                .execute(
                    "UPDATE tokens SET restore_acknowledged = 1 WHERE id = ?1",
                    params![duplicate.id],
                )
                .unwrap()
        );
        drop(connection);
        store
            .create_user_with_initial_pending_token(
                pending_registration_user(
                    "multi-token-second",
                    "multi-token-second@example.test",
                    second_created,
                ),
                "multi-token-second-primary",
                "phone",
                second_created,
                second_expiry,
            )
            .unwrap();
        assert_eq!(
            3,
            store.cleanup_expired_pending_tokens(second_expiry).unwrap()
        );

        let reclaim_at = second_expiry
            .saturating_add(INITIAL_REGISTRATION_RECOVERY_WINDOW_MILLIS)
            .saturating_add(ABANDONED_REGISTRATION_SAFETY_MARGIN_MILLIS)
            .saturating_add(1);
        let mut connection = store.open_connection(false).unwrap();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert_eq!(
            2,
            reclaim_abandoned_initial_registrations(&transaction, reclaim_at, 2).unwrap()
        );
        transaction.commit().unwrap();
        assert!(store
            .find_user_by_id("multi-token-first")
            .unwrap()
            .is_none());
        assert!(store
            .find_user_by_id("multi-token-second")
            .unwrap()
            .is_none());
    }

    #[test]
    fn capacity_pressure_reclaims_only_expired_empty_initial_registrations() {
        let directory = TestDirectory::new("abandoned_registration_reclaim");
        let store = open_empty(&directory);
        let first_created = 10_000_i64;
        for index in 0..MAX_REGISTERED_ACCOUNTS {
            let created_at = first_created + index;
            store
                .create_user_with_initial_pending_token(
                    pending_registration_user(
                        &format!("pending-{index}"),
                        &format!("pending-{index}@example.test"),
                        created_at,
                    ),
                    &format!("pending-token-{index}"),
                    "phone",
                    created_at,
                    created_at + INITIAL_REGISTRATION_ACTIVATION_LEASE_MILLIS,
                )
                .unwrap();
        }
        let earliest_expiry = first_created + INITIAL_REGISTRATION_ACTIVATION_LEASE_MILLIS;
        let latest_expiry = first_created + MAX_REGISTERED_ACCOUNTS - 1
            + INITIAL_REGISTRATION_ACTIVATION_LEASE_MILLIS;
        let before_recovery_cutoff = earliest_expiry
            + INITIAL_REGISTRATION_RECOVERY_WINDOW_MILLIS
            + ABANDONED_REGISTRATION_SAFETY_MARGIN_MILLIS
            - 1;
        assert!(matches!(
            store.create_user_with_initial_pending_token(
                pending_registration_user(
                    "too-early",
                    "too-early@example.test",
                    before_recovery_cutoff,
                ),
                "too-early-token",
                "phone",
                before_recovery_cutoff,
                before_recovery_cutoff + INITIAL_REGISTRATION_ACTIVATION_LEASE_MILLIS,
            ),
            Err(StoreError::RegisteredAccountQuotaExceeded { .. })
        ));

        let cleanup_at = latest_expiry + PENDING_TOKEN_TOMBSTONE_RETENTION_MILLIS + 1;
        store.cleanup_expired_pending_tokens(cleanup_at).unwrap();
        let connection = store.open_connection(false).unwrap();
        assert_eq!(
            MAX_REGISTERED_ACCOUNTS,
            connection
                .query_row(
                    "SELECT COUNT(*) FROM tokens WHERE activation_state = 0",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
        );
        drop(connection);

        let replacement_created = (latest_expiry
            + INITIAL_REGISTRATION_RECOVERY_WINDOW_MILLIS
            + ABANDONED_REGISTRATION_SAFETY_MARGIN_MILLIS
            + 1)
        .max(cleanup_at + 1);
        store
            .create_user_with_initial_pending_token(
                pending_registration_user(
                    "replacement",
                    "replacement@example.test",
                    replacement_created,
                ),
                "replacement-token",
                "phone",
                replacement_created,
                replacement_created + INITIAL_REGISTRATION_ACTIVATION_LEASE_MILLIS,
            )
            .unwrap();
        assert!(store
            .find_user_by_email("pending-0@example.test")
            .unwrap()
            .is_none());
        assert!(store
            .find_user_by_email("replacement@example.test")
            .unwrap()
            .is_some());
        let connection = store.open_connection(false).unwrap();
        assert_eq!(
            MAX_REGISTERED_ACCOUNTS,
            connection
                .query_row("SELECT COUNT(*) FROM users", [], |row| row.get::<_, i64>(0))
                .unwrap()
        );
        verify_snapshot_content_index(&connection).unwrap();
    }
}
