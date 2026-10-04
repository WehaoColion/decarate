// v0.0.8 - Upgrade the reader format atomically without changing recovery identities.
// v0.0.7 - Validate live declarations through compact note identities before storage.
// v0.0.6 - Admit retained receipts only after transactional history validation.
// v0.0.5 - Validate each declaration batch through a shared retained-note index.
// v0.0.4 - Persist a verified declaration batch as one journal transaction.
// v0.0.3 - Reserve a monotonic journal event for metadata without duplicating snapshots.
// v0.0.2 - Commit authenticated attachment references with their exact snapshot.
// v0.0.1 - Reserve the journal while retaining attachments from recovery snapshots.
// v2.22.56 - Commit note privacy barriers and retained-history redaction together.
// v2.22.51 - Reuse content-bound snapshot analysis without caching integrity decisions.
// v2.22.38 - Verify recovery history one snapshot at a time to bound memory use.
//! Durable local snapshot journal for the non-Android desktop client.
//!
//! The journal is deliberately separate from the human-readable primary JSON
//! mirror. A row is recoverable only when its owner, semantic metadata, raw and
//! canonical JSON digests, protected sync-state digest, parent pointer, source
//! and creation time all reproduce the stored envelope digest. SQLite
//! structural integrity alone is not treated as proof that application data
//! and its recovery state still belong to an account.
//!
//! Every new row binds the envelope of the latest valid row that existed when
//! it was appended. Recovery deliberately does not require that the complete
//! parent chain still be present: retention may prune an old prefix and a bad
//! parent may be preserved only in quarantine. The parent pointer is therefore
//! tamper evidence for an individual append, not a promise that all ancestors
//! remain recoverable.

#![cfg(not(target_os = "android"))]

use crate::app_data::{self, AppDataJsonCompatibility};
use crate::sync_core;
use rand::{rngs::OsRng, RngCore};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

const STORE_SCHEMA_VERSION: i64 = 4;
const BUSY_TIMEOUT: Duration = Duration::from_secs(10);
// Keep several independent recovery points even when a single document is
// large, but do not let a 24 MiB document force 64 full copies (~1.5 GiB).
const MIN_RETAINED_SNAPSHOTS_PER_OWNER: usize = 3;
const SOFT_BUDGET_BYTES_PER_OWNER: u64 = 64 * 1024 * 1024;
const ENVELOPE_DOMAIN: &[u8] = b"gridtimer-desktop-state-envelope-v2";
const LEGACY_ENVELOPE_DOMAIN: &[u8] = b"gridtimer-desktop-state-envelope-v1";
const OWNER_REGISTRY_DOMAIN: &[u8] = b"gridtimer-desktop-state-owner-v1";
const MAX_OWNER_BYTES: usize = 4 * 1024;
const MAX_SOURCE_BYTES: usize = 256;
const SUSPICIOUS_DROP_MIN_ITEMS: i64 = 8;
const SUSPICIOUS_DROP_ABSOLUTE_ITEMS: i64 = 4;

const SNAPSHOT_TABLE: &str = "desktop_state_snapshots";
const QUARANTINE_TABLE: &str = "desktop_state_snapshot_quarantine";
const OWNER_TABLE: &str = "desktop_state_owners";
const METADATA_TABLE: &str = "desktop_state_journal_metadata";
const SCOPE_MEDIA_TRANSACTION_PROOF_TABLE: &str = "desktop_state_scope_media_transaction_proofs";
const MIGRATION_SNAPSHOT_TABLE: &str = "desktop_state_snapshots_v2";
const MIGRATION_QUARANTINE_TABLE: &str = "desktop_state_snapshot_quarantine_v2";
const MIGRATION_OWNER_TABLE: &str = "desktop_state_owners_v2";
const MIGRATION_METADATA_TABLE: &str = "desktop_state_journal_metadata_v2";
const METADATA_SINGLETON_KEY: i64 = 1;

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct SaveAuditCounts {
    integrity: usize,
    history: usize,
    quick: usize,
}

#[cfg(test)]
thread_local! {
    static SAVE_AUDIT_COUNTS: std::cell::Cell<SaveAuditCounts> = const {
        std::cell::Cell::new(SaveAuditCounts { integrity: 0, history: 0, quick: 0 })
    };
}

#[path = "desktop_state_save_session.rs"]
mod save_session;
pub use save_session::{
    DesktopCommittedSnapshot, DesktopSaveObserver, DesktopSavePhase, NoopDesktopSaveObserver,
};
#[path = "desktop_state_snapshot_match.rs"]
mod snapshot_match;
use snapshot_match::snapshot_matches_row;

#[path = "desktop_state_privacy.rs"]
mod privacy;
pub use privacy::DesktopPrivacyPolicy;
#[cfg(target_os = "windows")]
#[path = "desktop_state_startup_session.rs"]
mod startup_session;
#[cfg(target_os = "windows")]
pub use startup_session::DesktopStartupWorkspaceHead;
#[path = "desktop_state_media_references.rs"]
mod media_references;
pub use media_references::DesktopMediaRetentionGuard;
#[path = "desktop_state_mirror_provenance.rs"]
mod mirror_provenance;
pub use mirror_provenance::DesktopMirrorWriter;
#[path = "desktop_sealed_media.rs"]
mod sealed_media;
use privacy::{create_privacy_schema, prepare_privacy_record, PRIVACY_TABLE};
pub use sealed_media::DesktopSealedMediaDeclaration;

pub type DesktopStateStoreResult<T> = Result<T, DesktopStateStoreError>;

/// The desktop identity used by both authenticated media receipts and the
/// account journal. Anonymous workspaces have their own explicit owner.
pub fn desktop_state_owner(
    server_instance_id: &str,
    account_namespace: &str,
    user_id: &str,
) -> DesktopStateStoreResult<String> {
    let server = server_instance_id.trim();
    let namespace = account_namespace.trim();
    let user = user_id.trim();
    if user.is_empty() {
        if !server.is_empty() || !namespace.is_empty() {
            return Err(integrity(
                "guest data cannot be bound to a partial account namespace",
            ));
        }
        return Ok("guest-v1".to_owned());
    }
    if server.is_empty() && namespace.is_empty() {
        return Ok(format!(
            "legacy-account-v1:{}",
            sync_core::token_identifier(user)
        ));
    }
    let identity_component = |value: &str| {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };
    if !identity_component(server)
        || !identity_component(namespace)
        || namespace != sync_core::account_namespace_identifier(server, user)
    {
        return Err(integrity(
            "the local state owner does not have a verifiable server account namespace",
        ));
    }
    Ok(format!(
        "account-v1:{server}:{namespace}:{}",
        sync_core::token_identifier(user)
    ))
}

/// One canonical builder prevents session creation and store verification from
/// disagreeing about relative paths, separators or Windows path casing.
pub fn desktop_note_session_scope(root: &Path, owner: &str) -> DesktopStateStoreResult<String> {
    validate_owner(owner)?;
    let root = fs::canonicalize(root)?;
    let normalized = root
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    let fingerprint =
        sha256_hex(format!("gridtimer-isolated-app-namespace-v2\0{normalized}").as_bytes());
    Ok(format!("desktop-note-session-v1:{owner}:{fingerprint}"))
}

#[derive(Debug)]
pub enum DesktopStateStoreError {
    Io(io::Error),
    Sqlite(rusqlite::Error),
    Json(serde_json::Error),
    Integrity(String),
    InitializedOwnerWithoutValidSnapshot,
    SuspiciousItemDrop {
        previous_items: i64,
        incoming_items: i64,
    },
}

impl fmt::Display for DesktopStateStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "desktop state journal I/O failed: {error}"),
            Self::Sqlite(error) => write!(formatter, "desktop state journal SQLite failed: {error}"),
            Self::Json(error) => write!(formatter, "desktop state JSON failed: {error}"),
            Self::Integrity(message) => {
                write!(formatter, "desktop state journal integrity failure: {message}")
            }
            Self::InitializedOwnerWithoutValidSnapshot => write!(
                formatter,
                "desktop state owner was initialized previously but has no valid recovery snapshot"
            ),
            Self::SuspiciousItemDrop {
                previous_items,
                incoming_items,
            } => write!(
                formatter,
                "desktop state snapshot dropped from {previous_items} recoverable items to {incoming_items} without deletion evidence"
            ),
        }
    }
}

impl Error for DesktopStateStoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Sqlite(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Integrity(_)
            | Self::InitializedOwnerWithoutValidSnapshot
            | Self::SuspiciousItemDrop { .. } => None,
        }
    }
}

impl From<io::Error> for DesktopStateStoreError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for DesktopStateStoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<serde_json::Error> for DesktopStateStoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesktopStateSnapshot {
    pub id: i64,
    pub owner: String,
    pub app_data_json: String,
    pub protected_sync_state: Vec<u8>,
    pub schema_version: i64,
    pub revision: i64,
    pub item_count: i64,
    pub semantic_summary: String,
    pub raw_sha256: String,
    pub canonical_json_sha256: String,
    pub sync_state_sha256: String,
    pub parent_envelope_sha256: String,
    pub envelope_sha256: String,
    pub created_at_epoch_millis: i64,
    pub source: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesktopStateJournalEvidence {
    pub journal_id: String,
    pub commit_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LegacyDesktopStateSnapshot {
    id: i64,
    owner: String,
    app_data_json: String,
    schema_version: i64,
    revision: i64,
    item_count: i64,
    semantic_summary: String,
    raw_sha256: String,
    canonical_json_sha256: String,
    parent_envelope_sha256: String,
    envelope_sha256: String,
    created_at_epoch_millis: i64,
    source: String,
}

#[derive(Clone, Debug)]
pub struct DesktopStateStore {
    database_path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DesktopStateOwnerRegistry {
    owner: String,
    initialized_at_epoch_millis: i64,
    first_envelope_sha256: String,
    registry_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ScopeMediaTransactionProof {
    source: String,
    snapshot_id: i64,
    pinned_snapshot_id: Option<i64>,
    owner: String,
    raw_sha256: String,
    sync_state_sha256: String,
    envelope_sha256: String,
}

impl DesktopStateStore {
    pub fn open(database_path: impl Into<PathBuf>) -> DesktopStateStoreResult<Self> {
        let database_path = database_path.into();
        if let Some(parent) = database_path.parent() {
            fs::create_dir_all(parent)?;
        }
        if database_path.exists() && fs::metadata(&database_path)?.len() == 0 {
            return Err(DesktopStateStoreError::Integrity(format!(
                "existing journal is empty: {}",
                database_path.display()
            )));
        }

        let store = Self { database_path };
        let mut connection = store.open_connection(true)?;
        verify_sqlite_integrity(&connection)?;
        apply_schema(&mut connection)?;
        verify_required_schema(&connection)?;
        verify_foreign_keys(&connection)?;
        verify_all_owner_registries(&connection)?;
        quarantine_invalid_rows(&mut connection, None, system_time_epoch_millis())?;
        verify_quick_check(&connection)?;
        Ok(store)
    }

    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    /// Returns the stable identity and monotonic successful-snapshot sequence
    /// for binding this journal to evidence stored outside the database. This
    /// method never repairs or mutates metadata; missing, duplicated, malformed
    /// or internally inconsistent evidence fails closed.
    pub fn journal_evidence(&self) -> DesktopStateStoreResult<DesktopStateJournalEvidence> {
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
        verify_quick_check(&transaction)?;
        verify_required_schema(&transaction)?;
        verify_foreign_keys(&transaction)?;
        let evidence = read_journal_evidence(&transaction)?;
        transaction.commit()?;
        Ok(evidence)
    }

    /// Returns whether this owner has ever committed a snapshot to this
    /// journal. The registry survives snapshot quarantine and retention GC, so
    /// `true` with `latest_valid(owner) == None` is a fail-closed recovery state,
    /// not permission to bootstrap an unjournaled primary JSON file.
    pub fn owner_initialized(&self, owner: &str) -> DesktopStateStoreResult<bool> {
        validate_owner(owner)?;
        let connection = self.open_connection(false)?;
        verify_quick_check(&connection)?;
        owner_registry_initialized(&connection, owner)
    }

    /// Records an exact snapshot with an empty protected sync state. A later
    /// note privacy transition can redact older note payloads transactionally.
    /// Identical application and sync-state bytes for the same owner are
    /// deduplicated. Ordinary local writes that collapse a substantial state to
    /// almost empty without tombstone evidence fail closed; explicit restore
    /// and migration sources are allowed to cross that guard.
    pub fn record(
        &self,
        owner: &str,
        app_data_json: &str,
        now_epoch_millis: i64,
        source: &str,
    ) -> DesktopStateStoreResult<DesktopStateSnapshot> {
        self.record_with_sync_state(owner, app_data_json, &[], now_epoch_millis, source)
    }

    /// Records application data and its already-protected workspace sync state
    /// as one atomic SQLite commit. The sync-state bytes are opaque to this
    /// store; their digest is bound into the same envelope as the application
    /// JSON so recovery can never pair values from different commits. Privacy
    /// redaction rebinds the application envelope while preserving these bytes.
    pub fn record_with_sync_state(
        &self,
        owner: &str,
        app_data_json: &str,
        protected_sync_state: &[u8],
        now_epoch_millis: i64,
        source: &str,
    ) -> DesktopStateStoreResult<DesktopStateSnapshot> {
        self.record_with_sync_state_and_media_declaration(
            owner,
            app_data_json,
            protected_sync_state,
            now_epoch_millis,
            source,
            None,
        )
    }

    /// Commit the exact sealed note and its authenticated reference metadata
    /// together. A failed snapshot append cannot leave a new declaration behind.
    pub fn record_with_sync_state_and_media_declaration(
        &self,
        owner: &str,
        app_data_json: &str,
        protected_sync_state: &[u8],
        now_epoch_millis: i64,
        source: &str,
        declaration: Option<&DesktopSealedMediaDeclaration>,
    ) -> DesktopStateStoreResult<DesktopStateSnapshot> {
        self.record_with_sync_state_and_media_declarations(
            owner,
            app_data_json,
            protected_sync_state,
            now_epoch_millis,
            source,
            declaration.map(std::slice::from_ref).unwrap_or_default(),
        )
    }

    pub fn record_with_sync_state_and_media_declarations(
        &self,
        owner: &str,
        app_data_json: &str,
        protected_sync_state: &[u8],
        now_epoch_millis: i64,
        source: &str,
        declarations: &[DesktopSealedMediaDeclaration],
    ) -> DesktopStateStoreResult<DesktopStateSnapshot> {
        let declaration_scope = if declarations.is_empty() {
            String::new()
        } else {
            desktop_note_session_scope(
                self.database_path()
                    .parent()
                    .ok_or_else(|| integrity("journal has no workspace root"))?,
                owner,
            )?
        };
        let declaration_index = if declarations.is_empty() {
            None
        } else {
            Some(
                crate::desktop_private_media_index::DesktopPrivateMediaReferences::from_snapshot(
                    app_data_json,
                )
                .map_err(integrity)?,
            )
        };
        for declaration in declarations {
            declaration.validate_authority(owner, &declaration_scope)?;
            if !declaration.permits_retained_snapshot() {
                declaration.apply_to_index(
                    owner,
                    &declaration_scope,
                    &DesktopPrivacyPolicy::default(),
                    declaration_index.as_ref().unwrap(),
                )?;
            }
            if scope_media_transaction_id_from_source(source).is_some() {
                return Err(integrity(
                    "scope migration cannot attach a live editor declaration",
                ));
            }
        }
        validate_owner(owner)?;
        let source = normalized_source(source)?;
        let analysis = analyze_app_data_json(app_data_json, 0)?;
        let raw_sha256 = sha256_hex(app_data_json.as_bytes());
        let sync_state_sha256 = sha256_hex(protected_sync_state);
        let now_epoch_millis = now_epoch_millis.max(0);

        let mut connection = self.open_connection(false)?;
        verify_quick_check(&connection)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = record_snapshot_in_transaction(
            &transaction,
            owner,
            app_data_json,
            protected_sync_state,
            now_epoch_millis,
            &source,
            &declaration_scope,
            declarations,
            analysis,
            raw_sha256,
            sync_state_sha256,
            None,
        );
        // Preserve quarantine on these two established fail-closed outcomes.
        if result.is_ok()
            || matches!(
                &result,
                Err(DesktopStateStoreError::InitializedOwnerWithoutValidSnapshot
                    | DesktopStateStoreError::SuspiciousItemDrop { .. })
            )
        {
            transaction.commit()?;
        }
        result
    }

    /// Returns the newest inserted row that fully verifies for `owner`.
    /// Corrupt rows are copied to quarantine and excluded from recovery.
    pub fn latest_valid(
        &self,
        owner: &str,
        now_epoch_millis: i64,
    ) -> DesktopStateStoreResult<Option<DesktopStateSnapshot>> {
        validate_owner(owner)?;
        let mut connection = self.open_connection(false)?;
        verify_quick_check(&connection)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        quarantine_invalid_rows_preserving_evidence_in_transaction(
            &transaction,
            Some(owner),
            now_epoch_millis.max(0),
        )?;
        if !owner_registry_initialized(&transaction, owner)? {
            transaction.commit()?;
            return Ok(None);
        }
        let latest = latest_valid_in_transaction(&transaction, owner, 0)?;
        transaction.commit()?;
        Ok(latest)
    }

    /// Proves that the exact primary JSON bytes are present in a valid journal
    /// envelope for this owner. Invalid JSON is simply not a match; physical
    /// SQLite failures remain hard errors.
    pub fn validate_exact(
        &self,
        owner: &str,
        app_data_json: &str,
        now_epoch_millis: i64,
    ) -> DesktopStateStoreResult<bool> {
        validate_owner(owner)?;
        if analyze_app_data_json(app_data_json, 0).is_err() {
            return Ok(false);
        }
        let raw_sha256 = sha256_hex(app_data_json.as_bytes());
        let mut connection = self.open_connection(false)?;
        verify_quick_check(&connection)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        quarantine_invalid_rows_preserving_evidence_in_transaction(
            &transaction,
            Some(owner),
            now_epoch_millis.max(0),
        )?;
        if !owner_registry_initialized(&transaction, owner)? {
            transaction.commit()?;
            return Ok(false);
        }
        let matched = read_snapshot_by_raw_digest(&transaction, owner, &raw_sha256, 0)?
            .is_some_and(|snapshot| snapshot.app_data_json == app_data_json);
        transaction.commit()?;
        Ok(matched)
    }

    /// Returns the one fully verified snapshot bound to a durable scope-media
    /// transaction source. A reused source or a mismatched owner/digest is an
    /// integrity error rather than an ambiguous negative result.
    pub fn valid_scope_media_transaction_snapshot(
        &self,
        owner: &str,
        raw_sha256: &str,
        source: &str,
        _now_epoch_millis: i64,
    ) -> DesktopStateStoreResult<Option<DesktopStateSnapshot>> {
        validate_owner(owner)?;
        let source = normalized_source(source)?;
        if scope_media_transaction_id_from_source(&source).is_none()
            || !valid_sha256_hex(raw_sha256)
            || raw_sha256.bytes().any(|byte| byte.is_ascii_uppercase())
        {
            return Err(integrity("scope-media transaction proof is malformed"));
        }
        let mut connection = self.open_connection(false)?;
        verify_quick_check(&connection)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let Some(proof) = read_scope_media_transaction_proof(&transaction, &source)? else {
            if scope_media_transaction_source_was_quarantined(&transaction, &source)? {
                return Err(integrity(
                    "scope-media transaction source has quarantined evidence",
                ));
            }
            transaction.commit()?;
            return Ok(None);
        };
        if proof.owner != owner || proof.raw_sha256 != raw_sha256 {
            return Err(integrity(
                "scope-media transaction proof does not match its owner and snapshot",
            ));
        }
        let snapshot = verified_scope_media_transaction_snapshot(&transaction, &proof)?;
        transaction.commit()?;
        Ok(snapshot)
    }

    /// Marks a verified scope-media transaction as complete without freeing
    /// its source for reuse. Completion is idempotent: the permanent tombstone
    /// retains the snapshot identity while removing the foreign-key pin that
    /// protects pending snapshots from retention GC. Later privacy redaction
    /// can rebind its digests; it never makes the completed source reusable.
    pub fn complete_scope_media_transaction(&self, source: &str) -> DesktopStateStoreResult<()> {
        let source = normalized_source(source)?;
        if scope_media_transaction_id_from_source(&source).is_none() {
            return Err(integrity("scope-media transaction proof is malformed"));
        }
        let mut connection = self.open_connection(false)?;
        verify_quick_check(&connection)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let proof = read_scope_media_transaction_proof(&transaction, &source)?
            .ok_or_else(|| integrity("scope-media transaction proof does not exist"))?;
        let Some(snapshot) = verified_scope_media_transaction_snapshot(&transaction, &proof)?
        else {
            transaction.commit()?;
            return Ok(());
        };
        let changed = transaction.execute(
            "UPDATE desktop_state_scope_media_transaction_proofs
             SET pinned_snapshot_id = NULL
             WHERE source = ?1 AND snapshot_id = ?2 AND pinned_snapshot_id = ?2",
            params![source, snapshot.id],
        )?;
        if changed != 1 {
            return Err(integrity(
                "scope-media transaction completion changed an unexpected row count",
            ));
        }
        let tombstone = read_scope_media_transaction_proof(&transaction, &proof.source)?
            .ok_or_else(|| integrity("completed scope-media transaction tombstone disappeared"))?;
        verify_scope_media_transaction_proof(&tombstone)?;
        let expected = ScopeMediaTransactionProof {
            pinned_snapshot_id: None,
            ..proof
        };
        if tombstone != expected {
            return Err(integrity(
                "scope-media transaction completion changed its permanent binding",
            ));
        }
        transaction.commit()?;
        Ok(())
    }

    fn open_connection(&self, create: bool) -> DesktopStateStoreResult<Connection> {
        let mut flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_FULL_MUTEX;
        if create {
            flags |= OpenFlags::SQLITE_OPEN_CREATE;
        }
        let connection = Connection::open_with_flags(&self.database_path, flags)?;
        configure_connection(&connection)?;
        Ok(connection)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SemanticSummary {
    categories: i64,
    slots: i64,
    meaningful_slots: i64,
    slot_order_entries: i64,
    sessions: i64,
    archived_tasks: i64,
    note_folders: i64,
    notes: i64,
    note_revisions: i64,
    note_attachments: i64,
    active_note_versions: i64,
    deleted_note_versions: i64,
    active_note_version_attachments: i64,
    deleted_note_version_attachments: i64,
    tombstones: i64,
    sync_conflicts: i64,
    finance_day_entries: i64,
    finance_month_entries: i64,
    finance_day_revision_entries: i64,
    finance_month_revision_entries: i64,
    recoverable_items: i64,
}

#[derive(Clone, Debug)]
struct AppDataAnalysis {
    schema_version: i64,
    revision: i64,
    item_count: i64,
    tombstone_count: i64,
    semantic_summary: String,
    canonical_json_sha256: String,
}

fn configure_connection(connection: &Connection) -> DesktopStateStoreResult<()> {
    connection.busy_timeout(BUSY_TIMEOUT)?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.pragma_update(None, "secure_delete", "ON")?;
    connection.pragma_update(None, "wal_autocheckpoint", 1_000)?;
    connection.pragma_update(None, "trusted_schema", "OFF")?;
    verify_connection_settings(connection)
}

fn verify_connection_settings(connection: &Connection) -> DesktopStateStoreResult<()> {
    let journal_mode: String =
        connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
    let synchronous: i64 = connection.pragma_query_value(None, "synchronous", |row| row.get(0))?;
    let foreign_keys: i64 =
        connection.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
    let secure_delete: i64 =
        connection.pragma_query_value(None, "secure_delete", |row| row.get(0))?;
    if !journal_mode.eq_ignore_ascii_case("wal")
        || synchronous != 2
        || foreign_keys != 1
        || secure_delete != 1
    {
        return Err(integrity(format!(
            "unsafe SQLite settings: journal_mode={journal_mode}, synchronous={synchronous}, foreign_keys={foreign_keys}"
        )));
    }
    Ok(())
}

fn apply_schema(connection: &mut Connection) -> DesktopStateStoreResult<()> {
    let current_version: i64 =
        connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if current_version > STORE_SCHEMA_VERSION {
        return Err(integrity(format!(
            "journal schema {current_version} is newer than supported {STORE_SCHEMA_VERSION}"
        )));
    }
    match current_version {
        0 => {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            create_v2_schema(&transaction)?;
            transaction.pragma_update(None, "user_version", STORE_SCHEMA_VERSION)?;
            verify_required_schema(&transaction)?;
            verify_foreign_keys(&transaction)?;
            read_journal_evidence(&transaction)?;
            verify_sqlite_integrity(&transaction)?;
            transaction.commit()?;
        }
        1 => migrate_v1_to_v2(connection)?,
        2 => {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            ensure_scope_media_transaction_proof_schema(&transaction)?;
            create_privacy_schema(&transaction)?;
            mirror_provenance::create_schema(&transaction)?;
            transaction.pragma_update(None, "user_version", STORE_SCHEMA_VERSION)?;
            verify_required_schema(&transaction)?;
            verify_foreign_keys(&transaction)?;
            read_journal_evidence(&transaction)?;
            transaction.commit()?;
        }
        3 | STORE_SCHEMA_VERSION => {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            ensure_scope_media_transaction_proof_schema(&transaction)?;
            // No snapshot, digest or owner identity changes during this format
            // transition. The marker commits atomically with its validation.
            if current_version == 3 {
                mirror_provenance::create_schema(&transaction)?;
                transaction.pragma_update(None, "user_version", STORE_SCHEMA_VERSION)?;
            }
            verify_required_schema(&transaction)?;
            verify_foreign_keys(&transaction)?;
            verify_all_owner_registries(&transaction)?;
            read_journal_evidence(&transaction)?;
            transaction.commit()?;
        }
        _ => {
            return Err(integrity(format!(
                "unsupported journal schema transition from {current_version}"
            )));
        }
    }
    Ok(())
}

fn create_v2_schema(connection: &Connection) -> DesktopStateStoreResult<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS desktop_state_journal_metadata (
             singleton INTEGER NOT NULL PRIMARY KEY CHECK(singleton = 1),
             journal_id TEXT NOT NULL
                 CHECK(length(journal_id) = 64)
                 CHECK(journal_id NOT GLOB '*[^0-9a-f]*'),
             commit_sequence INTEGER NOT NULL CHECK(commit_sequence >= 0)
         ) STRICT;

         CREATE TABLE IF NOT EXISTS desktop_state_owners (
             owner TEXT NOT NULL PRIMARY KEY,
             initialized_at_epoch_millis INTEGER NOT NULL
                 CHECK(initialized_at_epoch_millis >= 0),
             first_envelope_sha256 TEXT NOT NULL
                 CHECK(length(first_envelope_sha256) = 64),
             registry_sha256 TEXT NOT NULL UNIQUE
                 CHECK(length(registry_sha256) = 64)
         ) STRICT;

         CREATE TABLE IF NOT EXISTS desktop_state_snapshots (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             owner TEXT NOT NULL,
             app_data_json TEXT NOT NULL CHECK(json_valid(app_data_json)),
             protected_sync_state BLOB NOT NULL,
             schema_version INTEGER NOT NULL CHECK(schema_version >= 0),
             revision INTEGER NOT NULL CHECK(revision >= 0),
             item_count INTEGER NOT NULL CHECK(item_count >= 0),
             semantic_summary TEXT NOT NULL CHECK(json_valid(semantic_summary)),
             raw_sha256 TEXT NOT NULL CHECK(length(raw_sha256) = 64),
             canonical_json_sha256 TEXT NOT NULL CHECK(length(canonical_json_sha256) = 64),
             sync_state_sha256 TEXT NOT NULL CHECK(length(sync_state_sha256) = 64),
             parent_envelope_sha256 TEXT NOT NULL
                 CHECK(length(parent_envelope_sha256) IN (0, 64)),
             envelope_sha256 TEXT NOT NULL UNIQUE CHECK(length(envelope_sha256) = 64),
             created_at_epoch_millis INTEGER NOT NULL CHECK(created_at_epoch_millis >= 0),
             source TEXT NOT NULL,
             FOREIGN KEY(owner) REFERENCES desktop_state_owners(owner)
                 ON UPDATE RESTRICT ON DELETE RESTRICT
                 DEFERRABLE INITIALLY DEFERRED
         ) STRICT;

         CREATE INDEX IF NOT EXISTS desktop_state_snapshots_owner_id
             ON desktop_state_snapshots(owner, id DESC);
         CREATE INDEX IF NOT EXISTS desktop_state_snapshots_owner_raw
             ON desktop_state_snapshots(owner, raw_sha256, id DESC);

         CREATE TABLE IF NOT EXISTS desktop_state_snapshot_quarantine (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             snapshot_id INTEGER NOT NULL,
             owner TEXT NOT NULL,
             app_data_json TEXT NOT NULL,
             protected_sync_state BLOB NOT NULL,
             schema_version INTEGER NOT NULL,
             revision INTEGER NOT NULL,
             item_count INTEGER NOT NULL,
             semantic_summary TEXT NOT NULL,
             stored_raw_sha256 TEXT NOT NULL,
             observed_raw_sha256 TEXT NOT NULL,
             stored_canonical_json_sha256 TEXT NOT NULL,
             observed_canonical_json_sha256 TEXT NOT NULL,
             sync_state_sha256 TEXT NOT NULL,
             observed_sync_state_sha256 TEXT NOT NULL,
             parent_envelope_sha256 TEXT NOT NULL,
             stored_envelope_sha256 TEXT NOT NULL,
             observed_envelope_sha256 TEXT NOT NULL,
             created_at_epoch_millis INTEGER NOT NULL,
             source TEXT NOT NULL,
             reason TEXT NOT NULL,
             quarantined_at_epoch_millis INTEGER NOT NULL,
             FOREIGN KEY(owner) REFERENCES desktop_state_owners(owner)
                 ON UPDATE RESTRICT ON DELETE RESTRICT
         ) STRICT;

         CREATE INDEX IF NOT EXISTS desktop_state_snapshot_quarantine_owner_time
              ON desktop_state_snapshot_quarantine(owner, quarantined_at_epoch_millis DESC, id DESC);",
    )?;
    create_scope_media_transaction_proof_table(connection)?;
    create_privacy_schema(connection)?;
    mirror_provenance::create_schema(connection)?;
    insert_journal_metadata(connection, METADATA_TABLE, 0)?;
    Ok(())
}

fn create_scope_media_transaction_proof_table(
    connection: &Connection,
) -> DesktopStateStoreResult<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS desktop_state_scope_media_transaction_proofs (
             source TEXT NOT NULL PRIMARY KEY
                 CHECK(length(source) <= 256),
             snapshot_id INTEGER NOT NULL UNIQUE CHECK(snapshot_id > 0),
             pinned_snapshot_id INTEGER UNIQUE
                 CHECK(pinned_snapshot_id IS NULL OR pinned_snapshot_id = snapshot_id),
             owner TEXT NOT NULL,
             raw_sha256 TEXT NOT NULL
                 CHECK(length(raw_sha256) = 64)
                 CHECK(raw_sha256 NOT GLOB '*[^0-9a-f]*'),
             sync_state_sha256 TEXT NOT NULL
                 CHECK(length(sync_state_sha256) = 64)
                 CHECK(sync_state_sha256 NOT GLOB '*[^0-9a-f]*'),
             envelope_sha256 TEXT NOT NULL UNIQUE
                 CHECK(length(envelope_sha256) = 64)
                 CHECK(envelope_sha256 NOT GLOB '*[^0-9a-f]*'),
             FOREIGN KEY(pinned_snapshot_id) REFERENCES desktop_state_snapshots(id)
                 ON UPDATE RESTRICT ON DELETE RESTRICT,
             FOREIGN KEY(owner) REFERENCES desktop_state_owners(owner)
                 ON UPDATE RESTRICT ON DELETE RESTRICT
         ) STRICT;",
    )?;
    Ok(())
}

fn ensure_scope_media_transaction_proof_schema(
    connection: &Connection,
) -> DesktopStateStoreResult<()> {
    create_scope_media_transaction_proof_table(connection)?;
    verify_scope_media_transaction_proof_table_layout(connection)?;
    backfill_scope_media_transaction_proofs(connection)?;
    verify_all_scope_media_transaction_proofs(connection)
}

fn create_v2_migration_schema(connection: &Connection) -> DesktopStateStoreResult<()> {
    connection.execute_batch(
        "CREATE TABLE desktop_state_journal_metadata_v2 (
             singleton INTEGER NOT NULL PRIMARY KEY CHECK(singleton = 1),
             journal_id TEXT NOT NULL
                 CHECK(length(journal_id) = 64)
                 CHECK(journal_id NOT GLOB '*[^0-9a-f]*'),
             commit_sequence INTEGER NOT NULL CHECK(commit_sequence >= 0)
         ) STRICT;

         CREATE TABLE desktop_state_owners_v2 (
             owner TEXT NOT NULL PRIMARY KEY,
             initialized_at_epoch_millis INTEGER NOT NULL
                 CHECK(initialized_at_epoch_millis >= 0),
             first_envelope_sha256 TEXT NOT NULL
                 CHECK(length(first_envelope_sha256) = 64),
             registry_sha256 TEXT NOT NULL UNIQUE
                 CHECK(length(registry_sha256) = 64)
         ) STRICT;

         CREATE TABLE desktop_state_snapshots_v2 (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             owner TEXT NOT NULL,
             app_data_json TEXT NOT NULL CHECK(json_valid(app_data_json)),
             protected_sync_state BLOB NOT NULL,
             schema_version INTEGER NOT NULL CHECK(schema_version >= 0),
             revision INTEGER NOT NULL CHECK(revision >= 0),
             item_count INTEGER NOT NULL CHECK(item_count >= 0),
             semantic_summary TEXT NOT NULL CHECK(json_valid(semantic_summary)),
             raw_sha256 TEXT NOT NULL CHECK(length(raw_sha256) = 64),
             canonical_json_sha256 TEXT NOT NULL CHECK(length(canonical_json_sha256) = 64),
             sync_state_sha256 TEXT NOT NULL CHECK(length(sync_state_sha256) = 64),
             parent_envelope_sha256 TEXT NOT NULL
                 CHECK(length(parent_envelope_sha256) IN (0, 64)),
             envelope_sha256 TEXT NOT NULL UNIQUE CHECK(length(envelope_sha256) = 64),
             created_at_epoch_millis INTEGER NOT NULL CHECK(created_at_epoch_millis >= 0),
             source TEXT NOT NULL,
             FOREIGN KEY(owner) REFERENCES desktop_state_owners_v2(owner)
                 ON UPDATE RESTRICT ON DELETE RESTRICT
                 DEFERRABLE INITIALLY DEFERRED
         ) STRICT;

         CREATE TABLE desktop_state_snapshot_quarantine_v2 (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             snapshot_id INTEGER NOT NULL,
             owner TEXT NOT NULL,
             app_data_json TEXT NOT NULL,
             protected_sync_state BLOB NOT NULL,
             schema_version INTEGER NOT NULL,
             revision INTEGER NOT NULL,
             item_count INTEGER NOT NULL,
             semantic_summary TEXT NOT NULL,
             stored_raw_sha256 TEXT NOT NULL,
             observed_raw_sha256 TEXT NOT NULL,
             stored_canonical_json_sha256 TEXT NOT NULL,
             observed_canonical_json_sha256 TEXT NOT NULL,
             sync_state_sha256 TEXT NOT NULL,
             observed_sync_state_sha256 TEXT NOT NULL,
             parent_envelope_sha256 TEXT NOT NULL,
             stored_envelope_sha256 TEXT NOT NULL,
             observed_envelope_sha256 TEXT NOT NULL,
             created_at_epoch_millis INTEGER NOT NULL,
             source TEXT NOT NULL,
             reason TEXT NOT NULL,
             quarantined_at_epoch_millis INTEGER NOT NULL,
             FOREIGN KEY(owner) REFERENCES desktop_state_owners_v2(owner)
                 ON UPDATE RESTRICT ON DELETE RESTRICT
         ) STRICT;",
    )?;
    Ok(())
}

fn migrate_v1_to_v2(connection: &mut Connection) -> DesktopStateStoreResult<()> {
    migrate_v1_to_v2_with_precommit_check(connection, || Ok(()))
}

fn migrate_v1_to_v2_with_precommit_check<F>(
    connection: &mut Connection,
    precommit_check: F,
) -> DesktopStateStoreResult<()>
where
    F: FnOnce() -> DesktopStateStoreResult<()>,
{
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    verify_sqlite_integrity(&transaction)?;
    verify_v1_required_schema(&transaction)?;
    verify_foreign_keys(&transaction)?;
    verify_all_owner_registries(&transaction)?;
    // v1 had no external-evidence counter. Its AUTOINCREMENT value survives
    // quarantine and retention deletes, so it is the lossless monotonic base.
    let legacy_commit_sequence =
        read_snapshot_autoincrement_sequence(&transaction, SNAPSHOT_TABLE)?;

    let registries = read_all_owner_registries(&transaction)?;
    let legacy_snapshots = read_all_legacy_snapshots(&transaction)?;
    for snapshot in &legacy_snapshots {
        verify_legacy_snapshot(snapshot, Some(&snapshot.owner), 0)?;
    }
    let (migrated_snapshots, envelope_mappings) =
        migrate_legacy_snapshot_envelopes(legacy_snapshots)?;
    let legacy_quarantine_count = table_row_count(&transaction, QUARANTINE_TABLE)?;

    create_v2_migration_schema(&transaction)?;
    insert_journal_metadata(
        &transaction,
        MIGRATION_METADATA_TABLE,
        legacy_commit_sequence,
    )?;
    for registry in registries {
        let migrated_first_envelope = envelope_mappings
            .get(&(
                registry.owner.clone(),
                registry.first_envelope_sha256.to_ascii_lowercase(),
            ))
            .cloned()
            .unwrap_or(registry.first_envelope_sha256);
        let migrated_registry_sha256 = owner_registry_sha256(
            &registry.owner,
            registry.initialized_at_epoch_millis,
            &migrated_first_envelope,
        );
        transaction.execute(
            "INSERT INTO desktop_state_owners_v2(
                 owner, initialized_at_epoch_millis, first_envelope_sha256,
                 registry_sha256
             ) VALUES (?1, ?2, ?3, ?4)",
            params![
                registry.owner,
                registry.initialized_at_epoch_millis,
                migrated_first_envelope,
                migrated_registry_sha256,
            ],
        )?;
    }
    for snapshot in &migrated_snapshots {
        transaction.execute(
            "INSERT INTO desktop_state_snapshots_v2(
                 id, owner, app_data_json, protected_sync_state, schema_version,
                 revision, item_count, semantic_summary, raw_sha256,
                 canonical_json_sha256, sync_state_sha256,
                 parent_envelope_sha256, envelope_sha256,
                 created_at_epoch_millis, source
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                       ?13, ?14, ?15)",
            params![
                snapshot.id,
                snapshot.owner,
                snapshot.app_data_json,
                snapshot.protected_sync_state,
                snapshot.schema_version,
                snapshot.revision,
                snapshot.item_count,
                snapshot.semantic_summary,
                snapshot.raw_sha256,
                snapshot.canonical_json_sha256,
                snapshot.sync_state_sha256,
                snapshot.parent_envelope_sha256,
                snapshot.envelope_sha256,
                snapshot.created_at_epoch_millis,
                snapshot.source,
            ],
        )?;
    }
    set_snapshot_autoincrement_sequence(
        &transaction,
        MIGRATION_SNAPSHOT_TABLE,
        legacy_commit_sequence,
    )?;

    let empty_sync_state_sha256 = sha256_hex(&[]);
    transaction.execute(
        "INSERT INTO desktop_state_snapshot_quarantine_v2(
             id, snapshot_id, owner, app_data_json, protected_sync_state,
             schema_version, revision, item_count, semantic_summary,
             stored_raw_sha256, observed_raw_sha256,
             stored_canonical_json_sha256, observed_canonical_json_sha256,
             sync_state_sha256, observed_sync_state_sha256,
             parent_envelope_sha256, stored_envelope_sha256,
             observed_envelope_sha256, created_at_epoch_millis, source,
             reason, quarantined_at_epoch_millis
         )
         SELECT id, snapshot_id, owner, app_data_json, X'', schema_version,
                revision, item_count, semantic_summary, stored_raw_sha256,
                observed_raw_sha256, stored_canonical_json_sha256,
                observed_canonical_json_sha256, ?1, ?1,
                parent_envelope_sha256, stored_envelope_sha256,
                observed_envelope_sha256, created_at_epoch_millis, source,
                reason, quarantined_at_epoch_millis
         FROM desktop_state_snapshot_quarantine ORDER BY id",
        params![empty_sync_state_sha256],
    )?;

    if table_row_count(&transaction, MIGRATION_METADATA_TABLE)? != 1
        || table_row_count(&transaction, MIGRATION_OWNER_TABLE)?
            != table_row_count(&transaction, OWNER_TABLE)?
        || table_row_count(&transaction, MIGRATION_SNAPSHOT_TABLE)?
            != migrated_snapshots.len() as i64
        || table_row_count(&transaction, MIGRATION_QUARANTINE_TABLE)? != legacy_quarantine_count
    {
        return Err(integrity(
            "v1 to v2 migration changed an unexpected number of rows",
        ));
    }

    transaction.execute_batch(
        "DROP TABLE desktop_state_snapshot_quarantine;
         DROP TABLE desktop_state_snapshots;
         DROP TABLE desktop_state_owners;
         ALTER TABLE desktop_state_journal_metadata_v2
             RENAME TO desktop_state_journal_metadata;
         ALTER TABLE desktop_state_owners_v2 RENAME TO desktop_state_owners;
         ALTER TABLE desktop_state_snapshots_v2 RENAME TO desktop_state_snapshots;
         ALTER TABLE desktop_state_snapshot_quarantine_v2
             RENAME TO desktop_state_snapshot_quarantine;
         CREATE INDEX desktop_state_snapshots_owner_id
             ON desktop_state_snapshots(owner, id DESC);
         CREATE INDEX desktop_state_snapshots_owner_raw
             ON desktop_state_snapshots(owner, raw_sha256, id DESC);
         CREATE INDEX desktop_state_snapshot_quarantine_owner_time
              ON desktop_state_snapshot_quarantine(owner, quarantined_at_epoch_millis DESC, id DESC);",
    )?;
    ensure_scope_media_transaction_proof_schema(&transaction)?;
    create_privacy_schema(&transaction)?;
    mirror_provenance::create_schema(&transaction)?;
    transaction.pragma_update(None, "user_version", STORE_SCHEMA_VERSION)?;

    verify_required_schema(&transaction)?;
    verify_foreign_keys(&transaction)?;
    verify_all_owner_registries(&transaction)?;
    verify_all_v2_snapshots(&transaction)?;
    let evidence = read_journal_evidence(&transaction)?;
    if evidence.commit_sequence != legacy_commit_sequence {
        return Err(integrity(
            "v1 to v2 migration changed the journal commit sequence",
        ));
    }
    verify_sqlite_integrity(&transaction)?;
    precommit_check()?;
    transaction.commit()?;
    Ok(())
}

fn read_all_owner_registries(
    connection: &Connection,
) -> DesktopStateStoreResult<Vec<DesktopStateOwnerRegistry>> {
    let mut statement = connection.prepare(
        "SELECT owner, initialized_at_epoch_millis, first_envelope_sha256,
                registry_sha256
         FROM desktop_state_owners ORDER BY owner",
    )?;
    let registries = statement
        .query_map([], |row| {
            Ok(DesktopStateOwnerRegistry {
                owner: row.get(0)?,
                initialized_at_epoch_millis: row.get(1)?,
                first_envelope_sha256: row.get(2)?,
                registry_sha256: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(registries)
}

fn read_all_legacy_snapshots(
    connection: &Connection,
) -> DesktopStateStoreResult<Vec<LegacyDesktopStateSnapshot>> {
    let mut statement = connection.prepare(
        "SELECT id, owner, app_data_json, schema_version, revision, item_count,
                semantic_summary, raw_sha256, canonical_json_sha256,
                parent_envelope_sha256, envelope_sha256,
                created_at_epoch_millis, source
         FROM desktop_state_snapshots ORDER BY owner, id",
    )?;
    let snapshots = statement
        .query_map([], legacy_snapshot_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(snapshots)
}

fn legacy_snapshot_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<LegacyDesktopStateSnapshot> {
    Ok(LegacyDesktopStateSnapshot {
        id: row.get(0)?,
        owner: row.get(1)?,
        app_data_json: row.get(2)?,
        schema_version: row.get(3)?,
        revision: row.get(4)?,
        item_count: row.get(5)?,
        semantic_summary: row.get(6)?,
        raw_sha256: row.get(7)?,
        canonical_json_sha256: row.get(8)?,
        parent_envelope_sha256: row.get(9)?,
        envelope_sha256: row.get(10)?,
        created_at_epoch_millis: row.get(11)?,
        source: row.get(12)?,
    })
}

fn migrate_legacy_snapshot_envelopes(
    legacy_snapshots: Vec<LegacyDesktopStateSnapshot>,
) -> DesktopStateStoreResult<(
    Vec<DesktopStateSnapshot>,
    BTreeMap<(String, String), String>,
)> {
    let protected_sync_state = Vec::new();
    let sync_state_sha256 = sha256_hex(&protected_sync_state);
    let mut present_envelopes = BTreeMap::new();
    for snapshot in &legacy_snapshots {
        let key = (
            snapshot.owner.clone(),
            snapshot.envelope_sha256.to_ascii_lowercase(),
        );
        if present_envelopes.insert(key, snapshot.id).is_some() {
            return Err(integrity(
                "v1 snapshot envelopes were not unique after digest normalization",
            ));
        }
    }
    let mut envelope_mappings: BTreeMap<(String, String), String> = BTreeMap::new();
    let mut migrated = Vec::with_capacity(legacy_snapshots.len());
    for snapshot in legacy_snapshots {
        let parent_key = (
            snapshot.owner.clone(),
            snapshot.parent_envelope_sha256.to_ascii_lowercase(),
        );
        let parent_envelope_sha256 = match envelope_mappings.get(&parent_key) {
            Some(migrated_parent) => migrated_parent.clone(),
            None if present_envelopes.contains_key(&parent_key) => {
                return Err(integrity(
                    "v1 snapshot parent references a live envelope outside its insertion prefix",
                ));
            }
            None => snapshot.parent_envelope_sha256.clone(),
        };
        let envelope_sha256 = snapshot_envelope_sha256(
            &snapshot.owner,
            snapshot.schema_version,
            snapshot.revision,
            snapshot.item_count,
            &snapshot.semantic_summary,
            &snapshot.raw_sha256,
            &snapshot.canonical_json_sha256,
            &sync_state_sha256,
            &parent_envelope_sha256,
            snapshot.created_at_epoch_millis,
            &snapshot.source,
        );
        if envelope_mappings
            .insert(
                (
                    snapshot.owner.clone(),
                    snapshot.envelope_sha256.to_ascii_lowercase(),
                ),
                envelope_sha256.clone(),
            )
            .is_some()
        {
            return Err(integrity(
                "v1 snapshot envelope mapping was not unique for its owner",
            ));
        }
        migrated.push(DesktopStateSnapshot {
            id: snapshot.id,
            owner: snapshot.owner,
            app_data_json: snapshot.app_data_json,
            protected_sync_state: protected_sync_state.clone(),
            schema_version: snapshot.schema_version,
            revision: snapshot.revision,
            item_count: snapshot.item_count,
            semantic_summary: snapshot.semantic_summary,
            raw_sha256: snapshot.raw_sha256,
            canonical_json_sha256: snapshot.canonical_json_sha256,
            sync_state_sha256: sync_state_sha256.clone(),
            parent_envelope_sha256,
            envelope_sha256,
            created_at_epoch_millis: snapshot.created_at_epoch_millis,
            source: snapshot.source,
        });
    }
    Ok((migrated, envelope_mappings))
}

fn table_row_count(connection: &Connection, table: &str) -> DesktopStateStoreResult<i64> {
    let query = match table {
        METADATA_TABLE => "SELECT COUNT(*) FROM desktop_state_journal_metadata",
        OWNER_TABLE => "SELECT COUNT(*) FROM desktop_state_owners",
        SNAPSHOT_TABLE => "SELECT COUNT(*) FROM desktop_state_snapshots",
        QUARANTINE_TABLE => "SELECT COUNT(*) FROM desktop_state_snapshot_quarantine",
        SCOPE_MEDIA_TRANSACTION_PROOF_TABLE => {
            "SELECT COUNT(*) FROM desktop_state_scope_media_transaction_proofs"
        }
        MIGRATION_OWNER_TABLE => "SELECT COUNT(*) FROM desktop_state_owners_v2",
        MIGRATION_SNAPSHOT_TABLE => "SELECT COUNT(*) FROM desktop_state_snapshots_v2",
        MIGRATION_QUARANTINE_TABLE => "SELECT COUNT(*) FROM desktop_state_snapshot_quarantine_v2",
        MIGRATION_METADATA_TABLE => "SELECT COUNT(*) FROM desktop_state_journal_metadata_v2",
        _ => return Err(integrity("unknown journal table requested")),
    };
    connection
        .query_row(query, [], |row| row.get(0))
        .map_err(DesktopStateStoreError::from)
}

fn random_journal_id() -> DesktopStateStoreResult<String> {
    let mut bytes = [0_u8; 32];
    let mut rng = OsRng;
    rng.try_fill_bytes(&mut bytes)
        .map_err(|error| integrity(format!("operating-system randomness failed: {error}")))?;
    Ok(hex_bytes(&bytes))
}

fn valid_journal_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn insert_journal_metadata(
    connection: &Connection,
    table: &str,
    commit_sequence: i64,
) -> DesktopStateStoreResult<()> {
    if commit_sequence < 0 {
        return Err(integrity("journal commit sequence cannot be negative"));
    }
    let statement = match table {
        METADATA_TABLE => {
            "INSERT INTO desktop_state_journal_metadata(
                 singleton, journal_id, commit_sequence
             ) VALUES (?1, ?2, ?3)"
        }
        MIGRATION_METADATA_TABLE => {
            "INSERT INTO desktop_state_journal_metadata_v2(
                 singleton, journal_id, commit_sequence
             ) VALUES (?1, ?2, ?3)"
        }
        _ => return Err(integrity("unknown journal metadata table requested")),
    };
    let journal_id = random_journal_id()?;
    let changed = connection.execute(
        statement,
        params![METADATA_SINGLETON_KEY, journal_id, commit_sequence],
    )?;
    if changed != 1 {
        return Err(integrity(
            "journal metadata insert changed an unexpected row count",
        ));
    }
    let inserted = read_journal_metadata_from_table(connection, table)?;
    if inserted.journal_id != journal_id || inserted.commit_sequence != commit_sequence {
        return Err(integrity(
            "journal metadata changed before initialization completed",
        ));
    }
    Ok(())
}

fn read_journal_metadata_from_table(
    connection: &Connection,
    table: &str,
) -> DesktopStateStoreResult<DesktopStateJournalEvidence> {
    let query = match table {
        METADATA_TABLE => {
            "SELECT singleton, journal_id, commit_sequence
             FROM desktop_state_journal_metadata ORDER BY singleton"
        }
        MIGRATION_METADATA_TABLE => {
            "SELECT singleton, journal_id, commit_sequence
             FROM desktop_state_journal_metadata_v2 ORDER BY singleton"
        }
        _ => return Err(integrity("unknown journal metadata table requested")),
    };
    let mut statement = connection.prepare(query)?;
    let mut rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if rows.len() != 1 {
        return Err(integrity(
            "journal metadata must contain exactly one singleton row",
        ));
    }
    let (singleton, journal_id, commit_sequence) = rows
        .pop()
        .ok_or_else(|| integrity("journal metadata singleton disappeared"))?;
    if singleton != METADATA_SINGLETON_KEY || !valid_journal_id(&journal_id) || commit_sequence < 0
    {
        return Err(integrity("journal metadata singleton is malformed"));
    }
    Ok(DesktopStateJournalEvidence {
        journal_id,
        commit_sequence,
    })
}

fn snapshot_table_identity(table: &str) -> DesktopStateStoreResult<(&'static str, &'static str)> {
    match table {
        SNAPSHOT_TABLE => Ok((
            SNAPSHOT_TABLE,
            "SELECT COALESCE(MAX(id), 0) FROM desktop_state_snapshots",
        )),
        MIGRATION_SNAPSHOT_TABLE => Ok((
            MIGRATION_SNAPSHOT_TABLE,
            "SELECT COALESCE(MAX(id), 0) FROM desktop_state_snapshots_v2",
        )),
        _ => Err(integrity("unknown snapshot sequence table requested")),
    }
}

fn read_snapshot_autoincrement_sequence(
    connection: &Connection,
    table: &str,
) -> DesktopStateStoreResult<i64> {
    let (table_name, max_id_query) = snapshot_table_identity(table)?;
    let table_sql = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![table_name],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| integrity(format!("snapshot table is missing: {table_name}")))?;
    if !table_sql.to_ascii_uppercase().contains("AUTOINCREMENT") {
        return Err(integrity(format!(
            "snapshot table does not preserve monotonic row ids: {table_name}"
        )));
    }
    let maximum_id: i64 = connection.query_row(max_id_query, [], |row| row.get(0))?;
    if maximum_id < 0 {
        return Err(integrity("snapshot table contains a negative maximum id"));
    }
    let mut statement =
        connection.prepare("SELECT seq FROM sqlite_sequence WHERE name = ?1 ORDER BY rowid")?;
    let sequences = statement
        .query_map(params![table_name], |row| row.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    match sequences.as_slice() {
        [] if maximum_id == 0 => Ok(0),
        [] => Err(integrity(
            "snapshot AUTOINCREMENT sequence is missing for existing rows",
        )),
        [sequence] if *sequence >= maximum_id && *sequence >= 0 => Ok(*sequence),
        [_] => Err(integrity(
            "snapshot AUTOINCREMENT sequence is below the retained row ids",
        )),
        _ => Err(integrity(
            "snapshot AUTOINCREMENT sequence contains duplicate table entries",
        )),
    }
}

fn set_snapshot_autoincrement_sequence(
    connection: &Connection,
    table: &str,
    sequence: i64,
) -> DesktopStateStoreResult<()> {
    let (table_name, max_id_query) = snapshot_table_identity(table)?;
    let maximum_id: i64 = connection.query_row(max_id_query, [], |row| row.get(0))?;
    if sequence < 0 || sequence < maximum_id {
        return Err(integrity(
            "requested snapshot sequence is below the migrated row ids",
        ));
    }
    let deleted = connection.execute(
        "DELETE FROM sqlite_sequence WHERE name = ?1",
        params![table_name],
    )?;
    if deleted > 1 {
        return Err(integrity(
            "snapshot AUTOINCREMENT sequence contained duplicate table entries",
        ));
    }
    let inserted = connection.execute(
        "INSERT INTO sqlite_sequence(name, seq) VALUES (?1, ?2)",
        params![table_name, sequence],
    )?;
    if inserted != 1 || read_snapshot_autoincrement_sequence(connection, table)? != sequence {
        return Err(integrity(
            "snapshot AUTOINCREMENT sequence was not preserved during migration",
        ));
    }
    Ok(())
}

fn read_journal_evidence(
    connection: &Connection,
) -> DesktopStateStoreResult<DesktopStateJournalEvidence> {
    let evidence = read_journal_metadata_from_table(connection, METADATA_TABLE)?;
    let snapshot_sequence = read_snapshot_autoincrement_sequence(connection, SNAPSHOT_TABLE)?;
    if evidence.commit_sequence != snapshot_sequence {
        return Err(integrity(format!(
            "journal commit sequence {} does not match snapshot sequence {snapshot_sequence}",
            evidence.commit_sequence
        )));
    }
    Ok(evidence)
}

/// Consume one monotonic sequence value for an atomic metadata-only update.
/// Retained snapshot IDs already permit gaps after pruning or quarantine. Keep
/// SQLite's sequence and independent journal evidence equal, without copying a
/// potentially large application snapshot just to persist an attachment list.
fn advance_metadata_commit_sequence(
    connection: &Connection,
    expected_before: &DesktopStateJournalEvidence,
) -> DesktopStateStoreResult<()> {
    if &read_journal_evidence(connection)? != expected_before {
        return Err(integrity("journal evidence changed before metadata commit"));
    }
    let next = expected_before
        .commit_sequence
        .checked_add(1)
        .ok_or_else(|| integrity("journal commit sequence is exhausted"))?;
    set_snapshot_autoincrement_sequence(connection, SNAPSHOT_TABLE, next)?;
    advance_commit_sequence(connection, expected_before)
}

fn advance_commit_sequence(
    connection: &Connection,
    expected_before: &DesktopStateJournalEvidence,
) -> DesktopStateStoreResult<()> {
    let before = read_journal_metadata_from_table(connection, METADATA_TABLE)?;
    if &before != expected_before {
        return Err(integrity(
            "journal evidence changed before the snapshot sequence was committed",
        ));
    }
    let next_sequence = before
        .commit_sequence
        .checked_add(1)
        .ok_or_else(|| integrity("journal commit sequence is exhausted"))?;
    let snapshot_sequence = read_snapshot_autoincrement_sequence(connection, SNAPSHOT_TABLE)?;
    if snapshot_sequence != next_sequence {
        return Err(integrity(format!(
            "new snapshot sequence {snapshot_sequence} did not advance exactly once from {}",
            before.commit_sequence
        )));
    }
    let changed = connection.execute(
        "UPDATE desktop_state_journal_metadata
         SET commit_sequence = ?1
         WHERE singleton = ?2 AND journal_id = ?3 AND commit_sequence = ?4",
        params![
            next_sequence,
            METADATA_SINGLETON_KEY,
            before.journal_id,
            before.commit_sequence,
        ],
    )?;
    if changed != 1 {
        return Err(integrity(
            "journal commit sequence update changed an unexpected row count",
        ));
    }
    let after = read_journal_evidence(connection)?;
    if after.journal_id != before.journal_id || after.commit_sequence != next_sequence {
        return Err(integrity(
            "journal evidence changed unexpectedly while committing a snapshot",
        ));
    }
    Ok(())
}

fn verify_required_schema(connection: &Connection) -> DesktopStateStoreResult<()> {
    mirror_provenance::verify_schema(connection)?;
    verify_table_column_layout(
        connection,
        PRIVACY_TABLE,
        &["owner", "policy_json", "binding_sha256", "cleanup_pending"],
    )?;
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version != STORE_SCHEMA_VERSION {
        return Err(integrity(format!(
            "expected journal schema {STORE_SCHEMA_VERSION}, found {version}"
        )));
    }
    verify_table_column_layout(
        connection,
        METADATA_TABLE,
        &["singleton", "journal_id", "commit_sequence"],
    )?;
    verify_table_column_layout(
        connection,
        OWNER_TABLE,
        &[
            "owner",
            "initialized_at_epoch_millis",
            "first_envelope_sha256",
            "registry_sha256",
        ],
    )?;
    verify_table_column_layout(
        connection,
        SNAPSHOT_TABLE,
        &[
            "id",
            "owner",
            "app_data_json",
            "protected_sync_state",
            "schema_version",
            "revision",
            "item_count",
            "semantic_summary",
            "raw_sha256",
            "canonical_json_sha256",
            "sync_state_sha256",
            "parent_envelope_sha256",
            "envelope_sha256",
            "created_at_epoch_millis",
            "source",
        ],
    )?;
    verify_table_column_layout(
        connection,
        QUARANTINE_TABLE,
        &[
            "id",
            "snapshot_id",
            "owner",
            "app_data_json",
            "protected_sync_state",
            "schema_version",
            "revision",
            "item_count",
            "semantic_summary",
            "stored_raw_sha256",
            "observed_raw_sha256",
            "stored_canonical_json_sha256",
            "observed_canonical_json_sha256",
            "sync_state_sha256",
            "observed_sync_state_sha256",
            "parent_envelope_sha256",
            "stored_envelope_sha256",
            "observed_envelope_sha256",
            "created_at_epoch_millis",
            "source",
            "reason",
            "quarantined_at_epoch_millis",
        ],
    )?;
    verify_scope_media_transaction_proof_table_layout(connection)?;
    Ok(())
}

fn verify_scope_media_transaction_proof_table_layout(
    connection: &Connection,
) -> DesktopStateStoreResult<()> {
    verify_table_column_layout(
        connection,
        SCOPE_MEDIA_TRANSACTION_PROOF_TABLE,
        &[
            "source",
            "snapshot_id",
            "pinned_snapshot_id",
            "owner",
            "raw_sha256",
            "sync_state_sha256",
            "envelope_sha256",
        ],
    )
}

fn verify_v1_required_schema(connection: &Connection) -> DesktopStateStoreResult<()> {
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version != 1 {
        return Err(integrity(format!(
            "expected legacy journal schema 1, found {version}"
        )));
    }
    verify_table_column_layout(
        connection,
        OWNER_TABLE,
        &[
            "owner",
            "initialized_at_epoch_millis",
            "first_envelope_sha256",
            "registry_sha256",
        ],
    )?;
    verify_table_column_layout(
        connection,
        SNAPSHOT_TABLE,
        &[
            "id",
            "owner",
            "app_data_json",
            "schema_version",
            "revision",
            "item_count",
            "semantic_summary",
            "raw_sha256",
            "canonical_json_sha256",
            "parent_envelope_sha256",
            "envelope_sha256",
            "created_at_epoch_millis",
            "source",
        ],
    )?;
    verify_table_column_layout(
        connection,
        QUARANTINE_TABLE,
        &[
            "id",
            "snapshot_id",
            "owner",
            "app_data_json",
            "schema_version",
            "revision",
            "item_count",
            "semantic_summary",
            "stored_raw_sha256",
            "observed_raw_sha256",
            "stored_canonical_json_sha256",
            "observed_canonical_json_sha256",
            "parent_envelope_sha256",
            "stored_envelope_sha256",
            "observed_envelope_sha256",
            "created_at_epoch_millis",
            "source",
            "reason",
            "quarantined_at_epoch_millis",
        ],
    )?;
    Ok(())
}

fn verify_table_column_layout(
    connection: &Connection,
    table: &str,
    expected_columns: &[&str],
) -> DesktopStateStoreResult<()> {
    let query = match table {
        METADATA_TABLE => "PRAGMA table_info(desktop_state_journal_metadata)",
        OWNER_TABLE => "PRAGMA table_info(desktop_state_owners)",
        SNAPSHOT_TABLE => "PRAGMA table_info(desktop_state_snapshots)",
        PRIVACY_TABLE => "PRAGMA table_info(desktop_state_privacy_barriers)",
        mirror_provenance::TABLE => "PRAGMA table_info(desktop_state_mirror_provenance)",
        QUARANTINE_TABLE => "PRAGMA table_info(desktop_state_snapshot_quarantine)",
        SCOPE_MEDIA_TRANSACTION_PROOF_TABLE => {
            "PRAGMA table_info(desktop_state_scope_media_transaction_proofs)"
        }
        _ => return Err(integrity("unknown journal table layout requested")),
    };
    let mut statement = connection.prepare(query)?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if columns.len() != expected_columns.len()
        || columns
            .iter()
            .zip(expected_columns)
            .any(|(actual, expected)| actual.as_str() != *expected)
    {
        return Err(integrity(format!(
            "journal table {table} has an unexpected column layout"
        )));
    }
    let table_sql = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![table],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| integrity(format!("required journal table is missing: {table}")))?;
    if !table_sql.to_ascii_uppercase().contains("STRICT") {
        return Err(integrity(format!(
            "required journal table is not STRICT: {table}"
        )));
    }
    Ok(())
}

fn verify_sqlite_integrity(connection: &Connection) -> DesktopStateStoreResult<()> {
    verify_quick_check(connection)?;
    verify_sqlite_full_integrity(connection)
}

// integrity_check includes quick_check's checks and also validates unique
// constraints and index consistency. A reserved save needs only this full pass.
fn verify_sqlite_full_integrity(connection: &Connection) -> DesktopStateStoreResult<()> {
    #[cfg(test)]
    SAVE_AUDIT_COUNTS.with(|counts| {
        let mut value = counts.get();
        value.integrity += 1;
        counts.set(value);
    });
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
        Err(integrity(format!(
            "SQLite integrity_check returned {}",
            messages.join("; ")
        )))
    }
}

fn verify_quick_check(connection: &Connection) -> DesktopStateStoreResult<()> {
    #[cfg(test)]
    SAVE_AUDIT_COUNTS.with(|counts| {
        let mut value = counts.get();
        value.quick += 1;
        counts.set(value);
    });
    let result: String = connection.query_row("PRAGMA quick_check(1)", [], |row| row.get(0))?;
    if result.eq_ignore_ascii_case("ok") {
        Ok(())
    } else {
        Err(integrity(format!("SQLite quick_check returned {result}")))
    }
}

fn verify_foreign_keys(connection: &Connection) -> DesktopStateStoreResult<()> {
    let enabled: i64 = connection.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
    if enabled != 1 {
        return Err(integrity("SQLite foreign_keys is not enabled"));
    }
    let mut statement = connection.prepare("PRAGMA foreign_key_check")?;
    if statement.query([])?.next()?.is_some() {
        return Err(integrity("SQLite foreign_key_check found a violation"));
    }
    Ok(())
}

fn analyze_app_data_json(
    raw: &str,
    compatibility_now: i64,
) -> DesktopStateStoreResult<AppDataAnalysis> {
    analyze_app_data_json_with_raw_digest(raw, compatibility_now).map(|(analysis, _)| analysis)
}

fn analyze_app_data_json_with_raw_digest(
    raw: &str,
    compatibility_now: i64,
) -> DesktopStateStoreResult<(AppDataAnalysis, String)> {
    cached_app_data_analysis(raw, || {
        analyze_app_data_json_uncached(raw, compatibility_now)
    })
}

fn analyze_app_data_json_with_value(
    raw: &str,
    compatibility_now: i64,
) -> DesktopStateStoreResult<(AppDataAnalysis, Value)> {
    let mut parsed_value = None;
    let (analysis, _) = cached_app_data_analysis(raw, || {
        let (analysis, value) = analyze_app_data_json_uncached_with_value(raw, compatibility_now)?;
        parsed_value = Some(value);
        Ok(analysis)
    })?;
    // A cold analysis already parsed these exact raw bytes. Keep its document
    // only in this call; the shared cache retains semantic metadata alone.
    // A cache hit still needs a document for the caller, but no second parse.
    let value = match parsed_value {
        Some(value) => value,
        None => serde_json::from_str(raw)?,
    };
    Ok((analysis, value))
}

fn cached_app_data_analysis(
    raw: &str,
    compute: impl FnOnce() -> DesktopStateStoreResult<AppDataAnalysis>,
) -> DesktopStateStoreResult<(AppDataAnalysis, String)> {
    // Opening, recovering and appending a journal can inspect the same history
    // repeatedly. Only reuse the expensive pure JSON analysis. The key hashes
    // the actual bytes just read, never a digest supplied by the stored row.
    // Owner, raw/sync digests and envelope checks still run for every snapshot.
    // This analysis does not depend on the compatibility clock: every `now`
    // branch in AppData::sanitized ultimately forwards to sanitize_timestamp,
    // which deliberately returns value.max(0) and ignores the local clock.
    // Missing IDs, version repair, duplicate selection and serialization use
    // persisted values only. Keep this cache local to pure analysis; mutation
    // clocks and other caches retain their own contracts. The uncached path
    // still performs the full raw-token/unknown-field/serialization checks.
    type AnalysisCache = std::collections::VecDeque<(String, AppDataAnalysis)>;
    static CACHE: std::sync::OnceLock<std::sync::Mutex<AnalysisCache>> = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(AnalysisCache::new()));
    let key = sha256_hex(raw.as_bytes());
    if let Ok(mut entries) = cache.lock() {
        if let Some(index) = entries
            .iter()
            .position(|(cached_key, _)| cached_key == &key)
        {
            // Every save rereads retained history. Keep those hot analyses even
            // while new timer states enter the bounded cache.
            let entry = entries.remove(index).expect("located analysis cache entry");
            let analysis = entry.1.clone();
            entries.push_back(entry);
            return Ok((analysis, key));
        }
    }
    let analysis = compute()?;
    if let Ok(mut entries) = cache.lock() {
        if !entries.iter().any(|(cached_key, _)| cached_key == &key) {
            if entries.len() >= 128 {
                entries.pop_front();
            }
            entries.push_back((key.clone(), analysis.clone()));
        }
    }
    Ok((analysis, key))
}

fn analyze_app_data_json_uncached(
    raw: &str,
    compatibility_now: i64,
) -> DesktopStateStoreResult<AppDataAnalysis> {
    analyze_app_data_json_uncached_with_value(raw, compatibility_now).map(|(analysis, _)| analysis)
}

fn analyze_app_data_json_uncached_with_value(
    raw: &str,
    compatibility_now: i64,
) -> DesktopStateStoreResult<(AppDataAnalysis, Value)> {
    let value = serde_json::from_str::<Value>(raw)?;
    let root = value
        .as_object()
        .ok_or_else(|| integrity("app data root must be a JSON object"))?;
    match app_data::app_data_json_compatibility_from_value(raw, &value, compatibility_now) {
        AppDataJsonCompatibility::LegacyMigratable | AppDataJsonCompatibility::CurrentKnown => {}
        AppDataJsonCompatibility::CurrentUnknown => {
            return Err(integrity(
                "app data contains unknown fields for the current schema",
            ));
        }
        AppDataJsonCompatibility::Future => {
            return Err(integrity(
                "app data schema is newer than this journal writer",
            ));
        }
        AppDataJsonCompatibility::Invalid => {
            return Err(integrity("app data failed semantic validation"));
        }
    }

    let schema_version = match root.get("schemaVersion") {
        None => 0,
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok()))
            .ok_or_else(|| integrity("schemaVersion is not a non-negative integer"))?,
        Some(_) => return Err(integrity("schemaVersion is not an integer")),
    };
    if schema_version < 0 {
        return Err(integrity("schemaVersion cannot be negative"));
    }

    let summary = semantic_summary(&value);
    let semantic_summary = canonical_json_string(&serde_json::to_value(&summary)?)?;
    let canonical_json = canonical_json_bytes(&value)?;
    let analysis = AppDataAnalysis {
        schema_version,
        revision: sync_core::app_data_revision_in_value(&value, 0).max(0),
        item_count: summary.recoverable_items,
        tombstone_count: summary.tombstones,
        semantic_summary,
        canonical_json_sha256: sha256_hex(&canonical_json),
    };
    Ok((analysis, value))
}

fn semantic_summary(value: &Value) -> SemanticSummary {
    let root = value.as_object();
    let categories = array_len(root.and_then(|root| root.get("categories")));
    let slots_value = root.and_then(|root| root.get("slots"));
    let slots = array_len(slots_value);
    let meaningful_slots = slots_value
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter(|value| slot_is_meaningful(value))
                .count() as i64
        })
        .unwrap_or(0);
    let slot_order_entries = array_len(root.and_then(|root| root.get("slotOrder")));
    let sessions = array_len(root.and_then(|root| root.get("sessions")));
    let archived_tasks = array_len(root.and_then(|root| root.get("archivedTasks")));
    let note_folders = array_len(root.and_then(|root| root.get("noteFolders")));
    let notes_value = root.and_then(|root| root.get("notes"));
    let notes = array_len(notes_value);
    let mut note_revisions = 0_i64;
    let mut note_attachments = 0_i64;
    let mut active_note_versions = 0_i64;
    let mut deleted_note_versions = 0_i64;
    let mut active_note_version_attachments = 0_i64;
    let mut deleted_note_version_attachments = 0_i64;
    if let Some(notes) = notes_value.and_then(Value::as_array) {
        for note in notes {
            note_revisions = note_revisions.saturating_add(array_len(note.get("revisions")));
            note_attachments = note_attachments.saturating_add(array_len(note.get("attachments")));
            if let Some(revisions) = note.get("revisions").and_then(Value::as_array) {
                for revision in revisions {
                    note_attachments =
                        note_attachments.saturating_add(array_len(revision.get("attachments")));
                }
            }
            if let Some(versions) = note.get("versions").and_then(Value::as_array) {
                for version in versions {
                    let attachment_count = array_len(version.get("attachments"));
                    if version
                        .get("deletedAtEpochMillis")
                        .is_some_and(|deleted_at| !deleted_at.is_null())
                    {
                        deleted_note_versions = deleted_note_versions.saturating_add(1);
                        deleted_note_version_attachments =
                            deleted_note_version_attachments.saturating_add(attachment_count);
                    } else {
                        active_note_versions = active_note_versions.saturating_add(1);
                        active_note_version_attachments =
                            active_note_version_attachments.saturating_add(attachment_count);
                    }
                }
            }
        }
    }
    let tombstones = array_len(root.and_then(|root| root.get("tombstones")));
    let sync_conflicts = array_len(root.and_then(|root| root.get("syncConflictHistory")));
    let finance_day_entries = root
        .and_then(|root| root.get("financeProfile"))
        .and_then(|finance| finance.get("dailyLedgers"))
        .and_then(Value::as_object)
        .map(|values| values.len() as i64)
        .unwrap_or(0);
    let finance_month_entries = root
        .and_then(|root| root.get("financeProfile"))
        .and_then(|finance| finance.get("monthlySnapshots"))
        .and_then(Value::as_object)
        .map(|values| values.len() as i64)
        .unwrap_or(0);
    let finance_day_revision_entries = root
        .and_then(|root| root.get("financeDayLedgerRevisions"))
        .and_then(Value::as_object)
        .map(|values| values.len() as i64)
        .unwrap_or(0);
    let finance_month_revision_entries = root
        .and_then(|root| root.get("financeMonthSnapshotRevisions"))
        .and_then(Value::as_object)
        .map(|values| values.len() as i64)
        .unwrap_or(0);
    let recoverable_items = [
        categories,
        meaningful_slots,
        slot_order_entries,
        sessions,
        archived_tasks,
        note_folders,
        notes,
        note_revisions,
        note_attachments,
        active_note_versions,
        deleted_note_versions,
        active_note_version_attachments,
        deleted_note_version_attachments,
        tombstones,
        sync_conflicts,
        finance_day_entries.max(finance_day_revision_entries),
        finance_month_entries.max(finance_month_revision_entries),
    ]
    .into_iter()
    .fold(0_i64, i64::saturating_add);
    SemanticSummary {
        categories,
        slots,
        meaningful_slots,
        slot_order_entries,
        sessions,
        archived_tasks,
        note_folders,
        notes,
        note_revisions,
        note_attachments,
        active_note_versions,
        deleted_note_versions,
        active_note_version_attachments,
        deleted_note_version_attachments,
        tombstones,
        sync_conflicts,
        finance_day_entries,
        finance_month_entries,
        finance_day_revision_entries,
        finance_month_revision_entries,
        recoverable_items,
    }
}

fn semantic_metadata_matches(
    stored_item_count: i64,
    stored_summary: &str,
    analysis: &AppDataAnalysis,
) -> bool {
    if stored_item_count == analysis.item_count && stored_summary == analysis.semantic_summary {
        return true;
    }

    // v2 journal rows written before note-version accounting must remain
    // recoverable. Their envelope continues to bind the original summary and
    // item_count; compatibility is limited to the one exact projection of the
    // current summary with the newly introduced fields removed.
    let Ok(stored_value) = serde_json::from_str::<Value>(stored_summary) else {
        return false;
    };
    if canonical_json_string(&stored_value).ok().as_deref() != Some(stored_summary) {
        return false;
    }
    let Ok(mut legacy_projection) = serde_json::from_str::<Value>(&analysis.semantic_summary)
    else {
        return false;
    };
    let Some(summary) = legacy_projection.as_object_mut() else {
        return false;
    };
    let version_item_count = [
        "activeNoteVersions",
        "deletedNoteVersions",
        "activeNoteVersionAttachments",
        "deletedNoteVersionAttachments",
    ]
    .into_iter()
    .try_fold(0_i64, |total, key| {
        summary
            .remove(key)
            .and_then(|value| value.as_i64())
            .filter(|value| *value >= 0)
            .map(|value| total.saturating_add(value))
    });
    let Some(version_item_count) = version_item_count else {
        return false;
    };
    let Some(legacy_item_count) = analysis.item_count.checked_sub(version_item_count) else {
        return false;
    };
    summary.insert(
        "recoverableItems".to_string(),
        Value::Number(legacy_item_count.into()),
    );
    stored_item_count == legacy_item_count && stored_value == legacy_projection
}

fn slot_is_meaningful(value: &&Value) -> bool {
    let Some(slot) = value.as_object() else {
        return false;
    };
    ["title", "note"].into_iter().any(|key| {
        slot.get(key)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
    }) || ["accumulatedMillis", "runningSinceEpochMillis"]
        .into_iter()
        .any(|key| {
            slot.get(key)
                .and_then(Value::as_i64)
                .is_some_and(|value| value != 0)
        })
}

fn array_len(value: Option<&Value>) -> i64 {
    value
        .and_then(Value::as_array)
        .map(|values| values.len().min(i64::MAX as usize) as i64)
        .unwrap_or(0)
}

fn summary_guarded_item_count(summary: &str, fallback: i64) -> i64 {
    serde_json::from_str::<Value>(summary)
        .ok()
        .and_then(|value| {
            let recoverable = value.get("recoverableItems")?.as_i64()?;
            let slot_order = value.get("slotOrderEntries")?.as_i64()?;
            Some(recoverable.saturating_sub(slot_order.max(0)))
        })
        .unwrap_or(fallback)
}

fn suspicious_item_drop(
    parent: &DesktopStateSnapshot,
    incoming: &AppDataAnalysis,
) -> Option<(i64, i64)> {
    // Every healthy snapshot contains the full slot-order vector even when the
    // user has not created any data.  Counting those structural entries in the
    // ratio can hide a complete loss of a modest session/note history (for
    // example 10 sessions becomes 28 -> 18 instead of 10 -> 0).  Keep the
    // stored item_count unchanged for envelope verification, but exclude that
    // invariant vector when deciding whether a local transition is suspicious.
    // Re-analyse the parent's raw snapshot with the current summary schema. An
    // older row may legitimately have a summary written before note-version
    // counts existed; relying on that old item_count would hide a 100 -> 1
    // version-stack collapse on the next local save.
    let previous_items = analyze_app_data_json(&parent.app_data_json, 0)
        .ok()
        .map(|analysis| summary_guarded_item_count(&analysis.semantic_summary, analysis.item_count))
        .unwrap_or_else(|| summary_guarded_item_count(&parent.semantic_summary, parent.item_count));
    let incoming_items =
        summary_guarded_item_count(&incoming.semantic_summary, incoming.item_count);
    if previous_items < SUSPICIOUS_DROP_MIN_ITEMS {
        return None;
    }
    let previous_tombstones = serde_json::from_str::<Value>(&parent.semantic_summary)
        .ok()
        .and_then(|value| value.get("tombstones").and_then(Value::as_i64))
        .unwrap_or(0);
    (incoming_items.saturating_add(SUSPICIOUS_DROP_ABSOLUTE_ITEMS) < previous_items
        && incoming_items.saturating_mul(2) < previous_items
        && incoming.tombstone_count <= previous_tombstones)
        .then_some((previous_items, incoming_items))
}

fn source_allows_destructive_transition(source: &str) -> bool {
    let source = source.to_ascii_lowercase();
    matches!(
        source.as_str(),
        "restore"
            | "verified_restore"
            | "journal_restore"
            | "migration"
            | "verified_migration"
            | "legacy_migration"
            | "download"
            | "verified_download"
            | "server_download"
            | "sync_download"
            | "journal_recovery"
    )
}

fn source_allows_journal_reseed(source: &str) -> bool {
    source_allows_destructive_transition(source)
}

fn validate_owner(owner: &str) -> DesktopStateStoreResult<()> {
    if owner.is_empty() {
        return Err(integrity("snapshot owner cannot be empty"));
    }
    if owner.len() > MAX_OWNER_BYTES {
        return Err(integrity(format!(
            "snapshot owner exceeds {MAX_OWNER_BYTES} bytes"
        )));
    }
    Ok(())
}

fn normalized_source(source: &str) -> DesktopStateStoreResult<String> {
    let source = source.trim();
    if source.is_empty() {
        return Err(integrity("snapshot source cannot be empty"));
    }
    if source.len() > MAX_SOURCE_BYTES || source.chars().any(char::is_control) {
        return Err(integrity("snapshot source is invalid"));
    }
    if source.starts_with("scope_media_tx:")
        && scope_media_transaction_id_from_source(source).is_none()
    {
        return Err(integrity("scope-media transaction source is invalid"));
    }
    Ok(source.to_string())
}

fn scope_media_transaction_id_from_source(source: &str) -> Option<&str> {
    let transaction_id = source.strip_prefix("scope_media_tx:")?;
    (transaction_id.len() == 64
        && transaction_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()))
    .then_some(transaction_id)
}

fn read_owner_registry(
    connection: &Connection,
    owner: &str,
) -> DesktopStateStoreResult<Option<DesktopStateOwnerRegistry>> {
    connection
        .query_row(
            "SELECT owner, initialized_at_epoch_millis, first_envelope_sha256,
                    registry_sha256
             FROM desktop_state_owners WHERE owner = ?1",
            params![owner],
            |row| {
                Ok(DesktopStateOwnerRegistry {
                    owner: row.get(0)?,
                    initialized_at_epoch_millis: row.get(1)?,
                    first_envelope_sha256: row.get(2)?,
                    registry_sha256: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(DesktopStateStoreError::from)
}

fn owner_registry_initialized(
    connection: &Connection,
    owner: &str,
) -> DesktopStateStoreResult<bool> {
    match read_owner_registry(connection, owner)? {
        Some(registry) => {
            verify_owner_registry(&registry, owner)?;
            Ok(true)
        }
        None if owner_has_snapshot_evidence(connection, owner)? => Err(integrity(
            "snapshot evidence exists without its persistent owner registry",
        )),
        None => Ok(false),
    }
}

fn verify_owner_registry(
    registry: &DesktopStateOwnerRegistry,
    expected_owner: &str,
) -> DesktopStateStoreResult<()> {
    validate_owner(&registry.owner)?;
    if registry.owner != expected_owner {
        return Err(integrity("owner registry belongs to a different workspace"));
    }
    if registry.initialized_at_epoch_millis < 0
        || !valid_sha256_hex(&registry.first_envelope_sha256)
        || !valid_sha256_hex(&registry.registry_sha256)
    {
        return Err(integrity("owner registry metadata is invalid"));
    }
    let observed = owner_registry_sha256(
        &registry.owner,
        registry.initialized_at_epoch_millis,
        &registry.first_envelope_sha256,
    );
    if !registry.registry_sha256.eq_ignore_ascii_case(&observed) {
        return Err(integrity("owner registry digest mismatch"));
    }
    Ok(())
}

fn verify_all_owner_registries(connection: &Connection) -> DesktopStateStoreResult<()> {
    let mut statement = connection.prepare(
        "SELECT owner, initialized_at_epoch_millis, first_envelope_sha256,
                registry_sha256
         FROM desktop_state_owners ORDER BY owner",
    )?;
    let registries = statement
        .query_map([], |row| {
            Ok(DesktopStateOwnerRegistry {
                owner: row.get(0)?,
                initialized_at_epoch_millis: row.get(1)?,
                first_envelope_sha256: row.get(2)?,
                registry_sha256: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for registry in registries {
        verify_owner_registry(&registry, &registry.owner)?;
    }
    Ok(())
}

fn insert_owner_registry(
    connection: &Connection,
    owner: &str,
    initialized_at_epoch_millis: i64,
    first_envelope_sha256: &str,
) -> DesktopStateStoreResult<()> {
    let initialized_at_epoch_millis = initialized_at_epoch_millis.max(0);
    if !valid_sha256_hex(first_envelope_sha256) {
        return Err(integrity("first owner envelope digest is invalid"));
    }
    let registry_sha256 =
        owner_registry_sha256(owner, initialized_at_epoch_millis, first_envelope_sha256);
    let changed = connection.execute(
        "INSERT INTO desktop_state_owners(
             owner, initialized_at_epoch_millis, first_envelope_sha256,
             registry_sha256
         ) VALUES (?1, ?2, ?3, ?4)",
        params![
            owner,
            initialized_at_epoch_millis,
            first_envelope_sha256,
            registry_sha256
        ],
    )?;
    if changed != 1 {
        return Err(integrity(
            "owner registry insert changed an unexpected row count",
        ));
    }
    let registry = read_owner_registry(connection, owner)?
        .ok_or_else(|| integrity("owner registry disappeared before verification"))?;
    verify_owner_registry(&registry, owner)
}

fn owner_has_snapshot_evidence(
    connection: &Connection,
    owner: &str,
) -> DesktopStateStoreResult<bool> {
    let exists: i64 = connection.query_row(
        "SELECT CASE WHEN
             EXISTS(SELECT 1 FROM desktop_state_snapshots WHERE owner = ?1)
             OR EXISTS(SELECT 1 FROM desktop_state_snapshot_quarantine WHERE owner = ?1)
         THEN 1 ELSE 0 END",
        params![owner],
        |row| row.get(0),
    )?;
    Ok(exists == 1)
}

struct VerifiedSaveHistory<'transaction> {
    owner_initialized: bool,
    parent: Option<DesktopStateSnapshot>,
    privacy: Option<privacy::PreparedPrivacyTransition<'transaction>>,
}

fn record_snapshot_in_transaction(
    transaction: &Connection,
    owner: &str,
    app_data_json: &str,
    protected_sync_state: &[u8],
    now_epoch_millis: i64,
    source: &str,
    declaration_scope: &str,
    declarations: &[DesktopSealedMediaDeclaration],
    analysis: AppDataAnalysis,
    raw_sha256: String,
    sync_state_sha256: String,
    checked_history: Option<VerifiedSaveHistory<'_>>,
) -> DesktopStateStoreResult<DesktopStateSnapshot> {
    record_snapshot_in_transaction_observed(
        transaction,
        owner,
        app_data_json,
        protected_sync_state,
        now_epoch_millis,
        source,
        declaration_scope,
        declarations,
        analysis,
        raw_sha256,
        sync_state_sha256,
        checked_history,
        None,
        &mut NoopDesktopSaveObserver,
    )
}

fn record_snapshot_in_transaction_observed(
    transaction: &Connection,
    owner: &str,
    app_data_json: &str,
    protected_sync_state: &[u8],
    now_epoch_millis: i64,
    source: &str,
    declaration_scope: &str,
    declarations: &[DesktopSealedMediaDeclaration],
    analysis: AppDataAnalysis,
    raw_sha256: String,
    sync_state_sha256: String,
    checked_history: Option<VerifiedSaveHistory<'_>>,
    mirror_parent: Option<&mut Option<DesktopStateSnapshot>>,
    observer: &mut impl DesktopSaveObserver,
) -> DesktopStateStoreResult<DesktopStateSnapshot> {
    let scope_media_transaction = scope_media_transaction_id_from_source(source).is_some();
    if scope_media_transaction {
        if let Some(proof) = read_scope_media_transaction_proof(transaction, source)? {
            let existing = verified_scope_media_transaction_snapshot(transaction, &proof)?
                .ok_or_else(|| integrity("scope-media transaction source is already completed"))?;
            if existing.owner == owner
                && existing.raw_sha256.eq_ignore_ascii_case(&raw_sha256)
                && existing.app_data_json == app_data_json
                && existing
                    .sync_state_sha256
                    .eq_ignore_ascii_case(&sync_state_sha256)
                && existing.protected_sync_state == protected_sync_state
            {
                return Ok(existing);
            }
            return Err(integrity(
                "scope-media transaction source is already bound to another snapshot",
            ));
        }
    }
    let checked_history = match checked_history {
        Some(checked) => checked,
        None => {
            quarantine_invalid_rows_preserving_evidence_in_transaction(
                transaction,
                Some(owner),
                now_epoch_millis,
            )?;
            VerifiedSaveHistory {
                owner_initialized: owner_registry_initialized(transaction, owner)?,
                parent: latest_valid_in_transaction(transaction, owner, now_epoch_millis)?,
                privacy: None,
            }
        }
    };
    let owner_was_initialized = checked_history.owner_initialized;
    if scope_media_transaction {
        if scope_media_transaction_source_was_quarantined(transaction, source)? {
            return Err(integrity(
                "scope-media transaction source already has quarantined evidence",
            ));
        }
    }

    let retain_original_parent = mirror_parent.is_some();
    let (parent, privacy_changed, original_parent) = observer.measure(
        DesktopSavePhase::DestructiveDropAndPrivacyCommit,
        || -> DesktopStateStoreResult<_> {
            // Reject a destructive save before publishing its privacy intent. Row
            // quarantine may be committed independently; new fences may not.
            let prepared_privacy = match checked_history.privacy {
                Some(prepared) => prepared,
                None => privacy::prepare_privacy_transition(transaction, owner, app_data_json)?,
            };
            let prospective_policy = prepared_privacy.policy();
            let checked_parent = checked_history.parent;
            if checked_parent.is_none()
                && owner_was_initialized
                && !source_allows_journal_reseed(source)
            {
                return Err(DesktopStateStoreError::InitializedOwnerWithoutValidSnapshot);
            }
            if let Some(checked_parent) = checked_parent.as_ref() {
                let mut checked_parent = std::borrow::Cow::Borrowed(checked_parent);
                // A legitimate seal/deletion can shrink note recovery data. Compare
                // the same prospective privacy projection without writing history.
                if let Some(projected) =
                    prospective_policy.redact_json(&checked_parent.app_data_json)?
                {
                    let projected_analysis = analyze_app_data_json(&projected, 0)?;
                    let checked_parent = checked_parent.to_mut();
                    checked_parent.app_data_json = projected;
                    checked_parent.semantic_summary = projected_analysis.semantic_summary;
                    checked_parent.item_count = projected_analysis.item_count;
                }
                if let Some((previous_items, incoming_items)) =
                    suspicious_item_drop(&checked_parent, &analysis)
                        .filter(|_| !source_allows_destructive_transition(source))
                {
                    return Err(DesktopStateStoreError::SuspiciousItemDrop {
                        previous_items,
                        incoming_items,
                    });
                }
            }
            let privacy_changed = prepared_privacy.commit()?;
            let (parent, original_parent) = if privacy_changed {
                (
                    latest_valid_in_transaction(transaction, owner, now_epoch_millis)?,
                    if retain_original_parent {
                        checked_parent
                    } else {
                        None
                    },
                )
            } else {
                (checked_parent, None)
            };
            Ok((parent, privacy_changed, original_parent))
        },
    )?;
    if let Some(candidate) = parent.as_ref() {
        if !scope_media_transaction
            && candidate.raw_sha256.eq_ignore_ascii_case(&raw_sha256)
            && candidate.app_data_json == app_data_json
            && candidate
                .sync_state_sha256
                .eq_ignore_ascii_case(&sync_state_sha256)
            && candidate.protected_sync_state == protected_sync_state
        {
            let media_changed = privacy::record_media_declarations(
                transaction,
                owner,
                declaration_scope,
                app_data_json,
                declarations,
            )?;
            if privacy_changed || media_changed {
                let evidence = read_journal_evidence(transaction)?;
                advance_metadata_commit_sequence(transaction, &evidence)?;
            }
            let recorded = candidate.clone();
            if let Some(destination) = mirror_parent {
                *destination = if privacy_changed {
                    original_parent
                } else {
                    parent
                };
            }
            return Ok(recorded);
        }
    } else if owner_was_initialized && !source_allows_journal_reseed(source) {
        return Err(DesktopStateStoreError::InitializedOwnerWithoutValidSnapshot);
    }

    let parent_envelope_sha256 = parent
        .as_ref()
        .map(|snapshot| snapshot.envelope_sha256.clone())
        .unwrap_or_default();
    let envelope_sha256 = snapshot_envelope_sha256(
        owner,
        analysis.schema_version,
        analysis.revision,
        analysis.item_count,
        &analysis.semantic_summary,
        &raw_sha256,
        &analysis.canonical_json_sha256,
        &sync_state_sha256,
        &parent_envelope_sha256,
        now_epoch_millis,
        source,
    );
    let evidence_before_insert = read_journal_evidence(transaction)?;

    let recorded = observer.measure(DesktopSavePhase::InsertReadbackVerify, || {
        transaction.execute(
            "INSERT INTO desktop_state_snapshots(
             owner, app_data_json, protected_sync_state, schema_version,
             revision, item_count, semantic_summary, raw_sha256,
             canonical_json_sha256, sync_state_sha256,
             parent_envelope_sha256, envelope_sha256,
             created_at_epoch_millis, source
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                   ?13, ?14)",
            params![
                owner,
                app_data_json,
                protected_sync_state,
                analysis.schema_version,
                analysis.revision,
                analysis.item_count,
                analysis.semantic_summary,
                raw_sha256,
                analysis.canonical_json_sha256,
                sync_state_sha256,
                parent_envelope_sha256,
                envelope_sha256,
                now_epoch_millis,
                source,
            ],
        )?;
        let id = transaction.last_insert_rowid();
        let recorded = read_snapshot_by_id(transaction, id)?
            .ok_or_else(|| integrity("inserted snapshot disappeared before verification"))?;
        verify_snapshot(&recorded, Some(owner), 0)?;
        Ok::<_, DesktopStateStoreError>(recorded)
    })?;
    if !owner_was_initialized {
        insert_owner_registry(
            transaction,
            owner,
            now_epoch_millis,
            &recorded.envelope_sha256,
        )?;
    }
    if scope_media_transaction {
        insert_scope_media_transaction_proof(transaction, &recorded)?;
    }
    observer.measure(DesktopSavePhase::HistoryPrune, || {
        prune_owner_history(transaction, owner, now_epoch_millis)
    })?;
    privacy::record_media_declarations(
        transaction,
        owner,
        declaration_scope,
        app_data_json,
        declarations,
    )?;
    advance_commit_sequence(transaction, &evidence_before_insert)?;
    // Return ownership of the pre-privacy parent only to the controlled save.
    // Its caller still rechecks the actual row before authorizing a mirror.
    if let Some(destination) = mirror_parent {
        *destination = if privacy_changed {
            original_parent
        } else {
            parent
        };
    }
    Ok(recorded)
}

fn latest_valid_in_transaction(
    connection: &Connection,
    owner: &str,
    compatibility_now: i64,
) -> DesktopStateStoreResult<Option<DesktopStateSnapshot>> {
    let mut statement = connection.prepare(
        "SELECT id, owner, app_data_json, protected_sync_state, schema_version,
                revision, item_count, semantic_summary, raw_sha256,
                canonical_json_sha256, sync_state_sha256,
                parent_envelope_sha256, envelope_sha256,
                created_at_epoch_millis, source
         FROM desktop_state_snapshots
         WHERE owner = ?1
         ORDER BY id DESC",
    )?;
    let rows = statement.query_map(params![owner], snapshot_from_row)?;
    for snapshot in rows {
        let snapshot = snapshot?;
        if verify_snapshot(&snapshot, Some(owner), compatibility_now).is_ok() {
            return Ok(Some(snapshot));
        }
    }
    Ok(None)
}

fn read_snapshot_by_raw_digest(
    connection: &Connection,
    owner: &str,
    raw_sha256: &str,
    compatibility_now: i64,
) -> DesktopStateStoreResult<Option<DesktopStateSnapshot>> {
    let snapshot = connection
        .query_row(
            "SELECT id, owner, app_data_json, protected_sync_state, schema_version,
                    revision, item_count, semantic_summary, raw_sha256,
                    canonical_json_sha256, sync_state_sha256,
                    parent_envelope_sha256, envelope_sha256,
                    created_at_epoch_millis, source
             FROM desktop_state_snapshots
             WHERE owner = ?1 AND raw_sha256 = ?2
             ORDER BY id DESC LIMIT 1",
            params![owner, raw_sha256],
            snapshot_from_row,
        )
        .optional()?;
    snapshot
        .map(|snapshot| {
            verify_snapshot(&snapshot, Some(owner), compatibility_now)?;
            Ok(snapshot)
        })
        .transpose()
}

fn read_snapshot_by_id(
    connection: &Connection,
    id: i64,
) -> DesktopStateStoreResult<Option<DesktopStateSnapshot>> {
    connection
        .query_row(
            "SELECT id, owner, app_data_json, protected_sync_state, schema_version,
                    revision, item_count, semantic_summary, raw_sha256,
                    canonical_json_sha256, sync_state_sha256,
                    parent_envelope_sha256, envelope_sha256,
                    created_at_epoch_millis, source
             FROM desktop_state_snapshots WHERE id = ?1",
            params![id],
            snapshot_from_row,
        )
        .optional()
        .map_err(DesktopStateStoreError::from)
}

fn read_scope_media_transaction_proof(
    connection: &Connection,
    source: &str,
) -> DesktopStateStoreResult<Option<ScopeMediaTransactionProof>> {
    connection
        .query_row(
            "SELECT source, snapshot_id, pinned_snapshot_id, owner, raw_sha256,
                    sync_state_sha256, envelope_sha256
             FROM desktop_state_scope_media_transaction_proofs WHERE source = ?1",
            params![source],
            scope_media_transaction_proof_from_row,
        )
        .optional()
        .map_err(DesktopStateStoreError::from)
}

fn scope_media_transaction_proof_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ScopeMediaTransactionProof> {
    Ok(ScopeMediaTransactionProof {
        source: row.get(0)?,
        snapshot_id: row.get(1)?,
        pinned_snapshot_id: row.get(2)?,
        owner: row.get(3)?,
        raw_sha256: row.get(4)?,
        sync_state_sha256: row.get(5)?,
        envelope_sha256: row.get(6)?,
    })
}

fn verify_scope_media_transaction_proof(
    proof: &ScopeMediaTransactionProof,
) -> DesktopStateStoreResult<()> {
    validate_owner(&proof.owner)?;
    if scope_media_transaction_id_from_source(&proof.source).is_none()
        || normalized_source(&proof.source)? != proof.source
        || proof.snapshot_id <= 0
        || proof
            .pinned_snapshot_id
            .is_some_and(|snapshot_id| snapshot_id != proof.snapshot_id)
    {
        return Err(integrity("scope-media transaction proof is malformed"));
    }
    for digest in [
        proof.raw_sha256.as_str(),
        proof.sync_state_sha256.as_str(),
        proof.envelope_sha256.as_str(),
    ] {
        if !valid_sha256_hex(digest) || digest.bytes().any(|byte| byte.is_ascii_uppercase()) {
            return Err(integrity(
                "scope-media transaction proof contains an invalid digest",
            ));
        }
    }
    Ok(())
}

fn verify_scope_media_transaction_proof_matches_snapshot(
    proof: &ScopeMediaTransactionProof,
    snapshot: &DesktopStateSnapshot,
) -> DesktopStateStoreResult<()> {
    verify_scope_media_transaction_proof(proof)?;
    verify_snapshot(snapshot, Some(&proof.owner), 0)?;
    if proof.snapshot_id != snapshot.id
        || proof.source != snapshot.source
        || !proof.raw_sha256.eq_ignore_ascii_case(&snapshot.raw_sha256)
        || !proof
            .sync_state_sha256
            .eq_ignore_ascii_case(&snapshot.sync_state_sha256)
        || !proof
            .envelope_sha256
            .eq_ignore_ascii_case(&snapshot.envelope_sha256)
    {
        return Err(integrity(
            "scope-media transaction proof diverged from its bound snapshot",
        ));
    }
    Ok(())
}

fn verified_scope_media_transaction_snapshot(
    connection: &Connection,
    proof: &ScopeMediaTransactionProof,
) -> DesktopStateStoreResult<Option<DesktopStateSnapshot>> {
    verify_scope_media_transaction_proof(proof)?;
    let Some(snapshot_id) = proof.pinned_snapshot_id else {
        return Ok(None);
    };
    let snapshot = read_snapshot_by_id(connection, snapshot_id)?
        .ok_or_else(|| integrity("pinned scope-media transaction snapshot disappeared"))?;
    verify_scope_media_transaction_proof_matches_snapshot(proof, &snapshot)?;
    if !owner_registry_initialized(connection, &proof.owner)? {
        return Err(integrity(
            "scope-media transaction snapshot has no verified owner registry",
        ));
    }
    Ok(Some(snapshot))
}

fn insert_scope_media_transaction_proof(
    connection: &Connection,
    snapshot: &DesktopStateSnapshot,
) -> DesktopStateStoreResult<()> {
    if scope_media_transaction_id_from_source(&snapshot.source).is_none() {
        return Err(integrity(
            "cannot register a non-scope snapshot as a scope-media transaction",
        ));
    }
    verify_snapshot(snapshot, Some(&snapshot.owner), 0)?;
    let proof = ScopeMediaTransactionProof {
        source: snapshot.source.clone(),
        snapshot_id: snapshot.id,
        pinned_snapshot_id: Some(snapshot.id),
        owner: snapshot.owner.clone(),
        raw_sha256: snapshot.raw_sha256.to_ascii_lowercase(),
        sync_state_sha256: snapshot.sync_state_sha256.to_ascii_lowercase(),
        envelope_sha256: snapshot.envelope_sha256.to_ascii_lowercase(),
    };
    verify_scope_media_transaction_proof_matches_snapshot(&proof, snapshot)?;
    let changed = connection.execute(
        "INSERT INTO desktop_state_scope_media_transaction_proofs(
             source, snapshot_id, pinned_snapshot_id, owner, raw_sha256,
             sync_state_sha256, envelope_sha256
         ) VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6)",
        params![
            proof.source,
            proof.snapshot_id,
            proof.owner,
            proof.raw_sha256,
            proof.sync_state_sha256,
            proof.envelope_sha256,
        ],
    )?;
    if changed != 1 {
        return Err(integrity(
            "scope-media transaction proof insert changed an unexpected row count",
        ));
    }
    let inserted = read_scope_media_transaction_proof(connection, &snapshot.source)?
        .ok_or_else(|| integrity("inserted scope-media transaction proof disappeared"))?;
    if inserted != proof {
        return Err(integrity(
            "inserted scope-media transaction proof changed before commit",
        ));
    }
    Ok(())
}

fn backfill_scope_media_transaction_proofs(connection: &Connection) -> DesktopStateStoreResult<()> {
    let mut statement = connection.prepare(
        "SELECT id FROM desktop_state_snapshots
         WHERE source GLOB 'scope_media_tx:*' ORDER BY id",
    )?;
    let snapshot_ids = statement
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for snapshot_id in snapshot_ids {
        let snapshot = read_snapshot_by_id(connection, snapshot_id)?
            .ok_or_else(|| integrity("scope-media migration snapshot disappeared"))?;
        verify_snapshot(&snapshot, Some(&snapshot.owner), 0)?;
        if scope_media_transaction_id_from_source(&snapshot.source).is_none() {
            return Err(integrity(
                "existing scope-media transaction source is malformed",
            ));
        }
        if let Some(proof) = read_scope_media_transaction_proof(connection, &snapshot.source)? {
            verify_scope_media_transaction_proof_matches_snapshot(&proof, &snapshot)?;
        } else {
            insert_scope_media_transaction_proof(connection, &snapshot)?;
        }
    }
    Ok(())
}

fn verify_all_scope_media_transaction_proofs(
    connection: &Connection,
) -> DesktopStateStoreResult<()> {
    let mut statement = connection.prepare(
        "SELECT source, snapshot_id, pinned_snapshot_id, owner, raw_sha256,
                sync_state_sha256, envelope_sha256
         FROM desktop_state_scope_media_transaction_proofs ORDER BY source",
    )?;
    let proofs = statement
        .query_map([], scope_media_transaction_proof_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for proof in proofs {
        verify_scope_media_transaction_proof(&proof)?;
        if let Some(snapshot) = read_snapshot_by_id(connection, proof.snapshot_id)? {
            verify_scope_media_transaction_proof_matches_snapshot(&proof, &snapshot)?;
            if !owner_registry_initialized(connection, &proof.owner)? {
                return Err(integrity(
                    "scope-media transaction proof has no verified owner registry",
                ));
            }
        } else if proof.pinned_snapshot_id.is_some() {
            return Err(integrity(
                "pinned scope-media transaction snapshot disappeared",
            ));
        }
    }
    Ok(())
}

fn scope_media_transaction_source_was_quarantined(
    connection: &Connection,
    source: &str,
) -> DesktopStateStoreResult<bool> {
    connection
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM desktop_state_snapshot_quarantine
                 WHERE source = ?1 LIMIT 1
             )",
            params![source],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count != 0)
        .map_err(DesktopStateStoreError::from)
}

fn snapshot_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DesktopStateSnapshot> {
    Ok(DesktopStateSnapshot {
        id: row.get(0)?,
        owner: row.get(1)?,
        app_data_json: row.get(2)?,
        protected_sync_state: row.get(3)?,
        schema_version: row.get(4)?,
        revision: row.get(5)?,
        item_count: row.get(6)?,
        semantic_summary: row.get(7)?,
        raw_sha256: row.get(8)?,
        canonical_json_sha256: row.get(9)?,
        sync_state_sha256: row.get(10)?,
        parent_envelope_sha256: row.get(11)?,
        envelope_sha256: row.get(12)?,
        created_at_epoch_millis: row.get(13)?,
        source: row.get(14)?,
    })
}

fn verify_all_v2_snapshots(connection: &Connection) -> DesktopStateStoreResult<()> {
    let mut statement = connection.prepare(
        "SELECT id, owner, app_data_json, protected_sync_state, schema_version,
                revision, item_count, semantic_summary, raw_sha256,
                canonical_json_sha256, sync_state_sha256,
                parent_envelope_sha256, envelope_sha256,
                created_at_epoch_millis, source
         FROM desktop_state_snapshots ORDER BY owner, id",
    )?;
    let snapshots = statement
        .query_map([], snapshot_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    for snapshot in snapshots {
        verify_snapshot(&snapshot, Some(&snapshot.owner), 0)?;
    }
    Ok(())
}

fn verify_legacy_snapshot(
    snapshot: &LegacyDesktopStateSnapshot,
    expected_owner: Option<&str>,
    compatibility_now: i64,
) -> DesktopStateStoreResult<()> {
    validate_owner(&snapshot.owner)?;
    if expected_owner.is_some_and(|owner| owner != snapshot.owner) {
        return Err(integrity(
            "legacy snapshot owner does not match requested workspace",
        ));
    }
    if snapshot.id <= 0
        || snapshot.schema_version < 0
        || snapshot.revision < 0
        || snapshot.item_count < 0
        || snapshot.created_at_epoch_millis < 0
    {
        return Err(integrity(
            "legacy snapshot contains negative or invalid metadata",
        ));
    }
    if normalized_source(&snapshot.source)? != snapshot.source {
        return Err(integrity("legacy snapshot source is not in canonical form"));
    }
    for (label, hash) in [
        ("raw JSON", snapshot.raw_sha256.as_str()),
        ("canonical JSON", snapshot.canonical_json_sha256.as_str()),
        ("envelope", snapshot.envelope_sha256.as_str()),
    ] {
        if !valid_sha256_hex(hash) {
            return Err(integrity(format!(
                "legacy snapshot {label} digest is invalid"
            )));
        }
    }
    if !snapshot.parent_envelope_sha256.is_empty()
        && !valid_sha256_hex(&snapshot.parent_envelope_sha256)
    {
        return Err(integrity(
            "legacy snapshot parent envelope digest is invalid",
        ));
    }

    let (analysis, observed_raw_sha256) =
        analyze_app_data_json_with_raw_digest(&snapshot.app_data_json, compatibility_now)?;
    if !snapshot
        .raw_sha256
        .eq_ignore_ascii_case(&observed_raw_sha256)
        || !snapshot
            .canonical_json_sha256
            .eq_ignore_ascii_case(&analysis.canonical_json_sha256)
        || snapshot.schema_version != analysis.schema_version
        || snapshot.revision != analysis.revision
        || !semantic_metadata_matches(snapshot.item_count, &snapshot.semantic_summary, &analysis)
    {
        return Err(integrity(
            "legacy snapshot content or semantic metadata diverged",
        ));
    }
    let observed_envelope = legacy_snapshot_envelope_sha256(
        &snapshot.owner,
        snapshot.schema_version,
        snapshot.revision,
        snapshot.item_count,
        &snapshot.semantic_summary,
        &snapshot.raw_sha256,
        &snapshot.canonical_json_sha256,
        &snapshot.parent_envelope_sha256,
        snapshot.created_at_epoch_millis,
        &snapshot.source,
    );
    if !snapshot
        .envelope_sha256
        .eq_ignore_ascii_case(&observed_envelope)
    {
        return Err(integrity("legacy snapshot envelope digest mismatch"));
    }
    Ok(())
}

fn verify_snapshot(
    snapshot: &DesktopStateSnapshot,
    expected_owner: Option<&str>,
    compatibility_now: i64,
) -> DesktopStateStoreResult<()> {
    validate_owner(&snapshot.owner)?;
    if expected_owner.is_some_and(|owner| owner != snapshot.owner) {
        return Err(integrity(
            "snapshot owner does not match requested workspace",
        ));
    }
    if snapshot.id <= 0
        || snapshot.schema_version < 0
        || snapshot.revision < 0
        || snapshot.item_count < 0
        || snapshot.created_at_epoch_millis < 0
    {
        return Err(integrity("snapshot contains negative or invalid metadata"));
    }
    if normalized_source(&snapshot.source)? != snapshot.source {
        return Err(integrity("snapshot source is not in canonical form"));
    }
    for (label, hash) in [
        ("raw JSON", snapshot.raw_sha256.as_str()),
        ("canonical JSON", snapshot.canonical_json_sha256.as_str()),
        ("protected sync state", snapshot.sync_state_sha256.as_str()),
        ("envelope", snapshot.envelope_sha256.as_str()),
    ] {
        if !valid_sha256_hex(hash) {
            return Err(integrity(format!("snapshot {label} digest is invalid")));
        }
    }
    if !snapshot.parent_envelope_sha256.is_empty()
        && !valid_sha256_hex(&snapshot.parent_envelope_sha256)
    {
        return Err(integrity("snapshot parent envelope digest is invalid"));
    }

    let (analysis, observed_raw_sha256) =
        analyze_app_data_json_with_raw_digest(&snapshot.app_data_json, compatibility_now)?;
    let observed_sync_state_sha256 = sha256_hex(&snapshot.protected_sync_state);
    if !snapshot
        .sync_state_sha256
        .eq_ignore_ascii_case(&observed_sync_state_sha256)
    {
        return Err(integrity("snapshot protected sync state digest mismatch"));
    }
    if !snapshot
        .raw_sha256
        .eq_ignore_ascii_case(&observed_raw_sha256)
        || !snapshot
            .canonical_json_sha256
            .eq_ignore_ascii_case(&analysis.canonical_json_sha256)
        || snapshot.schema_version != analysis.schema_version
        || snapshot.revision != analysis.revision
        || !semantic_metadata_matches(snapshot.item_count, &snapshot.semantic_summary, &analysis)
    {
        return Err(integrity("snapshot content or semantic metadata diverged"));
    }

    let observed_envelope = snapshot_envelope_sha256(
        &snapshot.owner,
        snapshot.schema_version,
        snapshot.revision,
        snapshot.item_count,
        &snapshot.semantic_summary,
        &snapshot.raw_sha256,
        &snapshot.canonical_json_sha256,
        &snapshot.sync_state_sha256,
        &snapshot.parent_envelope_sha256,
        snapshot.created_at_epoch_millis,
        &snapshot.source,
    );
    if !snapshot
        .envelope_sha256
        .eq_ignore_ascii_case(&observed_envelope)
    {
        return Err(integrity("snapshot envelope digest mismatch"));
    }
    Ok(())
}

fn quarantine_invalid_rows(
    connection: &mut Connection,
    owner: Option<&str>,
    now_epoch_millis: i64,
) -> DesktopStateStoreResult<usize> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let quarantined = quarantine_invalid_rows_preserving_evidence_in_transaction(
        &transaction,
        owner,
        now_epoch_millis.max(0),
    )?;
    transaction.commit()?;
    Ok(quarantined)
}

fn quarantine_invalid_rows_preserving_evidence_in_transaction(
    connection: &Connection,
    owner: Option<&str>,
    now_epoch_millis: i64,
) -> DesktopStateStoreResult<usize> {
    quarantine_invalid_rows_preserving_evidence_with_head_in_transaction(
        connection,
        owner,
        now_epoch_millis,
        None,
    )
    .map(|(quarantined, _)| quarantined)
}

fn quarantine_invalid_rows_preserving_evidence_with_head_in_transaction(
    connection: &Connection,
    owner: Option<&str>,
    now_epoch_millis: i64,
    retain_owner: Option<&str>,
) -> DesktopStateStoreResult<(usize, Option<DesktopStateSnapshot>)> {
    let evidence_before = read_journal_evidence(connection)?;
    let (quarantined, head) = quarantine_invalid_rows_in_transaction(
        connection,
        owner,
        now_epoch_millis.max(0),
        retain_owner,
    )?;
    let evidence_after = read_journal_evidence(connection)?;
    if evidence_after != evidence_before {
        return Err(integrity(
            "journal evidence changed during snapshot quarantine",
        ));
    }
    // Quarantine writes can fire triggers that change a previously visited
    // row. Only a read-only audit can supply a head without another row read.
    Ok((quarantined, if quarantined == 0 { head } else { None }))
}

fn quarantine_invalid_rows_in_transaction(
    connection: &Connection,
    owner: Option<&str>,
    now_epoch_millis: i64,
    retain_owner: Option<&str>,
) -> DesktopStateStoreResult<(usize, Option<DesktopStateSnapshot>)> {
    #[cfg(test)]
    SAVE_AUDIT_COUNTS.with(|counts| {
        let mut value = counts.get();
        value.history += 1;
        counts.set(value);
    });
    // Ordinary audits keep only the current document. A save may additionally
    // retain its owner's latest verified head, never the whole history. A scan
    // can span many owners, each with its own history budget. Keyset pagination
    // allows each corrupt row to be moved to quarantine without skipping the
    // next row or keeping a SELECT cursor open across writes.
    let sql = match owner {
        Some(_) => {
            "SELECT id, owner, app_data_json, protected_sync_state, schema_version,
                    revision, item_count, semantic_summary, raw_sha256,
                    canonical_json_sha256, sync_state_sha256,
                    parent_envelope_sha256, envelope_sha256,
                    created_at_epoch_millis, source
             FROM desktop_state_snapshots WHERE id >= ?1 AND owner = ?2
             ORDER BY id LIMIT 1"
        }
        None => {
            "SELECT id, owner, app_data_json, protected_sync_state, schema_version,
                    revision, item_count, semantic_summary, raw_sha256,
                    canonical_json_sha256, sync_state_sha256,
                    parent_envelope_sha256, envelope_sha256,
                    created_at_epoch_millis, source
             FROM desktop_state_snapshots WHERE id >= ?1 ORDER BY id LIMIT 1"
        }
    };
    let mut statement = connection.prepare(sql)?;
    let mut next_id = Some(i64::MIN);
    let mut quarantined = 0_usize;
    let mut head = None;
    while let Some(first_id) = next_id {
        let snapshot = match owner {
            Some(owner) => statement
                .query_row(params![first_id, owner], snapshot_from_row)
                .optional()?,
            None => statement
                .query_row(params![first_id], snapshot_from_row)
                .optional()?,
        };
        let Some(snapshot) = snapshot else { break };
        next_id = snapshot.id.checked_add(1);
        let Err(error) = verify_snapshot(&snapshot, Some(&snapshot.owner), 0) else {
            if quarantined == 0 && retain_owner == Some(snapshot.owner.as_str()) {
                head = Some(snapshot);
            }
            continue;
        };
        // Do not keep large pre-write bytes that cannot authorize a head after
        // quarantine. Later valid rows still receive their full audit.
        head = None;
        let observed = analyze_app_data_json(&snapshot.app_data_json, 0).ok();
        let observed_raw_sha256 = sha256_hex(snapshot.app_data_json.as_bytes());
        let observed_sync_state_sha256 = sha256_hex(&snapshot.protected_sync_state);
        let observed_canonical_json_sha256 = observed
            .as_ref()
            .map(|analysis| analysis.canonical_json_sha256.clone())
            .unwrap_or_default();
        let observed_envelope_sha256 = observed
            .as_ref()
            .map(|analysis| {
                snapshot_envelope_sha256(
                    &snapshot.owner,
                    analysis.schema_version,
                    analysis.revision,
                    analysis.item_count,
                    &analysis.semantic_summary,
                    &observed_raw_sha256,
                    &analysis.canonical_json_sha256,
                    &observed_sync_state_sha256,
                    &snapshot.parent_envelope_sha256,
                    snapshot.created_at_epoch_millis,
                    &snapshot.source,
                )
            })
            .unwrap_or_default();
        connection.execute(
            "INSERT INTO desktop_state_snapshot_quarantine(
                 snapshot_id, owner, app_data_json, protected_sync_state,
                 schema_version, revision, item_count, semantic_summary,
                 stored_raw_sha256, observed_raw_sha256,
                 stored_canonical_json_sha256, observed_canonical_json_sha256,
                 sync_state_sha256, observed_sync_state_sha256,
                 parent_envelope_sha256, stored_envelope_sha256,
                 observed_envelope_sha256, created_at_epoch_millis, source,
                 reason, quarantined_at_epoch_millis
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                       ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
            params![
                snapshot.id,
                snapshot.owner,
                snapshot.app_data_json,
                snapshot.protected_sync_state,
                snapshot.schema_version,
                snapshot.revision,
                snapshot.item_count,
                snapshot.semantic_summary,
                snapshot.raw_sha256,
                observed_raw_sha256,
                snapshot.canonical_json_sha256,
                observed_canonical_json_sha256,
                snapshot.sync_state_sha256,
                observed_sync_state_sha256,
                snapshot.parent_envelope_sha256,
                snapshot.envelope_sha256,
                observed_envelope_sha256,
                snapshot.created_at_epoch_millis,
                snapshot.source,
                error.to_string(),
                now_epoch_millis,
            ],
        )?;
        let deleted = connection.execute(
            "DELETE FROM desktop_state_snapshots WHERE id = ?1",
            params![snapshot.id],
        )?;
        if deleted != 1 {
            return Err(integrity(
                "quarantined snapshot changed an unexpected row count",
            ));
        }
        quarantined += 1;
    }
    Ok((quarantined, head))
}

fn prune_owner_history(
    connection: &Connection,
    owner: &str,
    _now_epoch_millis: i64,
) -> DesktopStateStoreResult<usize> {
    let mut statement = connection.prepare(
        "SELECT snapshots.id,
                octet_length(snapshots.app_data_json)
                    + length(snapshots.protected_sync_state),
                EXISTS(
                    SELECT 1 FROM desktop_state_scope_media_transaction_proofs proofs
                    WHERE proofs.pinned_snapshot_id = snapshots.id
                )
         FROM desktop_state_snapshots snapshots
         WHERE snapshots.owner = ?1 ORDER BY snapshots.id DESC",
    )?;
    let rows = statement
        .query_map(params![owner], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?.max(0) as u64,
                row.get::<_, i64>(2)? != 0,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut total_bytes = rows
        .iter()
        .fold(0_u64, |total, (_, bytes, _)| total.saturating_add(*bytes));
    let mut deleted = 0_usize;
    for (index, (id, bytes, pinned)) in rows.into_iter().enumerate().rev() {
        if total_bytes <= SOFT_BUDGET_BYTES_PER_OWNER {
            break;
        }
        if index < MIN_RETAINED_SNAPSHOTS_PER_OWNER || pinned {
            continue;
        }
        let changed = connection.execute(
            "DELETE FROM desktop_state_snapshots
             WHERE id = ?1 AND owner = ?2
               AND NOT EXISTS(
                   SELECT 1 FROM desktop_state_scope_media_transaction_proofs
                   WHERE pinned_snapshot_id = ?1
               )",
            params![id, owner],
        )?;
        if changed != 1 {
            return Err(integrity("snapshot GC changed an unexpected row count"));
        }
        total_bytes = total_bytes.saturating_sub(bytes);
        deleted += 1;
    }
    Ok(deleted)
}

#[allow(clippy::too_many_arguments)]
fn legacy_snapshot_envelope_sha256(
    owner: &str,
    schema_version: i64,
    revision: i64,
    item_count: i64,
    semantic_summary: &str,
    raw_sha256: &str,
    canonical_json_sha256: &str,
    parent_envelope_sha256: &str,
    created_at_epoch_millis: i64,
    source: &str,
) -> String {
    let mut digest = Sha256::new();
    digest_field(&mut digest, LEGACY_ENVELOPE_DOMAIN);
    digest_field(&mut digest, owner.as_bytes());
    digest_field(&mut digest, &schema_version.to_be_bytes());
    digest_field(&mut digest, &revision.to_be_bytes());
    digest_field(&mut digest, &item_count.to_be_bytes());
    digest_field(&mut digest, semantic_summary.as_bytes());
    digest_field(&mut digest, raw_sha256.as_bytes());
    digest_field(&mut digest, canonical_json_sha256.as_bytes());
    digest_field(&mut digest, parent_envelope_sha256.as_bytes());
    digest_field(&mut digest, &created_at_epoch_millis.to_be_bytes());
    digest_field(&mut digest, source.as_bytes());
    hex_bytes(&digest.finalize())
}

#[allow(clippy::too_many_arguments)]
fn snapshot_envelope_sha256(
    owner: &str,
    schema_version: i64,
    revision: i64,
    item_count: i64,
    semantic_summary: &str,
    raw_sha256: &str,
    canonical_json_sha256: &str,
    sync_state_sha256: &str,
    parent_envelope_sha256: &str,
    created_at_epoch_millis: i64,
    source: &str,
) -> String {
    let mut digest = Sha256::new();
    digest_field(&mut digest, ENVELOPE_DOMAIN);
    digest_field(&mut digest, owner.as_bytes());
    digest_field(&mut digest, &schema_version.to_be_bytes());
    digest_field(&mut digest, &revision.to_be_bytes());
    digest_field(&mut digest, &item_count.to_be_bytes());
    digest_field(&mut digest, semantic_summary.as_bytes());
    digest_field(&mut digest, raw_sha256.as_bytes());
    digest_field(&mut digest, canonical_json_sha256.as_bytes());
    digest_field(&mut digest, sync_state_sha256.as_bytes());
    digest_field(&mut digest, parent_envelope_sha256.as_bytes());
    digest_field(&mut digest, &created_at_epoch_millis.to_be_bytes());
    digest_field(&mut digest, source.as_bytes());
    hex_bytes(&digest.finalize())
}

fn owner_registry_sha256(
    owner: &str,
    initialized_at_epoch_millis: i64,
    first_envelope_sha256: &str,
) -> String {
    let mut digest = Sha256::new();
    digest_field(&mut digest, OWNER_REGISTRY_DOMAIN);
    digest_field(&mut digest, owner.as_bytes());
    digest_field(&mut digest, &initialized_at_epoch_millis.to_be_bytes());
    digest_field(&mut digest, first_envelope_sha256.as_bytes());
    hex_bytes(&digest.finalize())
}

fn digest_field(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}

fn canonical_json_string(value: &Value) -> DesktopStateStoreResult<String> {
    String::from_utf8(canonical_json_bytes(value)?)
        .map_err(|_| integrity("canonical JSON was not UTF-8"))
}

fn canonical_json_bytes(value: &Value) -> DesktopStateStoreResult<Vec<u8>> {
    let mut output = Vec::new();
    write_canonical_json(value, &mut output)?;
    Ok(output)
}

fn write_canonical_json(value: &Value, output: &mut Vec<u8>) -> DesktopStateStoreResult<()> {
    match value {
        Value::Null => output.extend_from_slice(b"null"),
        Value::Bool(value) => output.extend_from_slice(if *value { b"true" } else { b"false" }),
        Value::Number(value) => output.extend_from_slice(value.to_string().as_bytes()),
        Value::String(value) => serde_json::to_writer(output, value)?,
        Value::Array(values) => {
            output.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_canonical_json(value, output)?;
            }
            output.push(b']');
        }
        Value::Object(values) => {
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

fn sha256_hex(bytes: &[u8]) -> String {
    hex_bytes(&Sha256::digest(bytes))
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn valid_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn integrity(message: impl Into<String>) -> DesktopStateStoreError {
    DesktopStateStoreError::Integrity(message.into())
}

fn system_time_epoch_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    include!("desktop_state_audit_tests.rs");
    include!("desktop_state_privacy_tests.rs");
    include!("desktop_attachment_intent_tests.rs");
    include!("desktop_storage_format_tests.rs");
    include!("desktop_state_media_reference_tests.rs");
    include!("desktop_sealed_media_tests.rs");
    include!("desktop_state_mirror_provenance_tests.rs");
    include!("desktop_explicit_restore_policy_tests.rs");
    use super::*;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_store(label: &str) -> (PathBuf, DesktopStateStore) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "gridtimer_desktop_state_store_{label}_{}_{}",
            std::process::id(),
            nonce
        ));
        fs::create_dir_all(&directory).unwrap();
        let store = DesktopStateStore::open(directory.join("state_journal.sqlite3")).unwrap();
        (directory, store)
    }

    fn temp_legacy_v1_database(label: &str) -> (PathBuf, PathBuf, Connection) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "gridtimer_desktop_state_store_legacy_{label}_{}_{}",
            std::process::id(),
            nonce
        ));
        fs::create_dir_all(&directory).unwrap();
        let database_path = directory.join("state_journal.sqlite3");
        let mut connection = Connection::open(&database_path).unwrap();
        configure_connection(&connection).unwrap();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        transaction
            .execute_batch(
                "CREATE TABLE desktop_state_owners (
                     owner TEXT NOT NULL PRIMARY KEY,
                     initialized_at_epoch_millis INTEGER NOT NULL
                         CHECK(initialized_at_epoch_millis >= 0),
                     first_envelope_sha256 TEXT NOT NULL
                         CHECK(length(first_envelope_sha256) = 64),
                     registry_sha256 TEXT NOT NULL UNIQUE
                         CHECK(length(registry_sha256) = 64)
                 ) STRICT;

                 CREATE TABLE desktop_state_snapshots (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     owner TEXT NOT NULL,
                     app_data_json TEXT NOT NULL CHECK(json_valid(app_data_json)),
                     schema_version INTEGER NOT NULL CHECK(schema_version >= 0),
                     revision INTEGER NOT NULL CHECK(revision >= 0),
                     item_count INTEGER NOT NULL CHECK(item_count >= 0),
                     semantic_summary TEXT NOT NULL CHECK(json_valid(semantic_summary)),
                     raw_sha256 TEXT NOT NULL CHECK(length(raw_sha256) = 64),
                     canonical_json_sha256 TEXT NOT NULL CHECK(length(canonical_json_sha256) = 64),
                     parent_envelope_sha256 TEXT NOT NULL
                         CHECK(length(parent_envelope_sha256) IN (0, 64)),
                     envelope_sha256 TEXT NOT NULL UNIQUE CHECK(length(envelope_sha256) = 64),
                     created_at_epoch_millis INTEGER NOT NULL
                         CHECK(created_at_epoch_millis >= 0),
                     source TEXT NOT NULL,
                     FOREIGN KEY(owner) REFERENCES desktop_state_owners(owner)
                         ON UPDATE RESTRICT ON DELETE RESTRICT
                         DEFERRABLE INITIALLY DEFERRED
                 ) STRICT;

                 CREATE INDEX desktop_state_snapshots_owner_id
                     ON desktop_state_snapshots(owner, id DESC);
                 CREATE INDEX desktop_state_snapshots_owner_raw
                     ON desktop_state_snapshots(owner, raw_sha256, id DESC);

                 CREATE TABLE desktop_state_snapshot_quarantine (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     snapshot_id INTEGER NOT NULL,
                     owner TEXT NOT NULL,
                     app_data_json TEXT NOT NULL,
                     schema_version INTEGER NOT NULL,
                     revision INTEGER NOT NULL,
                     item_count INTEGER NOT NULL,
                     semantic_summary TEXT NOT NULL,
                     stored_raw_sha256 TEXT NOT NULL,
                     observed_raw_sha256 TEXT NOT NULL,
                     stored_canonical_json_sha256 TEXT NOT NULL,
                     observed_canonical_json_sha256 TEXT NOT NULL,
                     parent_envelope_sha256 TEXT NOT NULL,
                     stored_envelope_sha256 TEXT NOT NULL,
                     observed_envelope_sha256 TEXT NOT NULL,
                     created_at_epoch_millis INTEGER NOT NULL,
                     source TEXT NOT NULL,
                     reason TEXT NOT NULL,
                     quarantined_at_epoch_millis INTEGER NOT NULL,
                     FOREIGN KEY(owner) REFERENCES desktop_state_owners(owner)
                         ON UPDATE RESTRICT ON DELETE RESTRICT
                 ) STRICT;

                 CREATE INDEX desktop_state_snapshot_quarantine_owner_time
                     ON desktop_state_snapshot_quarantine(
                         owner, quarantined_at_epoch_millis DESC, id DESC
                     );",
            )
            .unwrap();
        transaction.pragma_update(None, "user_version", 1).unwrap();
        transaction.commit().unwrap();
        (directory, database_path, connection)
    }

    fn insert_legacy_owner_chain(
        connection: &mut Connection,
        owner: &str,
        states: &[String],
        initialized_at_epoch_millis: i64,
    ) -> Vec<LegacyDesktopStateSnapshot> {
        assert!(!states.is_empty());
        let mut parent_envelope_sha256 = String::new();
        let mut snapshots = Vec::new();
        for (index, state) in states.iter().enumerate() {
            let analysis = analyze_app_data_json(state, 0).unwrap();
            let raw_sha256 = sha256_hex(state.as_bytes());
            let created_at_epoch_millis = initialized_at_epoch_millis + index as i64 + 1;
            let source = "local_save".to_string();
            let envelope_sha256 = legacy_snapshot_envelope_sha256(
                owner,
                analysis.schema_version,
                analysis.revision,
                analysis.item_count,
                &analysis.semantic_summary,
                &raw_sha256,
                &analysis.canonical_json_sha256,
                &parent_envelope_sha256,
                created_at_epoch_millis,
                &source,
            );
            snapshots.push(LegacyDesktopStateSnapshot {
                id: 0,
                owner: owner.to_string(),
                app_data_json: state.clone(),
                schema_version: analysis.schema_version,
                revision: analysis.revision,
                item_count: analysis.item_count,
                semantic_summary: analysis.semantic_summary,
                raw_sha256,
                canonical_json_sha256: analysis.canonical_json_sha256,
                parent_envelope_sha256,
                envelope_sha256: envelope_sha256.clone(),
                created_at_epoch_millis,
                source,
            });
            parent_envelope_sha256 = envelope_sha256;
        }

        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let first_envelope_sha256 = snapshots[0].envelope_sha256.clone();
        transaction
            .execute(
                "INSERT INTO desktop_state_owners(
                     owner, initialized_at_epoch_millis, first_envelope_sha256,
                     registry_sha256
                 ) VALUES (?1, ?2, ?3, ?4)",
                params![
                    owner,
                    initialized_at_epoch_millis,
                    first_envelope_sha256,
                    owner_registry_sha256(
                        owner,
                        initialized_at_epoch_millis,
                        &snapshots[0].envelope_sha256,
                    ),
                ],
            )
            .unwrap();
        for snapshot in &mut snapshots {
            transaction
                .execute(
                    "INSERT INTO desktop_state_snapshots(
                         owner, app_data_json, schema_version, revision,
                         item_count, semantic_summary, raw_sha256,
                         canonical_json_sha256, parent_envelope_sha256,
                         envelope_sha256, created_at_epoch_millis, source
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                               ?11, ?12)",
                    params![
                        snapshot.owner,
                        snapshot.app_data_json,
                        snapshot.schema_version,
                        snapshot.revision,
                        snapshot.item_count,
                        snapshot.semantic_summary,
                        snapshot.raw_sha256,
                        snapshot.canonical_json_sha256,
                        snapshot.parent_envelope_sha256,
                        snapshot.envelope_sha256,
                        snapshot.created_at_epoch_millis,
                        snapshot.source,
                    ],
                )
                .unwrap();
            snapshot.id = transaction.last_insert_rowid();
        }
        transaction.commit().unwrap();
        snapshots
    }

    fn insert_legacy_quarantine_evidence(
        connection: &Connection,
        snapshot: &LegacyDesktopStateSnapshot,
    ) -> i64 {
        connection
            .execute(
                "INSERT INTO desktop_state_snapshot_quarantine(
                     snapshot_id, owner, app_data_json, schema_version, revision,
                     item_count, semantic_summary, stored_raw_sha256,
                     observed_raw_sha256, stored_canonical_json_sha256,
                     observed_canonical_json_sha256, parent_envelope_sha256,
                     stored_envelope_sha256, observed_envelope_sha256,
                     created_at_epoch_millis, source, reason,
                     quarantined_at_epoch_millis
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9, ?9,
                           ?10, ?11, ?11, ?12, ?13, ?14, ?15)",
                params![
                    snapshot.id + 10_000,
                    snapshot.owner,
                    snapshot.app_data_json,
                    snapshot.schema_version,
                    snapshot.revision,
                    snapshot.item_count,
                    snapshot.semantic_summary,
                    snapshot.raw_sha256,
                    snapshot.canonical_json_sha256,
                    snapshot.parent_envelope_sha256,
                    snapshot.envelope_sha256,
                    snapshot.created_at_epoch_millis,
                    snapshot.source,
                    "legacy quarantine evidence",
                    snapshot.created_at_epoch_millis + 100,
                ],
            )
            .unwrap();
        connection.last_insert_rowid()
    }

    fn state_with_sessions(count: usize, base_revision: i64) -> String {
        let mut value: Value =
            serde_json::from_str(&app_data::default_app_data_json(base_revision)).unwrap();
        value["sessions"] = Value::Array(
            (0..count)
                .map(|index| {
                    let revision = base_revision + index as i64 + 1;
                    json!({
                        "id": format!("session-{index}"),
                        "slotId": 1,
                        "slotTitle": "",
                        "categoryId": null,
                        "startedAtEpochMillis": revision,
                        "endedAtEpochMillis": revision + 1,
                        "durationMillis": 1,
                        "updatedAtEpochMillis": revision + 1
                    })
                })
                .collect(),
        );
        app_data::sanitize_app_data_json(&value.to_string(), base_revision + count as i64 + 2)
            .unwrap()
    }

    fn state_with_note_versions(
        active_count: usize,
        deleted_count: usize,
        attachments_per_version: usize,
        base_revision: i64,
    ) -> String {
        let mut value: Value =
            serde_json::from_str(&app_data::default_app_data_json(base_revision)).unwrap();
        let total = active_count + deleted_count;
        let versions = (0..total)
            .map(|index| {
                let revision = base_revision + index as i64 + 1;
                let attachments = (0..attachments_per_version)
                    .map(|attachment_index| {
                        json!({
                            "id": format!("version-{index}-attachment-{attachment_index}"),
                            "fileName": format!("version-{index}-{attachment_index}.bin"),
                            "createdAtEpochMillis": revision,
                            "updatedAtEpochMillis": revision
                        })
                    })
                    .collect::<Vec<_>>();
                json!({
                    "id": format!("version-{index}"),
                    "noteId": "versioned-note",
                    "sequence": index as i64 + 1,
                    "title": format!("Version {index}"),
                    "content": format!("Body {index}"),
                    "attachments": attachments,
                    "createdAtEpochMillis": revision,
                    "updatedAtEpochMillis": revision,
                    "isLatest": active_count > 0 && index + 1 == active_count,
                    "deletedAtEpochMillis": (index >= active_count).then_some(revision)
                })
            })
            .collect::<Vec<_>>();
        let latest_active_index = active_count.checked_sub(1);
        let current_attachments = latest_active_index
            .map(|version_index| {
                (0..attachments_per_version)
                    .map(|attachment_index| {
                        let revision = base_revision + version_index as i64 + 1;
                        json!({
                            "id": format!("version-{version_index}-attachment-{attachment_index}"),
                            "fileName": format!("version-{version_index}-{attachment_index}.bin"),
                            "createdAtEpochMillis": revision,
                            "updatedAtEpochMillis": revision
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        value["notes"] = json!([{
            "id": "versioned-note",
            "title": latest_active_index
                .map(|index| format!("Version {index}"))
                .unwrap_or_else(|| "Latest version".to_string()),
            "content": latest_active_index
                .map(|index| format!("Body {index}"))
                .unwrap_or_else(|| "Latest body".to_string()),
            "attachments": current_attachments,
            "versions": versions,
            "latestVersionId": if active_count == 0 {
                String::new()
            } else {
                format!("version-{}", active_count - 1)
            },
            "createdAtEpochMillis": base_revision,
            "updatedAtEpochMillis": base_revision + total as i64 + 1
        }]);
        app_data::sanitize_app_data_json(&value.to_string(), base_revision + total as i64 + 2)
            .unwrap()
    }

    fn quarantine_count(store: &DesktopStateStore) -> i64 {
        let connection = store.open_connection(false).unwrap();
        connection
            .query_row(
                "SELECT COUNT(*) FROM desktop_state_snapshot_quarantine",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn real_v1_multi_owner_chains_upgrade_losslessly_to_v2() {
        let (directory, database_path, mut connection) =
            temp_legacy_v1_database("multi_owner_upgrade");
        let owner_a_states = vec![state_with_sessions(2, 100), state_with_sessions(3, 200)];
        let owner_b_states = vec![state_with_sessions(4, 300), state_with_sessions(5, 400)];
        let owner_a_legacy =
            insert_legacy_owner_chain(&mut connection, "owner-a", &owner_a_states, 1_000);
        let owner_b_legacy =
            insert_legacy_owner_chain(&mut connection, "owner-b", &owner_b_states, 2_000);
        let quarantine_id = insert_legacy_quarantine_evidence(&connection, &owner_a_legacy[0]);
        let largest_legacy_snapshot_id = owner_a_legacy
            .iter()
            .chain(&owner_b_legacy)
            .map(|snapshot| snapshot.id)
            .max()
            .unwrap();
        // Retention may leave the allocator ahead of every live row. The
        // migration must preserve that durable history instead of reusing ids.
        connection
            .execute(
                "UPDATE sqlite_sequence SET seq = ?1 WHERE name = ?2",
                params![largest_legacy_snapshot_id + 7, SNAPSHOT_TABLE],
            )
            .unwrap();
        let legacy_commit_sequence =
            read_snapshot_autoincrement_sequence(&connection, SNAPSHOT_TABLE).unwrap();
        verify_v1_required_schema(&connection).unwrap();
        verify_sqlite_integrity(&connection).unwrap();
        drop(connection);

        let store = DesktopStateStore::open(&database_path).unwrap();
        let connection = store.open_connection(false).unwrap();
        verify_required_schema(&connection).unwrap();
        verify_foreign_keys(&connection).unwrap();
        verify_all_owner_registries(&connection).unwrap();
        verify_all_v2_snapshots(&connection).unwrap();
        verify_sqlite_integrity(&connection).unwrap();
        assert_eq!(4, table_row_count(&connection, SNAPSHOT_TABLE).unwrap());
        let migrated_evidence = read_journal_evidence(&connection).unwrap();
        assert!(valid_journal_id(&migrated_evidence.journal_id));
        assert_eq!(legacy_commit_sequence, migrated_evidence.commit_sequence);

        let owner_a_first = read_snapshot_by_id(&connection, owner_a_legacy[0].id)
            .unwrap()
            .unwrap();
        let owner_a_second = read_snapshot_by_id(&connection, owner_a_legacy[1].id)
            .unwrap()
            .unwrap();
        let owner_b_first = read_snapshot_by_id(&connection, owner_b_legacy[0].id)
            .unwrap()
            .unwrap();
        let owner_b_second = read_snapshot_by_id(&connection, owner_b_legacy[1].id)
            .unwrap()
            .unwrap();
        let empty_sync_state_sha256 = sha256_hex(&[]);
        for (legacy, migrated) in [
            (&owner_a_legacy[0], &owner_a_first),
            (&owner_a_legacy[1], &owner_a_second),
            (&owner_b_legacy[0], &owner_b_first),
            (&owner_b_legacy[1], &owner_b_second),
        ] {
            assert_eq!(legacy.id, migrated.id);
            assert_eq!(legacy.owner, migrated.owner);
            assert_eq!(legacy.app_data_json, migrated.app_data_json);
            assert!(migrated.protected_sync_state.is_empty());
            assert_eq!(empty_sync_state_sha256, migrated.sync_state_sha256);
            assert_ne!(legacy.envelope_sha256, migrated.envelope_sha256);
        }
        assert!(owner_a_first.parent_envelope_sha256.is_empty());
        assert_eq!(
            owner_a_first.envelope_sha256,
            owner_a_second.parent_envelope_sha256
        );
        assert!(owner_b_first.parent_envelope_sha256.is_empty());
        assert_eq!(
            owner_b_first.envelope_sha256,
            owner_b_second.parent_envelope_sha256
        );
        let owner_a_registry = read_owner_registry(&connection, "owner-a")
            .unwrap()
            .unwrap();
        let owner_b_registry = read_owner_registry(&connection, "owner-b")
            .unwrap()
            .unwrap();
        assert_eq!(
            owner_a_first.envelope_sha256,
            owner_a_registry.first_envelope_sha256
        );
        assert_eq!(
            owner_b_first.envelope_sha256,
            owner_b_registry.first_envelope_sha256
        );

        let quarantined: (Vec<u8>, String, String, String) = connection
            .query_row(
                "SELECT protected_sync_state, sync_state_sha256,
                        observed_sync_state_sha256, stored_envelope_sha256
                 FROM desktop_state_snapshot_quarantine WHERE id = ?1",
                params![quarantine_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert!(quarantined.0.is_empty());
        assert_eq!(empty_sync_state_sha256, quarantined.1);
        assert_eq!(empty_sync_state_sha256, quarantined.2);
        assert_eq!(owner_a_legacy[0].envelope_sha256, quarantined.3);
        drop(connection);

        let appended = store
            .record_with_sync_state(
                "owner-a",
                &owner_a_states[1],
                b"new-protected-sync-state",
                5_000,
                "local_save",
            )
            .unwrap();
        assert!(appended.id > largest_legacy_snapshot_id);
        assert_eq!(legacy_commit_sequence + 1, appended.id);
        assert_eq!(
            owner_a_second.envelope_sha256,
            appended.parent_envelope_sha256
        );
        let appended_evidence = store.journal_evidence().unwrap();
        assert_eq!(migrated_evidence.journal_id, appended_evidence.journal_id);
        assert_eq!(
            legacy_commit_sequence + 1,
            appended_evidence.commit_sequence
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn injected_v1_migration_failure_rolls_back_schema_rows_and_envelopes() {
        let (directory, database_path, mut connection) =
            temp_legacy_v1_database("migration_rollback");
        let states = vec![state_with_sessions(2, 100), state_with_sessions(3, 200)];
        let legacy = insert_legacy_owner_chain(&mut connection, "owner-a", &states, 1_000);
        let legacy_registry = read_owner_registry(&connection, "owner-a")
            .unwrap()
            .unwrap();
        let legacy_commit_sequence =
            read_snapshot_autoincrement_sequence(&connection, SNAPSHOT_TABLE).unwrap();

        let error = migrate_v1_to_v2_with_precommit_check(&mut connection, || {
            Err(integrity("injected migration failure before commit"))
        })
        .unwrap_err();
        assert!(error.to_string().contains("injected migration failure"));

        verify_v1_required_schema(&connection).unwrap();
        verify_foreign_keys(&connection).unwrap();
        verify_all_owner_registries(&connection).unwrap();
        verify_sqlite_integrity(&connection).unwrap();
        assert_eq!(legacy, read_all_legacy_snapshots(&connection).unwrap());
        assert_eq!(
            legacy_commit_sequence,
            read_snapshot_autoincrement_sequence(&connection, SNAPSHOT_TABLE).unwrap()
        );
        assert_eq!(
            legacy_registry,
            read_owner_registry(&connection, "owner-a")
                .unwrap()
                .unwrap()
        );
        for table in [
            METADATA_TABLE,
            MIGRATION_METADATA_TABLE,
            MIGRATION_OWNER_TABLE,
            MIGRATION_SNAPSHOT_TABLE,
            MIGRATION_QUARANTINE_TABLE,
            SCOPE_MEDIA_TRANSACTION_PROOF_TABLE,
        ] {
            let exists: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    params![table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(0, exists);
        }
        drop(connection);

        let store = DesktopStateStore::open(&database_path).unwrap();
        assert_eq!(
            states[1],
            store
                .latest_valid("owner-a", 5_000)
                .unwrap()
                .unwrap()
                .app_data_json
        );
        let connection = store.open_connection(false).unwrap();
        verify_required_schema(&connection).unwrap();
        verify_sqlite_integrity(&connection).unwrap();
        assert_eq!(
            legacy_commit_sequence,
            read_journal_evidence(&connection).unwrap().commit_sequence
        );
        drop(connection);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn journal_identity_is_stable_and_sequence_advances_only_for_new_snapshots() {
        let (directory, store) = temp_store("journal_evidence_monotonic");
        let database_path = store.database_path().to_path_buf();
        let initial = store.journal_evidence().unwrap();
        assert!(valid_journal_id(&initial.journal_id));
        assert_eq!(0, initial.commit_sequence);

        drop(store);
        let store = DesktopStateStore::open(&database_path).unwrap();
        assert_eq!(initial, store.journal_evidence().unwrap());

        let first_state = state_with_sessions(2, 100);
        let second_state = state_with_sessions(3, 200);
        let first = store
            .record_with_sync_state(
                "guest",
                &first_state,
                b"protected-sync-state-1",
                1_000,
                "local_save",
            )
            .unwrap();
        let after_first = store.journal_evidence().unwrap();
        assert_eq!(initial.journal_id, after_first.journal_id);
        assert_eq!(1, after_first.commit_sequence);
        assert_eq!(first.id, after_first.commit_sequence);

        let duplicate = store
            .record_with_sync_state(
                "guest",
                &first_state,
                b"protected-sync-state-1",
                2_000,
                "local_save",
            )
            .unwrap();
        assert_eq!(first.id, duplicate.id);
        assert_eq!(after_first, store.journal_evidence().unwrap());

        let second = store
            .record_with_sync_state(
                "guest",
                &second_state,
                b"protected-sync-state-2",
                3_000,
                "local_save",
            )
            .unwrap();
        let after_second = store.journal_evidence().unwrap();
        assert!(second.id > first.id);
        assert_eq!(after_first.journal_id, after_second.journal_id);
        assert_eq!(
            after_first.commit_sequence + 1,
            after_second.commit_sequence
        );
        assert_eq!(second.id, after_second.commit_sequence);

        drop(store);
        let reopened = DesktopStateStore::open(&database_path).unwrap();
        assert_eq!(after_second, reopened.journal_evidence().unwrap());
        drop(reopened);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn older_database_copy_has_the_same_identity_but_a_lower_sequence() {
        let (directory, store) = temp_store("older_database_copy");
        let first_state = state_with_sessions(2, 100);
        let second_state = state_with_sessions(3, 200);
        store
            .record("guest", &first_state, 1_000, "local_save")
            .unwrap();
        let evidence_at_copy = store.journal_evidence().unwrap();

        let older_copy_path = directory.join("older_state_journal.sqlite3");
        let source = store.open_connection(false).unwrap();
        let mut target = Connection::open(&older_copy_path).unwrap();
        {
            let backup = rusqlite::backup::Backup::new(&source, &mut target).unwrap();
            backup
                .run_to_completion(128, std::time::Duration::from_millis(1), None)
                .unwrap();
        }
        drop(target);
        drop(source);

        store
            .record("guest", &second_state, 2_000, "local_save")
            .unwrap();
        let current_evidence = store.journal_evidence().unwrap();
        let older_store = DesktopStateStore::open(&older_copy_path).unwrap();
        let older_evidence = older_store.journal_evidence().unwrap();

        assert_eq!(evidence_at_copy, older_evidence);
        assert_eq!(current_evidence.journal_id, older_evidence.journal_id);
        assert!(older_evidence.commit_sequence < current_evidence.commit_sequence);
        drop(older_store);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn commit_sequence_update_failure_rolls_back_the_snapshot_and_owner() {
        let (directory, store) = temp_store("commit_sequence_rollback");
        let connection = store.open_connection(false).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER reject_journal_sequence_update
                 BEFORE UPDATE OF commit_sequence ON desktop_state_journal_metadata
                 BEGIN
                     SELECT RAISE(ABORT, 'injected journal sequence failure');
                 END;",
            )
            .unwrap();
        drop(connection);

        let state = state_with_sessions(2, 100);
        let error = store
            .record("guest", &state, 1_000, "local_save")
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("injected journal sequence failure"));
        assert_eq!(0, store.journal_evidence().unwrap().commit_sequence);
        let connection = store.open_connection(false).unwrap();
        assert_eq!(0, table_row_count(&connection, SNAPSHOT_TABLE).unwrap());
        assert_eq!(0, table_row_count(&connection, OWNER_TABLE).unwrap());
        connection
            .execute_batch("DROP TRIGGER reject_journal_sequence_update")
            .unwrap();
        drop(connection);

        store.record("guest", &state, 2_000, "local_save").unwrap();
        assert_eq!(1, store.journal_evidence().unwrap().commit_sequence);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn identical_app_json_with_new_sync_state_commits_a_new_recoverable_head() {
        let (directory, store) = temp_store("sync_state_new_head");
        let state = state_with_sessions(3, 100);
        let first_sync_state = b"dpapi-protected-ack-generation-6";
        let second_sync_state = b"dpapi-protected-ack-generation-7";

        let first = store
            .record_with_sync_state("guest", &state, first_sync_state, 1_000, "local_save")
            .unwrap();
        let second = store
            .record_with_sync_state("guest", &state, second_sync_state, 1_000, "local_save")
            .unwrap();
        let duplicate = store
            .record_with_sync_state("guest", &state, second_sync_state, 2_000, "local_save")
            .unwrap();

        assert!(second.id > first.id);
        assert_ne!(first.envelope_sha256, second.envelope_sha256);
        assert_eq!(first.envelope_sha256, second.parent_envelope_sha256);
        assert_eq!(second.id, duplicate.id);
        let recovered = store.latest_valid("guest", 3_000).unwrap().unwrap();
        assert_eq!(second.id, recovered.id);
        assert_eq!(state, recovered.app_data_json);
        assert_eq!(second_sync_state.as_slice(), recovered.protected_sync_state);
        assert_eq!(sha256_hex(second_sync_state), recovered.sync_state_sha256);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn one_byte_sync_state_corruption_quarantines_the_whole_row_and_falls_back() {
        let (directory, store) = temp_store("sync_state_corruption");
        let backup_json = state_with_sessions(3, 100);
        let newest_json = state_with_sessions(4, 200);
        let backup_sync_state = b"dpapi-protected-proof-a";
        let newest_sync_state = b"dpapi-protected-proof-b";
        let backup = store
            .record_with_sync_state(
                "guest",
                &backup_json,
                backup_sync_state,
                1_000,
                "local_save",
            )
            .unwrap();
        let newest = store
            .record_with_sync_state(
                "guest",
                &newest_json,
                newest_sync_state,
                2_000,
                "local_save",
            )
            .unwrap();
        let evidence_before_quarantine = store.journal_evidence().unwrap();
        let mut corrupted_sync_state = newest_sync_state.to_vec();
        corrupted_sync_state[0] ^= 1;
        let connection = store.open_connection(false).unwrap();
        connection
            .execute(
                "UPDATE desktop_state_snapshots
                 SET protected_sync_state = ?1 WHERE id = ?2",
                params![corrupted_sync_state, newest.id],
            )
            .unwrap();
        drop(connection);

        let recovered = store.latest_valid("guest", 3_000).unwrap().unwrap();
        assert_eq!(backup.id, recovered.id);
        assert_eq!(backup_json, recovered.app_data_json);
        assert_eq!(backup_sync_state.as_slice(), recovered.protected_sync_state);
        assert_eq!(1, quarantine_count(&store));
        assert_eq!(
            evidence_before_quarantine,
            store.journal_evidence().unwrap()
        );
        let connection = store.open_connection(false).unwrap();
        let quarantined: (Vec<u8>, String, String, String) = connection
            .query_row(
                "SELECT protected_sync_state, sync_state_sha256,
                        observed_sync_state_sha256, reason
                 FROM desktop_state_snapshot_quarantine
                 WHERE snapshot_id = ?1",
                params![newest.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(corrupted_sync_state, quarantined.0);
        assert_eq!(newest.sync_state_sha256, quarantined.1);
        assert_eq!(sha256_hex(&quarantined.0), quarantined.2);
        assert!(quarantined
            .3
            .contains("protected sync state digest mismatch"));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn record_with_sync_state_atomically_returns_the_exact_app_and_metadata_pair() {
        let (directory, store) = temp_store("atomic_app_and_sync_state");
        let state = state_with_sessions(5, 100);
        let protected_sync_state = vec![0_u8, 1, 2, 3, 0xff, 0, 4, 5];

        let recorded = store
            .record_with_sync_state(
                "guest",
                &state,
                &protected_sync_state,
                1_000,
                "verified_restore",
            )
            .unwrap();

        assert_eq!(state, recorded.app_data_json);
        assert_eq!(protected_sync_state, recorded.protected_sync_state);
        assert_eq!(
            sha256_hex(&recorded.protected_sync_state),
            recorded.sync_state_sha256
        );
        let recovered = store.latest_valid("guest", 2_000).unwrap().unwrap();
        assert_eq!(recorded, recovered);
        let connection = store.open_connection(false).unwrap();
        let matching_rows: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM desktop_state_snapshots
                 WHERE id = ?1 AND app_data_json = ?2 AND protected_sync_state = ?3",
                params![recorded.id, state, protected_sync_state],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(1, matching_rows);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn valid_json_digit_corruption_is_quarantined_and_previous_exact_snapshot_recovers() {
        let (directory, store) = temp_store("digit_corruption");
        let first = state_with_sessions(12, 100);
        let second = state_with_sessions(13, 200);
        store.record("guest", &first, 1_000, "local_save").unwrap();
        let newest = store.record("guest", &second, 2_000, "local_save").unwrap();

        let connection = store.open_connection(false).unwrap();
        let changed = second.replacen("\"durationMillis\":1", "\"durationMillis\":9", 1);
        assert_ne!(second, changed);
        connection
            .execute(
                "UPDATE desktop_state_snapshots SET app_data_json = ?1 WHERE id = ?2",
                params![changed, newest.id],
            )
            .unwrap();
        drop(connection);

        let recovered = store.latest_valid("guest", 3_000).unwrap().unwrap();
        assert_eq!(first, recovered.app_data_json);
        assert_eq!(1, quarantine_count(&store));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn pure_analysis_preserves_legacy_and_extreme_clocks_and_rejects_warm_cache_token_corruption() {
        let cases = [
            (
                json!({
                    "schemaVersion": 1,
                    "slots": [{"id": 1}],
                    "notes": [{
                        "id": "legacy-clock-note", "content": "retained legacy text",
                        "revisions": [{"id": "legacy-clock-revision", "content": "older text"}]
                    }]
                }),
                AppDataJsonCompatibility::LegacyMigratable,
            ),
            (
                json!({
                    "schemaVersion": app_data::APP_DATA_SCHEMA_VERSION,
                    "slots": [{
                        "id": 1, "updatedAt": i64::MAX,
                        "runningSinceEpochMillis": i64::MAX
                    }],
                    "notes": [{
                        "id": "extreme-clock-note", "content": "retained winning text",
                        "createdAtEpochMillis": i64::MIN, "updatedAtEpochMillis": i64::MAX,
                        "versions": [{
                            "id": "extreme-clock-version", "sequence": i64::MAX,
                            "createdAtEpochMillis": i64::MAX, "updatedAtEpochMillis": i64::MAX,
                            "content": "retained winning text"
                        }]
                    }, {
                        "id": "extreme-clock-note", "content": "retained losing text",
                        "createdAtEpochMillis": 0, "updatedAtEpochMillis": i64::MIN
                    }],
                    "tombstones": [{
                        "entityType": "note", "entityId": "deleted-clock-note",
                        "deletedAtEpochMillis": i64::MAX
                    }]
                }),
                AppDataJsonCompatibility::CurrentKnown,
            ),
        ];
        for (input, expected_compatibility) in cases {
            let raw = input.to_string();
            // Exercise the document-returning entry point before metadata has
            // been cached, then again after the pure analysis cache is warm.
            DesktopPrivacyPolicy::default()
                .including_snapshot(&raw)
                .unwrap();
            let expected = analyze_app_data_json_uncached(&raw, 0).unwrap();
            let normalized = app_data::sanitize_app_data_json(&raw, 0).unwrap();
            for now in [i64::MIN, 0, 1, i64::MAX] {
                // Call the uncached classifier directly: a cache hit must not
                // conceal a future time-dependent normalization change.
                assert_eq!(
                    app_data::app_data_json_compatibility_from_value(&raw, &input, now),
                    expected_compatibility,
                );
                assert_eq!(
                    app_data::sanitize_app_data_json(&raw, now).unwrap(),
                    normalized,
                    "normalization must retain the same records and stored clocks at {now}",
                );
                for observed in [
                    analyze_app_data_json_uncached(&raw, now).unwrap(),
                    analyze_app_data_json(&raw, now).unwrap(),
                ] {
                    assert_eq!(observed.schema_version, expected.schema_version);
                    assert_eq!(observed.revision, expected.revision);
                    assert_eq!(observed.item_count, expected.item_count);
                    assert_eq!(observed.tombstone_count, expected.tombstone_count);
                    assert_eq!(observed.semantic_summary, expected.semantic_summary);
                    assert_eq!(
                        observed.canonical_json_sha256,
                        expected.canonical_json_sha256
                    );
                }
            }
            DesktopPrivacyPolicy::default()
                .including_snapshot(&raw)
                .unwrap();

            // Value collapses this repeated known field to the original view.
            // A previously accepted document must never authorize different raw
            // tokens, even when their generic JSON value would be identical.
            let duplicate = raw.replacen(
                '{',
                &format!("{{\"schemaVersion\":{},", input["schemaVersion"]),
                1,
            );
            assert_eq!(serde_json::from_str::<Value>(&duplicate).unwrap(), input);
            for now in [0, i64::MAX] {
                assert!(analyze_app_data_json(&duplicate, now).is_err());
            }
            assert!(DesktopPrivacyPolicy::default()
                .including_snapshot(&duplicate)
                .is_err());
        }
    }

    #[test]
    fn retention_byte_count_preserves_text_encoding_unicode_nul_and_overflow_pages() {
        for encoding in ["UTF-8", "UTF-16le", "UTF-16be"] {
            let connection = Connection::open_in_memory().unwrap();
            connection
                .execute_batch(&format!(
                    "PRAGMA encoding = '{encoding}';
                     CREATE TABLE retention_text_samples (
                         app_data_json TEXT NOT NULL
                     ) STRICT;"
                ))
                .unwrap();
            let stored_encoding: String = connection
                .query_row("PRAGMA encoding", [], |row| row.get(0))
                .unwrap();
            assert_eq!(stored_encoding, encoding);
            for text in [
                String::new(),
                "\0".to_owned(),
                "字🙂\0尾".to_owned(),
                "é字🙂\0".repeat(4_096),
            ] {
                connection
                    .execute(
                        "INSERT INTO retention_text_samples (app_data_json) VALUES (?1)",
                        params![text],
                    )
                    .unwrap();
                let (previous_bytes, retained_bytes): (i64, i64) = connection
                    .query_row(
                        "SELECT length(CAST(app_data_json AS BLOB)), octet_length(app_data_json)
                         FROM retention_text_samples WHERE rowid = last_insert_rowid()",
                        [],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .unwrap();
                let expected_bytes = if encoding == "UTF-8" {
                    text.len()
                } else {
                    text.encode_utf16().count() * 2
                } as i64;
                assert_eq!(previous_bytes, expected_bytes, "legacy count in {encoding}");
                assert_eq!(
                    retained_bytes, expected_bytes,
                    "retention count in {encoding}"
                );
            }
        }
    }

    #[test]
    fn mostly_empty_local_snapshot_is_rejected_without_replacing_the_last_good_snapshot() {
        let (directory, store) = temp_store("item_drop");
        let populated = state_with_sessions(20, 100);
        let empty = app_data::default_app_data_json(1_000);
        let recorded = store
            .record("guest", &populated, 1_000, "local_save")
            .unwrap();
        assert!(matches!(
            store.record("guest", &empty, 2_000, "local_save"),
            Err(DesktopStateStoreError::SuspiciousItemDrop { .. })
        ));
        assert_eq!(
            recorded.envelope_sha256,
            store
                .latest_valid("guest", 3_000)
                .unwrap()
                .unwrap()
                .envelope_sha256
        );
        assert!(!store.validate_exact("guest", &empty, 3_000).unwrap());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn semantic_summary_counts_active_deleted_versions_and_their_attachments() {
        let state = state_with_note_versions(3, 2, 2, 100);
        let analysis = analyze_app_data_json(&state, 0).unwrap();
        let summary: Value = serde_json::from_str(&analysis.semantic_summary).unwrap();

        assert_eq!(Some(3), summary["activeNoteVersions"].as_i64());
        assert_eq!(Some(2), summary["deletedNoteVersions"].as_i64());
        assert_eq!(Some(6), summary["activeNoteVersionAttachments"].as_i64());
        assert_eq!(Some(4), summary["deletedNoteVersionAttachments"].as_i64());
    }

    #[test]
    fn legacy_summary_projection_without_version_counts_remains_recoverable() {
        let state = state_with_note_versions(3, 2, 2, 100);
        let analysis = analyze_app_data_json(&state, 0).unwrap();
        let mut legacy: Value = serde_json::from_str(&analysis.semantic_summary).unwrap();
        let legacy_summary = legacy.as_object_mut().unwrap();
        let removed = [
            "activeNoteVersions",
            "deletedNoteVersions",
            "activeNoteVersionAttachments",
            "deletedNoteVersionAttachments",
        ]
        .into_iter()
        .map(|key| legacy_summary.remove(key).unwrap().as_i64().unwrap())
        .sum::<i64>();
        let legacy_item_count = analysis.item_count - removed;
        legacy_summary.insert(
            "recoverableItems".to_string(),
            Value::Number(legacy_item_count.into()),
        );
        let legacy_json = canonical_json_string(&legacy).unwrap();

        assert!(semantic_metadata_matches(
            legacy_item_count,
            &legacy_json,
            &analysis
        ));
        assert!(!semantic_metadata_matches(
            legacy_item_count + 1,
            &legacy_json,
            &analysis
        ));
    }

    #[test]
    fn local_version_stack_drop_from_one_hundred_to_one_is_rejected() {
        let (directory, store) = temp_store("version_stack_drop");
        let populated = state_with_note_versions(100, 0, 0, 100);
        let collapsed = state_with_note_versions(1, 0, 0, 1_000);
        store
            .record("guest", &populated, 1_000, "local_save")
            .unwrap();

        let error = store
            .record("guest", &collapsed, 2_000, "local_save")
            .unwrap_err();
        match error {
            DesktopStateStoreError::SuspiciousItemDrop {
                previous_items,
                incoming_items,
            } => {
                assert!(previous_items >= 100, "previous={previous_items}");
                assert!(incoming_items < previous_items / 2);
            }
            other => panic!("unexpected error: {other}"),
        }
        let recovered = store.latest_valid("guest", 3_000).unwrap().unwrap();
        assert_eq!(populated, recovered.app_data_json);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn owner_is_part_of_the_envelope_and_tampered_row_cannot_cross_accounts() {
        let (directory, store) = temp_store("wrong_owner");
        let state = state_with_sessions(2, 100);
        let other_state = state_with_sessions(1, 200);
        let recorded = store
            .record("namespace-a\0user-a", &state, 1_000, "local_save")
            .unwrap();
        let other_recorded = store
            .record("namespace-b\0user-b", &other_state, 1_500, "local_save")
            .unwrap();
        let connection = store.open_connection(false).unwrap();
        connection
            .execute(
                "UPDATE desktop_state_snapshots SET owner = ?1 WHERE id = ?2",
                params!["namespace-b\0user-b", recorded.id],
            )
            .unwrap();
        drop(connection);

        assert_eq!(
            other_recorded.envelope_sha256,
            store
                .latest_valid("namespace-b\0user-b", 2_000)
                .unwrap()
                .unwrap()
                .envelope_sha256
        );
        assert!(!store
            .validate_exact("namespace-b\0user-b", &state, 2_000)
            .unwrap());
        assert_eq!(1, quarantine_count(&store));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn owner_registry_survives_total_snapshot_quarantine_and_blocks_silent_rebootstrap() {
        let (directory, store) = temp_store("owner_registry");
        let state = state_with_sessions(2, 100);
        let recorded = store.record("guest", &state, 1_000, "local_save").unwrap();
        assert!(store.owner_initialized("guest").unwrap());

        let connection = store.open_connection(false).unwrap();
        connection
            .execute(
                "UPDATE desktop_state_snapshots SET raw_sha256 = ?1 WHERE id = ?2",
                params!["0".repeat(64), recorded.id],
            )
            .unwrap();
        drop(connection);

        assert!(store.latest_valid("guest", 2_000).unwrap().is_none());
        assert!(store.owner_initialized("guest").unwrap());
        assert!(matches!(
            store.record("guest", &state, 3_000, "local_save"),
            Err(DesktopStateStoreError::InitializedOwnerWithoutValidSnapshot)
        ));
        assert!(store
            .record("guest", &state, 4_000, "verified_restore")
            .is_ok());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn dedupe_only_applies_to_the_current_head_and_does_not_turn_a_b_a_into_b() {
        let (directory, store) = temp_store("head_dedupe");
        let state_a = state_with_sessions(2, 100);
        let state_b = state_with_sessions(3, 200);
        let first_a = store
            .record("guest", &state_a, 1_000, "local_save")
            .unwrap();
        let state_b_row = store
            .record("guest", &state_b, 2_000, "local_save")
            .unwrap();
        let second_a = store
            .record("guest", &state_a, 3_000, "local_save")
            .unwrap();
        let duplicate_a = store
            .record("guest", &state_a, 4_000, "local_save")
            .unwrap();

        assert!(state_b_row.id > first_a.id);
        assert!(second_a.id > state_b_row.id);
        assert_eq!(second_a.id, duplicate_a.id);
        assert_eq!(
            second_a.id,
            store.latest_valid("guest", 5_000).unwrap().unwrap().id
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn scope_media_transaction_source_forces_and_proves_one_exact_snapshot() {
        let (directory, store) = temp_store("scope_media_transaction");
        let state = state_with_sessions(2, 100);
        let ordinary = store.record("guest", &state, 1_000, "local_save").unwrap();
        let source = format!("scope_media_tx:{}", "a".repeat(64));
        let transaction = store.record("guest", &state, 2_000, &source).unwrap();

        assert!(transaction.id > ordinary.id);
        let proof = store
            .valid_scope_media_transaction_snapshot(
                "guest",
                &sha256_hex(state.as_bytes()),
                &source,
                3_000,
            )
            .unwrap()
            .unwrap();
        assert_eq!(transaction.id, proof.id);

        let retry = store.record("guest", &state, 2_000, &source).unwrap();
        assert_eq!(transaction.id, retry.id);
        let changed_state = state_with_sessions(3, 300);
        assert!(store
            .record("guest", &changed_state, 3_000, &source)
            .is_err());
        assert!(store
            .record_with_sync_state("guest", &state, b"different-sync", 3_000, &source)
            .is_err());
        assert!(store.record("other-owner", &state, 3_000, &source).is_err());
        assert!(store
            .valid_scope_media_transaction_snapshot("guest", &"b".repeat(64), &source, 3_000,)
            .is_err());
        assert!(store
            .record("guest", &state, 4_000, "scope_media_tx:bad")
            .is_err());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn existing_v2_scope_snapshot_is_backfilled_into_the_additive_proof_table() {
        let (directory, store) = temp_store("scope_media_v2_backfill");
        let database_path = store.database_path().to_path_buf();
        let state = state_with_sessions(2, 100);
        let source = format!("scope_media_tx:{}", "c".repeat(64));
        let recorded = store.record("guest", &state, 1_000, &source).unwrap();
        let connection = store.open_connection(false).unwrap();
        connection
            .execute_batch("DROP TABLE desktop_state_scope_media_transaction_proofs")
            .unwrap();
        drop(connection);
        drop(store);

        let reopened = DesktopStateStore::open(&database_path).unwrap();
        let proof = reopened
            .valid_scope_media_transaction_snapshot(
                "guest",
                &sha256_hex(state.as_bytes()),
                &source,
                2_000,
            )
            .unwrap()
            .unwrap();
        assert_eq!(recorded.id, proof.id);
        let connection = reopened.open_connection(false).unwrap();
        assert_eq!(
            1,
            table_row_count(&connection, SCOPE_MEDIA_TRANSACTION_PROOF_TABLE).unwrap()
        );
        verify_required_schema(&connection).unwrap();
        verify_foreign_keys(&connection).unwrap();
        drop(connection);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn scope_media_proof_survives_gc_then_completion_leaves_a_permanent_tombstone() {
        let (directory, store) = temp_store("scope_media_proof_gc");
        let source = format!("scope_media_tx:{}", "d".repeat(64));
        let state = state_with_sessions(2, 100);
        let large_sync_state = vec![0x5a; 18 * 1024 * 1024];
        let recorded = store
            .record_with_sync_state("guest", &state, &large_sync_state, 1_000, &source)
            .unwrap();

        let mut later_ids = Vec::new();
        for index in 0..5 {
            let newer_state = state_with_sessions(index + 3, 200 + index as i64 * 100);
            let newer = store
                .record_with_sync_state(
                    "guest",
                    &newer_state,
                    &large_sync_state,
                    2_000 + index as i64,
                    "local_save",
                )
                .unwrap();
            later_ids.push(newer.id);
        }

        let mut connection = store.open_connection(false).unwrap();
        assert_eq!(4, table_row_count(&connection, SNAPSHOT_TABLE).unwrap());
        assert_eq!(
            1,
            table_row_count(&connection, SCOPE_MEDIA_TRANSACTION_PROOF_TABLE).unwrap()
        );
        let pending = read_scope_media_transaction_proof(&connection, &source)
            .unwrap()
            .unwrap();
        assert_eq!(recorded.id, pending.snapshot_id);
        assert_eq!(Some(recorded.id), pending.pinned_snapshot_id);
        assert!(read_snapshot_by_id(&connection, recorded.id)
            .unwrap()
            .is_some());
        for (index, id) in later_ids.iter().enumerate() {
            assert_eq!(
                read_snapshot_by_id(&connection, *id).unwrap().is_some(),
                index >= later_ids.len() - MIN_RETAINED_SNAPSHOTS_PER_OWNER,
                "GC must retain the newest minimum even while the older media snapshot is pinned",
            );
        }
        // If a transaction releases the old pin and prunes it, rolling that
        // transaction back must restore both the row and its retention proof.
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert_eq!(
            transaction
                .execute(
                    "UPDATE desktop_state_scope_media_transaction_proofs
                     SET pinned_snapshot_id = NULL WHERE source = ?1",
                    params![source],
                )
                .unwrap(),
            1,
        );
        assert_eq!(
            prune_owner_history(&transaction, "guest", 2_500).unwrap(),
            1
        );
        assert!(read_snapshot_by_id(&transaction, recorded.id)
            .unwrap()
            .is_none());
        transaction.rollback().unwrap();
        assert_eq!(4, table_row_count(&connection, SNAPSHOT_TABLE).unwrap());
        assert!(read_snapshot_by_id(&connection, recorded.id)
            .unwrap()
            .is_some());
        assert_eq!(
            read_scope_media_transaction_proof(&connection, &source)
                .unwrap()
                .unwrap()
                .pinned_snapshot_id,
            Some(recorded.id),
        );
        drop(connection);
        assert_eq!(
            recorded.id,
            store
                .valid_scope_media_transaction_snapshot(
                    "guest",
                    &sha256_hex(state.as_bytes()),
                    &source,
                    3_000,
                )
                .unwrap()
                .unwrap()
                .id
        );
        assert!(store
            .record_with_sync_state("guest", &state, b"rebound-sync", 3_000, &source)
            .is_err());

        store.complete_scope_media_transaction(&source).unwrap();
        store.complete_scope_media_transaction(&source).unwrap();
        let final_state = state_with_sessions(9, 900);
        store
            .record_with_sync_state(
                "guest",
                &final_state,
                &large_sync_state,
                4_000,
                "local_save",
            )
            .unwrap();

        let connection = store.open_connection(false).unwrap();
        let tombstone = read_scope_media_transaction_proof(&connection, &source)
            .unwrap()
            .unwrap();
        assert_eq!(recorded.id, tombstone.snapshot_id);
        assert_eq!(None, tombstone.pinned_snapshot_id);
        assert!(read_snapshot_by_id(&connection, recorded.id)
            .unwrap()
            .is_none());
        assert_eq!(
            1,
            table_row_count(&connection, SCOPE_MEDIA_TRANSACTION_PROOF_TABLE).unwrap()
        );
        drop(connection);
        assert!(store
            .valid_scope_media_transaction_snapshot(
                "guest",
                &sha256_hex(state.as_bytes()),
                &source,
                5_000,
            )
            .unwrap()
            .is_none());
        assert!(store.record("guest", &state, 5_000, &source).is_err());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn bad_hash_row_is_skipped_without_hiding_an_older_good_snapshot() {
        let (directory, store) = temp_store("bad_hash");
        let first = state_with_sessions(2, 100);
        let second = state_with_sessions(3, 200);
        store.record("guest", &first, 1_000, "local_save").unwrap();
        let newest = store.record("guest", &second, 2_000, "local_save").unwrap();
        let connection = store.open_connection(false).unwrap();
        connection
            .execute(
                "UPDATE desktop_state_snapshots SET raw_sha256 = ?1 WHERE id = ?2",
                params!["0".repeat(64), newest.id],
            )
            .unwrap();
        drop(connection);

        assert_eq!(
            first,
            store
                .latest_valid("guest", 3_000)
                .unwrap()
                .unwrap()
                .app_data_json
        );
        assert_eq!(1, quarantine_count(&store));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn streamed_quarantine_keeps_adjacent_rows_and_owner_boundaries_correct() {
        let (directory, store) = temp_store("streamed_quarantine");
        let first_a = store
            .record("owner-a", &state_with_sessions(1, 100), 100, "local_save")
            .unwrap();
        let first_b = store
            .record("owner-b", &state_with_sessions(1, 110), 110, "local_save")
            .unwrap();
        let second_a = store
            .record("owner-a", &state_with_sessions(2, 200), 200, "local_save")
            .unwrap();
        let second_b = store
            .record("owner-b", &state_with_sessions(2, 210), 210, "local_save")
            .unwrap();
        let latest_a = store
            .record("owner-a", &state_with_sessions(3, 300), 300, "local_save")
            .unwrap();
        let evidence = store.journal_evidence().unwrap();
        let connection = store.open_connection(false).unwrap();
        // Include an invalid negative primary key: starting a streamed scan
        // at zero would silently leave this corrupt recovery row unexamined.
        connection
            .execute(
                "UPDATE desktop_state_snapshots SET id = ?1 WHERE id = ?2",
                params![i64::MIN, first_a.id],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE desktop_state_snapshots SET raw_sha256 = ?1 WHERE id IN (?2, ?3)",
                params!["0".repeat(64), second_a.id, second_b.id],
            )
            .unwrap();
        drop(connection);

        assert_eq!(
            latest_a,
            store.latest_valid("owner-a", 400).unwrap().unwrap()
        );
        assert_eq!(2, quarantine_count(&store));
        assert_eq!(evidence, store.journal_evidence().unwrap());
        let connection = store.open_connection(false).unwrap();
        assert!(read_snapshot_by_id(&connection, second_b.id)
            .unwrap()
            .is_some());
        drop(connection);

        // Startup scans every owner, still without retaining all payloads.
        let reopened = DesktopStateStore::open(store.database_path()).unwrap();
        assert_eq!(3, quarantine_count(&reopened));
        assert_eq!(
            first_b,
            reopened.latest_valid("owner-b", 500).unwrap().unwrap()
        );
        assert_eq!(
            latest_a,
            reopened.latest_valid("owner-a", 500).unwrap().unwrap()
        );
        assert_eq!(evidence, reopened.journal_evidence().unwrap());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn torn_newest_envelope_falls_back_to_previous_journal_backup() {
        let (directory, store) = temp_store("torn_write");
        let backup = state_with_sessions(4, 100);
        let newest_json = state_with_sessions(5, 200);
        store.record("guest", &backup, 1_000, "local_save").unwrap();
        let newest = store
            .record("guest", &newest_json, 2_000, "local_save")
            .unwrap();
        let connection = store.open_connection(false).unwrap();
        connection
            .execute(
                "UPDATE desktop_state_snapshots
                 SET parent_envelope_sha256 = ?1 WHERE id = ?2",
                params!["1".repeat(64), newest.id],
            )
            .unwrap();
        drop(connection);

        assert!(!store.validate_exact("guest", &newest_json, 3_000).unwrap());
        let recovered = store.latest_valid("guest", 3_000).unwrap().unwrap();
        assert_eq!(backup, recovered.app_data_json);
        assert!(store.validate_exact("guest", &backup, 3_000).unwrap());
        let _ = fs::remove_dir_all(directory);
    }
}
