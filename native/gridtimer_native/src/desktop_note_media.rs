// v0.0.1 - Recheck snapshot references before retrying durable cleanup markers.
// v2.22.21 - Recover interrupted downloads and durably collect unreferenced committed media.
//! Durable, account-scoped note-image storage for desktop clients.
//!
//! The caller supplies the exact account workspace root. This module never
//! derives a location from a profile directory or environment variable. New
//! imports use a durable journal and a same-directory temporary blob so the
//! application can persist its AppData reference between [`begin_import`] and
//! [`commit_import`]. Call [`recover_pending`] when opening a workspace.

#![cfg(not(target_os = "android"))]

use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fmt::{self, Write as _};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
#[cfg(test)]
use std::path::Component;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_IMAGE_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_IMAGE_DIMENSION: u32 = 16_384;
pub const MAX_IMAGE_PIXELS: u64 = 64 * 1024 * 1024;
const MAX_IMAGE_HEADER_BYTES: usize = 1024 * 1024;

const PENDING_DIRECTORY: &str = ".pending_imports_v1";
const BLOB_SUFFIX: &str = ".blob";
const METADATA_SUFFIX: &str = ".media.json";
const PENDING_BLOB_PREFIX: &str = ".pending_";
const PENDING_BLOB_SUFFIX: &str = ".blob.tmp";
const JOURNAL_SUFFIX: &str = ".json";
const JOURNAL_TEMP_SUFFIX: &str = ".json.tmp";
const CLEANUP_BLOCK_PREFIX: &str = ".cleanup_block_";
const CLEANUP_BLOCK_SUFFIX: &str = ".json";
const CLEANUP_BLOCK_TEMP_SUFFIX: &str = ".json.tmp";
const CLEANUP_BLOCK_STATE_VERSION: u32 = 1;
const METADATA_TEMP_PREFIX: &str = ".pending_metadata_";
const METADATA_TEMP_SUFFIX: &str = ".json.tmp";
const IDENTITY_MARKER_FILE: &str = ".workspace_identity_v1.json";
const IDENTITY_MARKER_TEMP_FILE: &str = ".workspace_identity_v1.json.tmp";
const IDENTITY_MARKER_STATE_VERSION: u32 = 1;
const IDENTITY_FINGERPRINT_DOMAIN: &[u8] = b"gridtimer-desktop-note-media-workspace-v1\0";
const MAX_WORKSPACE_IDENTITY_BYTES: usize = 1024;
const JOURNAL_STATE_VERSION: u32 = 1;
const MAX_JOURNAL_BYTES: u64 = 1024 * 1024;
const COPY_BUFFER_BYTES: usize = 64 * 1024;

pub type MediaResult<T> = Result<T, DesktopNoteMediaError>;

#[derive(Debug)]
pub enum DesktopNoteMediaError {
    RootMustBeAbsolute(PathBuf),
    RootIsNotDirectory(PathBuf),
    UnsafeStorageLayout(PathBuf),
    InvalidWorkspaceIdentity,
    WorkspaceIdentityMismatch,
    WorkspaceIdentityMarkerCorrupt,
    InvalidAttachmentId(String),
    InvalidSha256(String),
    InvalidExpectedSize(i64),
    EmptyImage,
    ImageTooLarge {
        actual: u64,
        maximum: u64,
    },
    UnsupportedImageFormat,
    InvalidImage(String),
    ImageDimensionsTooLarge {
        width: u32,
        height: u32,
        maximum_dimension: u32,
        maximum_pixels: u64,
    },
    MimeTypeMismatch {
        expected: String,
        actual: String,
    },
    SizeMismatch {
        expected: u64,
        actual: u64,
    },
    Sha256Mismatch {
        expected: String,
        actual: String,
    },
    PendingImportBelongsToAnotherStore,
    PendingImportMissing(String),
    PendingImportCorrupt(String),
    ExistingBlobConflict(String),
    LockPoisoned,
    Io(io::Error),
    Json(serde_json::Error),
}

impl fmt::Display for DesktopNoteMediaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RootMustBeAbsolute(path) => {
                write!(formatter, "note-media root must be absolute: {}", path.display())
            }
            Self::RootIsNotDirectory(path) => {
                write!(formatter, "note-media root is not a directory: {}", path.display())
            }
            Self::UnsafeStorageLayout(path) => {
                write!(formatter, "note-media storage escaped its root: {}", path.display())
            }
            Self::InvalidWorkspaceIdentity => formatter.write_str(
                "workspace identity must be a stable non-token account namespace",
            ),
            Self::WorkspaceIdentityMismatch => formatter.write_str(
                "note-media root is already bound to another account namespace",
            ),
            Self::WorkspaceIdentityMarkerCorrupt => {
                formatter.write_str("note-media workspace identity marker is corrupt")
            }
            Self::InvalidAttachmentId(id) => write!(formatter, "invalid attachment id: {id}"),
            Self::InvalidSha256(value) => write!(formatter, "invalid SHA-256 digest: {value}"),
            Self::InvalidExpectedSize(value) => {
                write!(formatter, "invalid expected note-media size: {value}")
            }
            Self::EmptyImage => formatter.write_str("the selected image is empty"),
            Self::ImageTooLarge { actual, maximum } => write!(
                formatter,
                "the selected image is {actual} bytes; the limit is {maximum} bytes"
            ),
            Self::UnsupportedImageFormat => formatter.write_str(
                "only PNG, JPEG, WebP, GIF, and BMP images can be stored as note attachments",
            ),
            Self::InvalidImage(message) => write!(formatter, "invalid image: {message}"),
            Self::ImageDimensionsTooLarge {
                width,
                height,
                maximum_dimension,
                maximum_pixels,
            } => write!(
                formatter,
                "image dimensions {width}x{height} exceed {maximum_dimension} per side or {maximum_pixels} total pixels"
            ),
            Self::MimeTypeMismatch { expected, actual } => {
                write!(formatter, "image MIME mismatch: expected {expected}, found {actual}")
            }
            Self::SizeMismatch { expected, actual } => {
                write!(formatter, "media size mismatch: expected {expected}, found {actual}")
            }
            Self::Sha256Mismatch { expected, actual } => {
                write!(formatter, "media digest mismatch: expected {expected}, found {actual}")
            }
            Self::PendingImportBelongsToAnotherStore => {
                formatter.write_str("pending note-media import belongs to another workspace")
            }
            Self::PendingImportMissing(id) => {
                write!(formatter, "pending note-media import is missing: {id}")
            }
            Self::PendingImportCorrupt(id) => {
                write!(formatter, "pending note-media import is corrupt: {id}")
            }
            Self::ExistingBlobConflict(id) => {
                write!(formatter, "attachment id already stores different bytes: {id}")
            }
            Self::LockPoisoned => formatter.write_str("note-media workspace lock is poisoned"),
            Self::Io(error) => write!(formatter, "note-media I/O failed: {error}"),
            Self::Json(error) => write!(formatter, "note-media JSON failed: {error}"),
        }
    }
}

impl std::error::Error for DesktopNoteMediaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for DesktopNoteMediaError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for DesktopNoteMediaError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

/// AppData-compatible `NoteAttachment` JSON.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NoteAttachmentMetadata {
    pub id: String,
    pub kind: String,
    pub file_name: String,
    pub display_name: String,
    pub mime_type: String,
    pub width: i32,
    pub height: i32,
    pub size_bytes: i64,
    pub sha256: String,
    pub created_at_epoch_millis: i64,
    pub updated_at_epoch_millis: i64,
}

impl NoteAttachmentMetadata {
    pub fn to_app_data_json(&self) -> MediaResult<String> {
        Ok(serde_json::to_string(self)?)
    }

    fn validate(&self) -> MediaResult<()> {
        validate_attachment_id(&self.id)?;
        if !matches!(self.kind.as_str(), "IMAGE" | "FILE" | "AUDIO" | "VIDEO") {
            return Err(DesktopNoteMediaError::UnsupportedImageFormat);
        }
        let expected_file_name = blob_file_name(&self.id)?;
        if self.file_name != expected_file_name {
            return Err(DesktopNoteMediaError::PendingImportCorrupt(self.id.clone()));
        }
        let expected_size = validate_expected_size(self.size_bytes)?;
        validate_sha256(&self.sha256)?;
        if self.kind != "IMAGE" {
            if self.width != 0
                || self.height != 0
                || general_media_kind(&self.mime_type) != Some(self.kind.as_str())
            {
                return Err(DesktopNoteMediaError::UnsupportedImageFormat);
            }
            return Ok(());
        }
        if self.width <= 0 || self.height <= 0 {
            return Err(DesktopNoteMediaError::InvalidImage(
                "image dimensions are missing".to_string(),
            ));
        }
        validate_dimensions(self.width as u32, self.height as u32)?;
        if expected_size > MAX_IMAGE_BYTES {
            return Err(DesktopNoteMediaError::ImageTooLarge {
                actual: expected_size,
                maximum: MAX_IMAGE_BYTES,
            });
        }
        canonical_mime(&self.mime_type).ok_or(DesktopNoteMediaError::UnsupportedImageFormat)?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct PendingMediaImport {
    store_root: PathBuf,
    attachment: NoteAttachmentMetadata,
}

impl PendingMediaImport {
    pub fn attachment(&self) -> &NoteAttachmentMetadata {
        &self.attachment
    }

    pub fn attachment_id(&self) -> &str {
        &self.attachment.id
    }

    pub fn attachment_json(&self) -> MediaResult<String> {
        self.attachment.to_app_data_json()
    }
}

/// A sync-manifest row backed by bytes that were revalidated during the scan.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteMediaManifestEntry {
    pub attachment_id: String,
    pub sha256: String,
    pub mime_type: String,
    pub size_bytes: i64,
    pub updated_at_epoch_millis: i64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PendingRecoveryReport {
    pub recovered_count: usize,
    pub already_committed_count: usize,
    pub malformed_journal_count: usize,
    pub missing_blob_count: usize,
    pub corrupt_blob_count: usize,
    pub unreferenced_pending_count: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CleanupRecoveryReport {
    pub cleaned_count: usize,
    pub remaining_count: usize,
    pub malformed_marker_count: usize,
}

impl CleanupRecoveryReport {
    pub fn is_clear(&self) -> bool {
        self.remaining_count == 0 && self.malformed_marker_count == 0
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MediaEncryptionBlockerReport {
    pub cleanup_marker_count: usize,
    pub pending_artifact_count: usize,
    pub orphan_attachment_count: usize,
}

impl MediaEncryptionBlockerReport {
    pub fn is_clear(&self) -> bool {
        self.cleanup_marker_count == 0
            && self.pending_artifact_count == 0
            && self.orphan_attachment_count == 0
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PendingJournal {
    state_version: u32,
    attachment: NoteAttachmentMetadata,
    journaled_at_epoch_millis: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CleanupBlockMarker {
    state_version: u32,
    attachment_id: String,
    reason: String,
    created_at_epoch_millis: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkspaceIdentityMarker {
    state_version: u32,
    workspace_identity_sha256: String,
}

#[derive(Clone, Debug)]
struct InspectedImage {
    mime_type: &'static str,
    width: u32,
    height: u32,
}

type WorkspaceLockRegistry = HashMap<PathBuf, Weak<Mutex<()>>>;

static WORKSPACE_LOCKS: OnceLock<Mutex<WorkspaceLockRegistry>> = OnceLock::new();

/// Storage rooted at one explicit account workspace.
#[derive(Clone, Debug)]
pub struct DesktopNoteMediaStore {
    root: PathBuf,
    pending_directory: PathBuf,
    operation_lock: Arc<Mutex<()>>,
}

/// Result of probing a bound workspace without creating or repairing any
/// on-disk state.
#[derive(Clone, Debug)]
pub enum ExistingBoundNoteMediaProbe {
    Missing,
    Present(ReadOnlyDesktopNoteMediaStore),
}

/// A verified, existing workspace view that exposes only read operations.
/// Construction never creates the media root, pending directory, or identity
/// marker and never performs cleanup or crash recovery.
#[derive(Clone, Debug)]
pub struct ReadOnlyDesktopNoteMediaStore {
    inner: DesktopNoteMediaStore,
}

impl ReadOnlyDesktopNoteMediaStore {
    pub fn probe_existing_bound(
        workspace_media_root: impl AsRef<Path>,
        workspace_identity: impl AsRef<str>,
    ) -> MediaResult<ExistingBoundNoteMediaProbe> {
        let requested_root = workspace_media_root.as_ref();
        if !requested_root.is_absolute() {
            return Err(DesktopNoteMediaError::RootMustBeAbsolute(
                requested_root.to_path_buf(),
            ));
        }
        let metadata = match fs::symlink_metadata(requested_root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(ExistingBoundNoteMediaProbe::Missing);
            }
            Err(error) => return Err(error.into()),
        };
        if !metadata.is_dir() || metadata_is_reparse_point(&metadata) {
            return Err(DesktopNoteMediaError::UnsafeStorageLayout(
                requested_root.to_path_buf(),
            ));
        }
        let root = requested_root.canonicalize()?;
        if !root.is_dir() {
            return Err(DesktopNoteMediaError::RootIsNotDirectory(root));
        }
        let fingerprint = workspace_identity_fingerprint(workspace_identity.as_ref())?;
        let operation_lock = shared_workspace_lock(&root)?;
        let inner = DesktopNoteMediaStore {
            pending_directory: root.join(PENDING_DIRECTORY),
            root,
            operation_lock,
        };
        {
            let _guard = inner.lock()?;
            verify_existing_workspace_identity_marker(&inner.root, &fingerprint)?;
        }
        Ok(ExistingBoundNoteMediaProbe::Present(Self { inner }))
    }

    pub fn root(&self) -> &Path {
        self.inner.root()
    }

    pub fn manifest_entry_for(
        &self,
        attachment_id: &str,
    ) -> MediaResult<Option<NoteMediaManifestEntry>> {
        self.inner.manifest_entry_for(attachment_id)
    }

    pub fn read_blob(
        &self,
        attachment_id: &str,
        expected_sha256: &str,
        expected_size_bytes: i64,
    ) -> MediaResult<Vec<u8>> {
        self.inner
            .read_blob(attachment_id, expected_sha256, expected_size_bytes)
    }

    /// Supplies a path only after validating the scoped store and the exact
    /// metadata-bound blob. The legal scanner rehashes it immediately on read.
    pub fn verified_blob_path(
        &self,
        attachment_id: &str,
        expected_sha256: &str,
        expected_size_bytes: i64,
    ) -> MediaResult<PathBuf> {
        let _ = self.read_blob(attachment_id, expected_sha256, expected_size_bytes)?;
        self.inner.blob_path(attachment_id)
    }
}

impl DesktopNoteMediaStore {
    /// Opens an unbound store for tests and legacy callers. Production account
    /// workspaces should use [`Self::new_bound`] so an accidentally reused
    /// directory is rejected before any attachment operation runs.
    pub fn new(workspace_media_root: impl AsRef<Path>) -> MediaResult<Self> {
        let requested_root = workspace_media_root.as_ref();
        if !requested_root.is_absolute() {
            return Err(DesktopNoteMediaError::RootMustBeAbsolute(
                requested_root.to_path_buf(),
            ));
        }
        fs::create_dir_all(requested_root)?;
        let root = requested_root.canonicalize()?;
        if !root.is_dir() {
            return Err(DesktopNoteMediaError::RootIsNotDirectory(root));
        }

        let operation_lock = shared_workspace_lock(&root)?;
        let pending_directory;
        {
            let _guard = operation_lock
                .lock()
                .map_err(|_| DesktopNoteMediaError::LockPoisoned)?;
            let requested_pending_directory = root.join(PENDING_DIRECTORY);
            fs::create_dir_all(&requested_pending_directory)?;
            pending_directory = requested_pending_directory.canonicalize()?;
            if !pending_directory.is_dir() || pending_directory.parent() != Some(root.as_path()) {
                return Err(DesktopNoteMediaError::UnsafeStorageLayout(
                    pending_directory,
                ));
            }
        }

        Ok(Self {
            root,
            pending_directory,
            operation_lock,
        })
    }

    /// Opens a production store bound to a stable account/workspace identity.
    /// The marker stores only a domain-separated SHA-256 fingerprint, never the
    /// supplied identity or an authentication token. Pass an account namespace
    /// (or an equivalently stable, non-secret workspace id), not a login token.
    pub fn new_bound(
        workspace_media_root: impl AsRef<Path>,
        workspace_identity: impl AsRef<str>,
    ) -> MediaResult<Self> {
        let fingerprint = workspace_identity_fingerprint(workspace_identity.as_ref())?;
        let store = Self::new(workspace_media_root)?;
        {
            let _guard = store.lock()?;
            store.ensure_workspace_identity_marker(&fingerprint)?;
        }
        Ok(store)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Stages and journals an image. No final blob name is visible until
    /// [`commit_import`](Self::commit_import) succeeds.
    pub fn begin_import(&self, source_path: impl AsRef<Path>) -> MediaResult<PendingMediaImport> {
        let display_name = source_path
            .as_ref()
            .file_name()
            .and_then(|name| name.to_str())
            .map(sanitize_display_name)
            .filter(|name| !name.is_empty());
        self.begin_import_internal(
            source_path.as_ref(),
            display_name.as_deref(),
            current_time_millis(),
            None,
        )
    }

    pub fn begin_import_with_display_name(
        &self,
        source_path: impl AsRef<Path>,
        display_name: &str,
    ) -> MediaResult<PendingMediaImport> {
        let display_name = sanitize_display_name(display_name);
        self.begin_import_internal(
            source_path.as_ref(),
            (!display_name.is_empty()).then_some(display_name.as_str()),
            current_time_millis(),
            None,
        )
    }

    /// Stages an ordinary file or playable media through the same durable journal.
    pub fn begin_file_import(
        &self,
        source_path: impl AsRef<Path>,
        kind: &str,
    ) -> MediaResult<PendingMediaImport> {
        let path = source_path.as_ref();
        let mime = if kind == "FILE" {
            match general_media_mime(path) {
                "application/pdf" => "application/pdf",
                "text/plain" => "text/plain",
                "text/csv" => "text/csv",
                "application/json" => "application/json",
                "application/zip" => "application/zip",
                _ => "application/octet-stream",
            }
        } else {
            general_media_mime(path)
        };
        if general_media_kind(mime) != Some(kind) {
            return Err(DesktopNoteMediaError::UnsupportedImageFormat);
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("attachment");
        self.stage_import(
            path,
            Some(name),
            current_time_millis(),
            None,
            Some((kind, mime)),
        )
    }

    /// Publishes a staged import and clears its journal only after the blob and
    /// metadata have both been flushed and verified.
    pub fn commit_import(
        &self,
        pending: &PendingMediaImport,
    ) -> MediaResult<NoteAttachmentMetadata> {
        let _guard = self.lock()?;
        if pending.store_root != self.root {
            return Err(DesktopNoteMediaError::PendingImportBelongsToAnotherStore);
        }
        pending.attachment.validate()?;
        let journal = self.read_expected_journal(&pending.attachment.id)?;
        if journal.attachment != pending.attachment {
            return Err(DesktopNoteMediaError::PendingImportCorrupt(
                pending.attachment.id.clone(),
            ));
        }
        self.finalize_journal(&journal)?;
        Ok(pending.attachment.clone())
    }

    /// Cancels a staged import without ever exposing it as a committed blob.
    /// A durable cleanup marker is written before deletion starts. Therefore a
    /// crash, sharing violation, or partial deletion remains visible to the
    /// encryption gate and can be retried on the next open.
    pub fn cancel_import(&self, pending: &PendingMediaImport) -> MediaResult<()> {
        let _guard = self.lock()?;
        if pending.store_root != self.root {
            return Err(DesktopNoteMediaError::PendingImportBelongsToAnotherStore);
        }
        pending.attachment.validate()?;
        let journal = self.read_expected_journal(&pending.attachment.id)?;
        if journal.attachment != pending.attachment {
            return Err(DesktopNoteMediaError::PendingImportCorrupt(
                pending.attachment.id.clone(),
            ));
        }
        self.guarded_cleanup_attachment(&pending.attachment.id, "cancel-import")
            .map(|_| ())
    }

    /// Recovers every well-formed journal independently. Malformed or corrupt
    /// records are retained as evidence and counted instead of blocking other
    /// attachments.
    pub fn recover_pending(&self) -> MediaResult<PendingRecoveryReport> {
        self.recover_pending_internal(None)
    }

    /// Recovers only imports whose AppData references are already durable.
    /// Unreferenced journals are deliberately retained: they may represent a
    /// crash between `begin_import` and the AppData commit and must not be
    /// promoted into plaintext orphan blobs.
    pub fn recover_pending_referenced(
        &self,
        referenced_attachment_ids: &HashSet<String>,
    ) -> MediaResult<PendingRecoveryReport> {
        self.recover_pending_internal(Some(referenced_attachment_ids))
    }

    fn recover_pending_internal(
        &self,
        referenced_attachment_ids: Option<&HashSet<String>>,
    ) -> MediaResult<PendingRecoveryReport> {
        let _guard = self.lock()?;
        let mut report = PendingRecoveryReport::default();
        let mut journal_paths = fs::read_dir(&self.pending_directory)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension().and_then(|value| value.to_str()) == Some("json")
                    && !path
                        .file_name()
                        .and_then(|value| value.to_str())
                        .is_some_and(|name| name.starts_with(CLEANUP_BLOCK_PREFIX))
            })
            .collect::<Vec<_>>();
        journal_paths.sort();

        for journal_path in journal_paths {
            let journal = match self.read_journal_path(&journal_path) {
                Ok(journal) => journal,
                Err(_) => {
                    report.malformed_journal_count += 1;
                    continue;
                }
            };
            if referenced_attachment_ids
                .is_some_and(|referenced| !referenced.contains(&journal.attachment.id))
            {
                report.unreferenced_pending_count += 1;
                continue;
            }
            let final_path = self.blob_path_unchecked(&journal.attachment.id);
            let staged_path = self.pending_blob_path_unchecked(&journal.attachment.id);
            let final_exists = safe_regular_child_exists(&self.root, &final_path)?;
            let staged_exists = safe_regular_child_exists(&self.root, &staged_path)?;

            if final_exists {
                if self.verify_blob(&final_path, &journal.attachment).is_err() {
                    report.corrupt_blob_count += 1;
                    continue;
                }
                self.ensure_metadata_file(&journal.attachment)?;
                if staged_exists {
                    fs::remove_file(&staged_path)?;
                    sync_directory(&self.root)?;
                }
                self.remove_journal(&journal.attachment.id)?;
                report.already_committed_count += 1;
            } else if staged_exists {
                if self.verify_blob(&staged_path, &journal.attachment).is_err() {
                    report.corrupt_blob_count += 1;
                    continue;
                }
                self.publish_staged_blob(&journal.attachment)?;
                self.ensure_metadata_file(&journal.attachment)?;
                self.remove_journal(&journal.attachment.id)?;
                report.recovered_count += 1;
            } else {
                report.missing_blob_count += 1;
            }
        }
        Ok(report)
    }

    /// Removes staged imports that have no durable AppData reference. Each
    /// removal is protected by the same persistent cleanup marker used by an
    /// explicit rollback.
    pub fn cleanup_unreferenced_pending(
        &self,
        referenced_attachment_ids: &HashSet<String>,
    ) -> MediaResult<CleanupRecoveryReport> {
        let _guard = self.lock()?;
        let mut report = CleanupRecoveryReport::default();
        let mut journal_paths = fs::read_dir(&self.pending_directory)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension().and_then(|value| value.to_str()) == Some("json")
                    && !path
                        .file_name()
                        .and_then(|value| value.to_str())
                        .is_some_and(|name| name.starts_with(CLEANUP_BLOCK_PREFIX))
            })
            .collect::<Vec<_>>();
        journal_paths.sort();
        for journal_path in journal_paths {
            let journal = match self.read_journal_path(&journal_path) {
                Ok(journal) => journal,
                Err(_) => {
                    report.malformed_marker_count += 1;
                    continue;
                }
            };
            if referenced_attachment_ids.contains(&journal.attachment.id) {
                continue;
            }
            match self.guarded_cleanup_attachment(&journal.attachment.id, "unreferenced-import") {
                Ok(_) => report.cleaned_count += 1,
                Err(_) => report.remaining_count += 1,
            }
        }
        let mut orphan_temporary_ids = HashSet::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if !file_type.is_file() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if let Some(attachment_id) = parse_root_temporary_attachment_id(&name) {
                orphan_temporary_ids.insert(attachment_id.to_string());
            }
        }
        for entry in fs::read_dir(&self.pending_directory)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if !file_type.is_file() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if let Some(attachment_id) = parse_journal_temporary_attachment_id(&name) {
                orphan_temporary_ids.insert(attachment_id.to_string());
            }
        }
        let mut orphan_temporary_ids = orphan_temporary_ids.into_iter().collect::<Vec<_>>();
        orphan_temporary_ids.sort();
        for attachment_id in orphan_temporary_ids {
            if referenced_attachment_ids.contains(&attachment_id)
                || safe_regular_child_exists(
                    &self.pending_directory,
                    &self.journal_path_unchecked(&attachment_id),
                )?
            {
                continue;
            }
            match self.guarded_cleanup_attachment(&attachment_id, "orphan-import-temporary") {
                Ok(_) => report.cleaned_count += 1,
                Err(_) => report.remaining_count += 1,
            }
        }
        Ok(report)
    }

    /// Removes committed blob/metadata pairs that are no longer referenced by
    /// any current note, recovery point, or named version in the durable
    /// AppData snapshot. Every attachment is deleted behind a persistent
    /// cleanup marker, so a crash at any file boundary is completed by
    /// `retry_cleanup_blocks` on the next workspace open.
    pub fn cleanup_unreferenced_committed(
        &self,
        referenced_attachment_ids: &HashSet<String>,
    ) -> MediaResult<CleanupRecoveryReport> {
        let _guard = self.lock()?;
        let mut report = CleanupRecoveryReport::default();
        let mut committed_ids = HashSet::<String>::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                return Err(DesktopNoteMediaError::UnsafeStorageLayout(entry.path()));
            }
            if file_type.is_dir() {
                if entry.path() != self.pending_directory {
                    return Err(DesktopNoteMediaError::UnsafeStorageLayout(entry.path()));
                }
                continue;
            }
            if !file_type.is_file() {
                return Err(DesktopNoteMediaError::UnsafeStorageLayout(entry.path()));
            }
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                return Err(DesktopNoteMediaError::UnsafeStorageLayout(entry.path()));
            };
            if name == IDENTITY_MARKER_FILE
                || name == IDENTITY_MARKER_TEMP_FILE
                || (name.starts_with(PENDING_BLOB_PREFIX) && name.ends_with(PENDING_BLOB_SUFFIX))
                || (name.starts_with(METADATA_TEMP_PREFIX) && name.ends_with(METADATA_TEMP_SUFFIX))
            {
                continue;
            }
            let attachment_id = parse_blob_file_name(&name).or_else(|| {
                name.strip_suffix(METADATA_SUFFIX)
                    .filter(|id| validate_attachment_id(id).is_ok())
            });
            let Some(attachment_id) = attachment_id else {
                return Err(DesktopNoteMediaError::UnsafeStorageLayout(entry.path()));
            };
            committed_ids.insert(attachment_id.to_string());
        }

        let mut committed_ids = committed_ids.into_iter().collect::<Vec<_>>();
        committed_ids.sort();
        for attachment_id in committed_ids {
            if referenced_attachment_ids.contains(&attachment_id) {
                continue;
            }
            match self.guarded_cleanup_attachment(&attachment_id, "unreferenced-committed") {
                Ok(_) => report.cleaned_count += 1,
                Err(_) => report.remaining_count += 1,
            }
        }
        Ok(report)
    }

    /// Retries every cleanup transaction that previously failed after its
    /// durable blocker was written. Broken marker files remain in place and
    /// continue to block encryption. Referenced or unresolved files keep their
    /// marker and bytes until a complete newer snapshot permits cleanup.
    pub fn retry_cleanup_blocks(
        &self,
        references: &crate::desktop_media_references::DesktopMediaReferenceScope,
    ) -> MediaResult<CleanupRecoveryReport> {
        let _guard = self.lock()?;
        let mut report = CleanupRecoveryReport::default();
        let mut markers = self.cleanup_block_marker_paths()?;
        markers.sort();
        for marker_path in markers {
            let marker = match self.read_cleanup_block_marker(&marker_path) {
                Ok(marker) => marker,
                Err(_) => {
                    report.malformed_marker_count += 1;
                    continue;
                }
            };
            // A durable retry marker records unfinished work, not fresh authority.
            if !references.is_complete() || references.ids().contains(&marker.attachment_id) {
                report.remaining_count += 1;
                continue;
            }
            match self.delete_attachment_artifacts_unlocked(&marker.attachment_id) {
                Ok(_) => match self.remove_cleanup_block_marker(&marker.attachment_id) {
                    Ok(()) => report.cleaned_count += 1,
                    Err(_) => report.remaining_count += 1,
                },
                Err(_) => report.remaining_count += 1,
            }
        }
        Ok(report)
    }

    /// Reports any durable media state that makes sealing a note unsafe.
    /// Besides explicit cleanup blockers, any pending artifact or committed
    /// blob absent from the workspace-wide AppData reference set is treated as
    /// plaintext evidence and fails closed.
    pub fn encryption_blockers(
        &self,
        workspace_attachment_ids: &HashSet<String>,
    ) -> MediaResult<MediaEncryptionBlockerReport> {
        let _guard = self.lock()?;
        let mut cleanup_marker_count = 0_usize;
        let mut pending_artifact_count = 0_usize;
        for entry in fs::read_dir(&self.pending_directory)? {
            let entry = entry?;
            let name = entry.file_name();
            if name.to_string_lossy().starts_with(CLEANUP_BLOCK_PREFIX) {
                cleanup_marker_count += 1;
            } else {
                pending_artifact_count += 1;
            }
        }
        let mut orphan_ids = HashSet::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                return Err(DesktopNoteMediaError::UnsafeStorageLayout(entry.path()));
            }
            if file_type.is_dir() {
                if entry.path() != self.pending_directory {
                    return Err(DesktopNoteMediaError::UnsafeStorageLayout(entry.path()));
                }
                continue;
            }
            if !file_type.is_file() {
                return Err(DesktopNoteMediaError::UnsafeStorageLayout(entry.path()));
            }
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                return Err(DesktopNoteMediaError::UnsafeStorageLayout(entry.path()));
            };
            if name == IDENTITY_MARKER_FILE {
                continue;
            }
            if name == IDENTITY_MARKER_TEMP_FILE
                || (name.starts_with(PENDING_BLOB_PREFIX) && name.ends_with(PENDING_BLOB_SUFFIX))
                || (name.starts_with(METADATA_TEMP_PREFIX) && name.ends_with(METADATA_TEMP_SUFFIX))
            {
                pending_artifact_count += 1;
                continue;
            }
            let attachment_id = parse_blob_file_name(&name).or_else(|| {
                name.strip_suffix(METADATA_SUFFIX)
                    .filter(|id| validate_attachment_id(id).is_ok())
            });
            let Some(attachment_id) = attachment_id else {
                return Err(DesktopNoteMediaError::UnsafeStorageLayout(entry.path()));
            };
            if !workspace_attachment_ids.contains(attachment_id) {
                orphan_ids.insert(attachment_id.to_string());
            }
        }
        Ok(MediaEncryptionBlockerReport {
            cleanup_marker_count,
            pending_artifact_count,
            orphan_attachment_count: orphan_ids.len(),
        })
    }

    pub fn blob_path(&self, attachment_id: &str) -> MediaResult<PathBuf> {
        validate_attachment_id(attachment_id)?;
        Ok(self.blob_path_unchecked(attachment_id))
    }

    pub fn blob_exists(&self, attachment_id: &str) -> MediaResult<bool> {
        let path = self.blob_path(attachment_id)?;
        safe_regular_child_exists(&self.root, &path)
    }

    /// Reads and verifies the durable manifest entry for one attachment only.
    /// This keeps an unrelated damaged blob from blocking reconciliation of
    /// every other AppData reference in the same account workspace.
    pub fn manifest_entry_for(
        &self,
        attachment_id: &str,
    ) -> MediaResult<Option<NoteMediaManifestEntry>> {
        let _guard = self.lock()?;
        validate_attachment_id(attachment_id)?;
        let blob_path = self.blob_path_unchecked(attachment_id);
        let metadata_path = self.metadata_path_unchecked(attachment_id);
        let blob_exists = safe_regular_child_exists(&self.root, &blob_path)?;
        let metadata_exists = safe_regular_child_exists(&self.root, &metadata_path)?;
        match (blob_exists, metadata_exists) {
            (false, false) => return Ok(None),
            (true, true) => {}
            _ => {
                return Err(DesktopNoteMediaError::PendingImportCorrupt(
                    attachment_id.to_string(),
                ));
            }
        }

        let encoded = read_limited_file(&metadata_path, MAX_JOURNAL_BYTES)?;
        let attachment: NoteAttachmentMetadata = serde_json::from_slice(&encoded)?;
        if attachment.id != attachment_id {
            return Err(DesktopNoteMediaError::PendingImportCorrupt(
                attachment_id.to_string(),
            ));
        }
        attachment.validate()?;
        self.verify_blob(&blob_path, &attachment)?;
        Ok(Some(manifest_entry(&attachment)))
    }

    /// Reads one blob after checking the caller's AppData or server metadata.
    pub fn read_blob(
        &self,
        attachment_id: &str,
        expected_sha256: &str,
        expected_size_bytes: i64,
    ) -> MediaResult<Vec<u8>> {
        let _guard = self.lock()?;
        let path = self.blob_path(attachment_id)?;
        require_regular_child(&self.root, &path)?;
        let expected_size = validate_expected_size(expected_size_bytes)?;
        let expected_sha = normalize_sha256(expected_sha256)?;
        let actual_size = fs::metadata(&path)?.len();
        if actual_size != expected_size {
            return Err(DesktopNoteMediaError::SizeMismatch {
                expected: expected_size,
                actual: actual_size,
            });
        }
        let mut bytes = Vec::with_capacity(actual_size as usize);
        File::open(&path)?.read_to_end(&mut bytes)?;
        if bytes.len() as u64 != expected_size {
            return Err(DesktopNoteMediaError::SizeMismatch {
                expected: expected_size,
                actual: bytes.len() as u64,
            });
        }
        let actual_sha = sha256_bytes(&bytes);
        if actual_sha != expected_sha {
            return Err(DesktopNoteMediaError::Sha256Mismatch {
                expected: expected_sha,
                actual: actual_sha,
            });
        }
        Ok(bytes)
    }

    /// Atomically publishes bytes returned by the media-download endpoint.
    /// The id, digest, size, MIME type, image format, and dimensions are all
    /// verified before the final file name is made visible.
    #[allow(clippy::too_many_arguments)]
    pub fn write_download_blob(
        &self,
        attachment_id: &str,
        expected_sha256: &str,
        expected_size_bytes: i64,
        expected_mime_type: &str,
        updated_at_epoch_millis: i64,
        content: &[u8],
    ) -> MediaResult<NoteMediaManifestEntry> {
        let _guard = self.lock()?;
        validate_attachment_id(attachment_id)?;
        let expected_size = validate_expected_size(expected_size_bytes)?;
        let expected_sha = normalize_sha256(expected_sha256)?;
        if content.len() as u64 != expected_size {
            return Err(DesktopNoteMediaError::SizeMismatch {
                expected: expected_size,
                actual: content.len() as u64,
            });
        }
        let actual_sha = sha256_bytes(content);
        if actual_sha != expected_sha {
            return Err(DesktopNoteMediaError::Sha256Mismatch {
                expected: expected_sha,
                actual: actual_sha,
            });
        }

        let staged_path = self.pending_blob_path_unchecked(attachment_id);
        ensure_absent_or_remove_regular(&self.root, &staged_path)?;
        write_synced_new_file(&staged_path, content)?;
        let inspected = match inspect_stored_media(&staged_path, expected_mime_type) {
            Ok(inspected) => inspected,
            Err(error) => {
                let _ = fs::remove_file(&staged_path);
                return Err(error);
            }
        };
        if let Some(expected_mime) = canonical_storage_mime(expected_mime_type)
            .filter(|mime| *mime != "application/octet-stream")
        {
            if expected_mime != inspected.mime_type {
                let _ = fs::remove_file(&staged_path);
                return Err(DesktopNoteMediaError::MimeTypeMismatch {
                    expected: expected_mime.to_string(),
                    actual: inspected.mime_type.to_string(),
                });
            }
        } else if !expected_mime_type.trim().is_empty()
            && !expected_mime_type.eq_ignore_ascii_case("application/octet-stream")
        {
            let _ = fs::remove_file(&staged_path);
            return Err(DesktopNoteMediaError::UnsupportedImageFormat);
        }

        let timestamp = updated_at_epoch_millis.max(1);
        let attachment = NoteAttachmentMetadata {
            id: attachment_id.to_string(),
            kind: if inspected.width > 0 {
                "IMAGE"
            } else {
                general_media_kind(inspected.mime_type).unwrap_or("FILE")
            }
            .to_string(),
            file_name: blob_file_name(attachment_id)?,
            display_name: blob_file_name(attachment_id)?,
            mime_type: inspected.mime_type.to_string(),
            width: inspected.width as i32,
            height: inspected.height as i32,
            size_bytes: expected_size as i64,
            sha256: expected_sha,
            created_at_epoch_millis: timestamp,
            updated_at_epoch_millis: timestamp,
        };
        let final_path = self.blob_path_unchecked(attachment_id);
        if safe_regular_child_exists(&self.root, &final_path)? {
            if let Err(error) = self.verify_blob(&final_path, &attachment) {
                let _ = fs::remove_file(&staged_path);
                return match error {
                    DesktopNoteMediaError::SizeMismatch { .. }
                    | DesktopNoteMediaError::Sha256Mismatch { .. }
                    | DesktopNoteMediaError::MimeTypeMismatch { .. }
                    | DesktopNoteMediaError::InvalidImage(_) => Err(
                        DesktopNoteMediaError::ExistingBlobConflict(attachment_id.to_string()),
                    ),
                    other => Err(other),
                };
            }
            fs::remove_file(&staged_path)?;
            sync_directory(&self.root)?;
        } else {
            fs::rename(&staged_path, &final_path)?;
            sync_directory(&self.root)?;
        }
        let effective_metadata = self.ensure_metadata_file(&attachment)?;
        self.verify_blob(&final_path, &attachment)?;
        Ok(manifest_entry(&effective_metadata))
    }

    /// Deletes exactly one caller-selected blob. This function never scans for
    /// or prunes unreferenced files; the caller must first account for current,
    /// revision, and named-version attachment references.
    pub fn delete_blob(&self, attachment_id: &str) -> MediaResult<bool> {
        let _guard = self.lock()?;
        validate_attachment_id(attachment_id)?;
        self.guarded_cleanup_attachment(attachment_id, "explicit-delete")
    }

    fn delete_attachment_artifacts_unlocked(&self, attachment_id: &str) -> MediaResult<bool> {
        validate_attachment_id(attachment_id)?;
        let mut deleted_blob = false;
        for path in [
            self.blob_path_unchecked(attachment_id),
            self.metadata_path_unchecked(attachment_id),
            self.pending_blob_path_unchecked(attachment_id),
            self.journal_path_unchecked(attachment_id),
            self.journal_temp_path_unchecked(attachment_id),
            self.metadata_temp_path_unchecked(attachment_id),
        ] {
            let parent = path
                .parent()
                .ok_or_else(|| DesktopNoteMediaError::UnsafeStorageLayout(path.to_path_buf()))?;
            let allowed_parent = parent == self.root || parent == self.pending_directory;
            if !allowed_parent {
                return Err(DesktopNoteMediaError::UnsafeStorageLayout(path));
            }
            if safe_regular_child_exists(parent, &path)? {
                if path == self.blob_path_unchecked(attachment_id) {
                    deleted_blob = true;
                }
                fs::remove_file(&path)?;
                sync_directory(parent)?;
            }
        }
        Ok(deleted_blob)
    }

    /// Scans committed blobs only. Every returned digest and size was checked
    /// against the durable sidecar and recomputed from the current bytes.
    pub fn scan_manifest(&self) -> MediaResult<Vec<NoteMediaManifestEntry>> {
        let _guard = self.lock()?;
        let mut entries = Vec::new();
        let mut blob_paths = fs::read_dir(&self.root)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .and_then(parse_blob_file_name)
                    .is_some()
            })
            .collect::<Vec<_>>();
        blob_paths.sort();

        for blob_path in blob_paths {
            require_regular_child(&self.root, &blob_path)?;
            let file_name = blob_path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| DesktopNoteMediaError::UnsafeStorageLayout(blob_path.clone()))?;
            let attachment_id = parse_blob_file_name(file_name)
                .ok_or_else(|| DesktopNoteMediaError::InvalidAttachmentId(file_name.to_string()))?;
            let metadata_path = self.metadata_path_unchecked(attachment_id);
            require_regular_child(&self.root, &metadata_path)?;
            let encoded = read_limited_file(&metadata_path, MAX_JOURNAL_BYTES)?;
            let attachment: NoteAttachmentMetadata = serde_json::from_slice(&encoded)?;
            if attachment.id != attachment_id {
                return Err(DesktopNoteMediaError::PendingImportCorrupt(
                    attachment_id.to_string(),
                ));
            }
            attachment.validate()?;
            self.verify_blob(&blob_path, &attachment)?;
            entries.push(manifest_entry(&attachment));
        }
        entries.sort_by(|left, right| left.attachment_id.cmp(&right.attachment_id));
        Ok(entries)
    }

    fn begin_import_internal(
        &self,
        source_path: &Path,
        display_name: Option<&str>,
        now: i64,
        forced_attachment_id: Option<&str>,
    ) -> MediaResult<PendingMediaImport> {
        self.stage_import(source_path, display_name, now, forced_attachment_id, None)
    }

    fn stage_import(
        &self,
        source_path: &Path,
        display_name: Option<&str>,
        now: i64,
        forced_attachment_id: Option<&str>,
        media: Option<(&str, &'static str)>,
    ) -> MediaResult<PendingMediaImport> {
        let _guard = self.lock()?;
        let source_metadata = fs::metadata(source_path)?;
        if !source_metadata.is_file() {
            return Err(DesktopNoteMediaError::InvalidImage(
                "the selected path is not a regular file".to_string(),
            ));
        }
        if source_metadata.len() == 0 {
            return Err(DesktopNoteMediaError::EmptyImage);
        }
        if source_metadata.len() > MAX_IMAGE_BYTES {
            return Err(DesktopNoteMediaError::ImageTooLarge {
                actual: source_metadata.len(),
                maximum: MAX_IMAGE_BYTES,
            });
        }

        let attachment_id = match forced_attachment_id {
            Some(id) => {
                validate_attachment_id(id)?;
                id.to_string()
            }
            None => generate_attachment_id(),
        };
        let staged_path = self.pending_blob_path_unchecked(&attachment_id);
        ensure_absent_or_remove_regular(&self.root, &staged_path)?;
        let (size_bytes, sha256) = match copy_source_to_synced_temp(source_path, &staged_path) {
            Ok(value) => value,
            Err(error) => {
                let _ = fs::remove_file(&staged_path);
                return Err(error);
            }
        };
        // The journal is synced in a different directory. Persist the staged
        // blob's directory entry first so a durable journal can never point at
        // a merely cached temporary name.
        if let Err(error) = sync_directory(&self.root) {
            let _ = fs::remove_file(&staged_path);
            return Err(error);
        }
        let inspected = match media
            .map(|(_, mime)| inspect_stored_media(&staged_path, mime))
            .unwrap_or_else(|| inspect_image(&staged_path))
        {
            Ok(inspected) => inspected,
            Err(error) => {
                let _ = fs::remove_file(&staged_path);
                return Err(error);
            }
        };
        let file_name = blob_file_name(&attachment_id)?;
        let display_name = display_name
            .map(sanitize_display_name)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| file_name.clone());
        let timestamp = now.max(1);
        let attachment = NoteAttachmentMetadata {
            id: attachment_id.clone(),
            kind: if inspected.width > 0 {
                "IMAGE"
            } else {
                general_media_kind(inspected.mime_type).unwrap_or("FILE")
            }
            .to_string(),
            file_name,
            display_name,
            mime_type: inspected.mime_type.to_string(),
            width: inspected.width as i32,
            height: inspected.height as i32,
            size_bytes: size_bytes as i64,
            sha256,
            created_at_epoch_millis: timestamp,
            updated_at_epoch_millis: timestamp,
        };
        attachment.validate()?;
        let journal = PendingJournal {
            state_version: JOURNAL_STATE_VERSION,
            attachment: attachment.clone(),
            journaled_at_epoch_millis: timestamp,
        };
        if let Err(error) = self.write_journal(&journal) {
            let _ = fs::remove_file(&staged_path);
            return Err(error);
        }
        Ok(PendingMediaImport {
            store_root: self.root.clone(),
            attachment,
        })
    }

    fn finalize_journal(&self, journal: &PendingJournal) -> MediaResult<()> {
        journal.attachment.validate()?;
        let final_path = self.blob_path_unchecked(&journal.attachment.id);
        let staged_path = self.pending_blob_path_unchecked(&journal.attachment.id);
        if safe_regular_child_exists(&self.root, &final_path)? {
            self.verify_blob(&final_path, &journal.attachment)
                .map_err(|_| {
                    DesktopNoteMediaError::ExistingBlobConflict(journal.attachment.id.clone())
                })?;
            if safe_regular_child_exists(&self.root, &staged_path)? {
                self.verify_blob(&staged_path, &journal.attachment)?;
                fs::remove_file(&staged_path)?;
                sync_directory(&self.root)?;
            }
        } else {
            self.verify_blob(&staged_path, &journal.attachment)?;
            self.publish_staged_blob(&journal.attachment)?;
        }
        self.ensure_metadata_file(&journal.attachment)?;
        self.verify_blob(&final_path, &journal.attachment)?;
        self.remove_journal(&journal.attachment.id)
    }

    fn publish_staged_blob(&self, attachment: &NoteAttachmentMetadata) -> MediaResult<()> {
        let staged_path = self.pending_blob_path_unchecked(&attachment.id);
        let final_path = self.blob_path_unchecked(&attachment.id);
        require_regular_child(&self.root, &staged_path)?;
        if safe_regular_child_exists(&self.root, &final_path)? {
            return Err(DesktopNoteMediaError::ExistingBlobConflict(
                attachment.id.clone(),
            ));
        }
        fs::rename(&staged_path, &final_path)?;
        sync_directory(&self.root)?;
        Ok(())
    }

    fn verify_blob(&self, path: &Path, attachment: &NoteAttachmentMetadata) -> MediaResult<()> {
        require_regular_child(&self.root, path)?;
        let expected_size = validate_expected_size(attachment.size_bytes)?;
        let actual_size = fs::metadata(path)?.len();
        if actual_size != expected_size {
            return Err(DesktopNoteMediaError::SizeMismatch {
                expected: expected_size,
                actual: actual_size,
            });
        }
        let actual_sha = sha256_file(path)?;
        if actual_sha != normalize_sha256(&attachment.sha256)? {
            return Err(DesktopNoteMediaError::Sha256Mismatch {
                expected: attachment.sha256.clone(),
                actual: actual_sha,
            });
        }
        let inspected = inspect_stored_media(path, &attachment.mime_type)?;
        if inspected.width as i32 != attachment.width
            || inspected.height as i32 != attachment.height
        {
            return Err(DesktopNoteMediaError::InvalidImage(
                "stored dimensions differ from attachment metadata".to_string(),
            ));
        }
        let expected_mime = canonical_storage_mime(&attachment.mime_type)
            .ok_or(DesktopNoteMediaError::UnsupportedImageFormat)?;
        if inspected.mime_type != expected_mime {
            return Err(DesktopNoteMediaError::MimeTypeMismatch {
                expected: expected_mime.to_string(),
                actual: inspected.mime_type.to_string(),
            });
        }
        Ok(())
    }

    fn ensure_metadata_file(
        &self,
        attachment: &NoteAttachmentMetadata,
    ) -> MediaResult<NoteAttachmentMetadata> {
        attachment.validate()?;
        let metadata_path = self.metadata_path_unchecked(&attachment.id);
        if safe_regular_child_exists(&self.root, &metadata_path)? {
            let encoded = read_limited_file(&metadata_path, MAX_JOURNAL_BYTES)?;
            let stored: NoteAttachmentMetadata = serde_json::from_slice(&encoded)?;
            stored.validate()?;
            if !same_blob_identity(&stored, attachment) {
                return Err(DesktopNoteMediaError::ExistingBlobConflict(
                    attachment.id.clone(),
                ));
            }
            let merged = merge_metadata_revision(&stored, attachment);
            if merged == stored {
                return Ok(stored);
            }
            let temporary_path = self.metadata_temp_path_unchecked(&attachment.id);
            ensure_absent_or_remove_regular(&self.root, &temporary_path)?;
            write_synced_new_file(&temporary_path, &serde_json::to_vec(&merged)?)?;
            atomic_replace_existing(&temporary_path, &metadata_path)?;
            sync_directory(&self.root)?;
            let verified: NoteAttachmentMetadata =
                serde_json::from_slice(&read_limited_file(&metadata_path, MAX_JOURNAL_BYTES)?)?;
            if verified != merged {
                return Err(DesktopNoteMediaError::PendingImportCorrupt(
                    attachment.id.clone(),
                ));
            }
            return Ok(merged);
        }

        let temporary_path = self.metadata_temp_path_unchecked(&attachment.id);
        ensure_absent_or_remove_regular(&self.root, &temporary_path)?;
        let encoded = serde_json::to_vec(attachment)?;
        write_synced_new_file(&temporary_path, &encoded)?;
        fs::rename(&temporary_path, &metadata_path)?;
        sync_directory(&self.root)?;
        let verified: NoteAttachmentMetadata =
            serde_json::from_slice(&read_limited_file(&metadata_path, MAX_JOURNAL_BYTES)?)?;
        if verified != *attachment {
            return Err(DesktopNoteMediaError::PendingImportCorrupt(
                attachment.id.clone(),
            ));
        }
        Ok(verified)
    }

    fn write_journal(&self, journal: &PendingJournal) -> MediaResult<()> {
        journal.attachment.validate()?;
        let journal_path = self.journal_path_unchecked(&journal.attachment.id);
        if safe_regular_child_exists(&self.pending_directory, &journal_path)? {
            return Err(DesktopNoteMediaError::ExistingBlobConflict(
                journal.attachment.id.clone(),
            ));
        }
        let temporary_path = self.journal_temp_path_unchecked(&journal.attachment.id);
        ensure_absent_or_remove_regular(&self.pending_directory, &temporary_path)?;
        let encoded = serde_json::to_vec(journal)?;
        if encoded.is_empty() || encoded.len() as u64 > MAX_JOURNAL_BYTES {
            return Err(DesktopNoteMediaError::PendingImportCorrupt(
                journal.attachment.id.clone(),
            ));
        }
        write_synced_new_file(&temporary_path, &encoded)?;
        fs::rename(&temporary_path, &journal_path)?;
        sync_directory(&self.pending_directory)?;
        let verified = self.read_expected_journal(&journal.attachment.id)?;
        if verified != *journal {
            return Err(DesktopNoteMediaError::PendingImportCorrupt(
                journal.attachment.id.clone(),
            ));
        }
        Ok(())
    }

    fn read_expected_journal(&self, attachment_id: &str) -> MediaResult<PendingJournal> {
        validate_attachment_id(attachment_id)?;
        let path = self.journal_path_unchecked(attachment_id);
        if !safe_regular_child_exists(&self.pending_directory, &path)? {
            return Err(DesktopNoteMediaError::PendingImportMissing(
                attachment_id.to_string(),
            ));
        }
        self.read_journal_path(&path)
    }

    fn read_journal_path(&self, path: &Path) -> MediaResult<PendingJournal> {
        require_regular_child(&self.pending_directory, path)?;
        let encoded = read_limited_file(path, MAX_JOURNAL_BYTES)?;
        let journal: PendingJournal = serde_json::from_slice(&encoded)?;
        if journal.state_version != JOURNAL_STATE_VERSION || journal.journaled_at_epoch_millis <= 0
        {
            return Err(DesktopNoteMediaError::PendingImportCorrupt(
                journal.attachment.id,
            ));
        }
        journal.attachment.validate()?;
        let expected_path = self.journal_path_unchecked(&journal.attachment.id);
        if path != expected_path {
            return Err(DesktopNoteMediaError::PendingImportCorrupt(
                journal.attachment.id,
            ));
        }
        Ok(journal)
    }

    fn remove_journal(&self, attachment_id: &str) -> MediaResult<()> {
        let path = self.journal_path_unchecked(attachment_id);
        if safe_regular_child_exists(&self.pending_directory, &path)? {
            fs::remove_file(path)?;
            sync_directory(&self.pending_directory)?;
        }
        Ok(())
    }

    fn guarded_cleanup_attachment(&self, attachment_id: &str, reason: &str) -> MediaResult<bool> {
        self.guarded_cleanup_attachment_with(attachment_id, reason, |store, id| {
            store.delete_attachment_artifacts_unlocked(id)
        })
    }

    fn guarded_cleanup_attachment_with<F>(
        &self,
        attachment_id: &str,
        reason: &str,
        cleanup: F,
    ) -> MediaResult<bool>
    where
        F: FnOnce(&Self, &str) -> MediaResult<bool>,
    {
        validate_attachment_id(attachment_id)?;
        self.write_cleanup_block_marker(attachment_id, reason)?;
        let deleted = cleanup(self, attachment_id)?;
        self.remove_cleanup_block_marker(attachment_id)?;
        Ok(deleted)
    }

    fn write_cleanup_block_marker(&self, attachment_id: &str, reason: &str) -> MediaResult<()> {
        validate_attachment_id(attachment_id)?;
        let marker_path = self.cleanup_block_marker_path_unchecked(attachment_id);
        if safe_regular_child_exists(&self.pending_directory, &marker_path)? {
            let marker = self.read_cleanup_block_marker(&marker_path)?;
            if marker.attachment_id == attachment_id {
                return Ok(());
            }
            return Err(DesktopNoteMediaError::PendingImportCorrupt(
                attachment_id.to_string(),
            ));
        }
        let marker = CleanupBlockMarker {
            state_version: CLEANUP_BLOCK_STATE_VERSION,
            attachment_id: attachment_id.to_string(),
            reason: sanitize_cleanup_reason(reason),
            created_at_epoch_millis: current_time_millis().max(1),
        };
        let encoded = serde_json::to_vec(&marker)?;
        let temporary_path = self.cleanup_block_temp_path_unchecked(attachment_id);
        if safe_regular_child_exists(&self.pending_directory, &temporary_path)? {
            let temporary_marker = self.read_cleanup_block_marker(&temporary_path)?;
            if temporary_marker.attachment_id != attachment_id {
                return Err(DesktopNoteMediaError::PendingImportCorrupt(
                    attachment_id.to_string(),
                ));
            }
            fs::rename(&temporary_path, &marker_path)?;
            sync_directory(&self.pending_directory)?;
            return Ok(());
        }
        write_synced_new_file(&temporary_path, &encoded)?;
        fs::rename(&temporary_path, &marker_path)?;
        sync_directory(&self.pending_directory)?;
        if self.read_cleanup_block_marker(&marker_path)? != marker {
            return Err(DesktopNoteMediaError::PendingImportCorrupt(
                attachment_id.to_string(),
            ));
        }
        Ok(())
    }

    fn read_cleanup_block_marker(&self, path: &Path) -> MediaResult<CleanupBlockMarker> {
        require_regular_child(&self.pending_directory, path)?;
        let encoded = read_limited_file(path, MAX_JOURNAL_BYTES)?;
        let marker: CleanupBlockMarker = serde_json::from_slice(&encoded)?;
        if marker.state_version != CLEANUP_BLOCK_STATE_VERSION
            || marker.created_at_epoch_millis <= 0
            || marker.reason.is_empty()
        {
            return Err(DesktopNoteMediaError::PendingImportCorrupt(
                marker.attachment_id,
            ));
        }
        validate_attachment_id(&marker.attachment_id)?;
        if path != self.cleanup_block_marker_path_unchecked(&marker.attachment_id)
            && path != self.cleanup_block_temp_path_unchecked(&marker.attachment_id)
        {
            return Err(DesktopNoteMediaError::PendingImportCorrupt(
                marker.attachment_id,
            ));
        }
        Ok(marker)
    }

    fn cleanup_block_marker_paths(&self) -> MediaResult<Vec<PathBuf>> {
        let mut paths = Vec::new();
        for entry in fs::read_dir(&self.pending_directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if !name.starts_with(CLEANUP_BLOCK_PREFIX) {
                continue;
            }
            if name.ends_with(CLEANUP_BLOCK_TEMP_SUFFIX) {
                let temporary_path = entry.path();
                let marker = self.read_cleanup_block_marker(&temporary_path)?;
                let final_path = self.cleanup_block_marker_path_unchecked(&marker.attachment_id);
                if safe_regular_child_exists(&self.pending_directory, &final_path)? {
                    fs::remove_file(&temporary_path)?;
                } else {
                    fs::rename(&temporary_path, &final_path)?;
                }
                sync_directory(&self.pending_directory)?;
                if !paths.contains(&final_path) {
                    paths.push(final_path);
                }
            } else if name.ends_with(CLEANUP_BLOCK_SUFFIX) {
                paths.push(entry.path());
            }
        }
        Ok(paths)
    }

    fn remove_cleanup_block_marker(&self, attachment_id: &str) -> MediaResult<()> {
        let path = self.cleanup_block_marker_path_unchecked(attachment_id);
        if safe_regular_child_exists(&self.pending_directory, &path)? {
            fs::remove_file(path)?;
            sync_directory(&self.pending_directory)?;
        }
        Ok(())
    }

    fn ensure_workspace_identity_marker(&self, fingerprint: &str) -> MediaResult<()> {
        validate_sha256(fingerprint)?;
        let marker_path = self.root.join(IDENTITY_MARKER_FILE);
        let temporary_path = self.root.join(IDENTITY_MARKER_TEMP_FILE);
        if safe_regular_child_exists(&self.root, &marker_path)? {
            let marker = read_workspace_identity_marker(&marker_path)?;
            if marker.workspace_identity_sha256 != fingerprint {
                return Err(DesktopNoteMediaError::WorkspaceIdentityMismatch);
            }
            return Ok(());
        }

        if safe_regular_child_exists(&self.root, &temporary_path)? {
            let marker = read_workspace_identity_marker(&temporary_path)?;
            if marker.workspace_identity_sha256 != fingerprint {
                return Err(DesktopNoteMediaError::WorkspaceIdentityMismatch);
            }
            fs::rename(&temporary_path, &marker_path)?;
            sync_directory(&self.root)?;
            return Ok(());
        }

        let marker = WorkspaceIdentityMarker {
            state_version: IDENTITY_MARKER_STATE_VERSION,
            workspace_identity_sha256: fingerprint.to_string(),
        };
        let encoded = serde_json::to_vec(&marker)?;
        write_synced_new_file(&temporary_path, &encoded)?;
        fs::rename(&temporary_path, &marker_path)?;
        sync_directory(&self.root)?;
        if read_workspace_identity_marker(&marker_path)? != marker {
            return Err(DesktopNoteMediaError::WorkspaceIdentityMarkerCorrupt);
        }
        Ok(())
    }

    fn lock(&self) -> MediaResult<MutexGuard<'_, ()>> {
        self.operation_lock
            .lock()
            .map_err(|_| DesktopNoteMediaError::LockPoisoned)
    }

    fn blob_path_unchecked(&self, attachment_id: &str) -> PathBuf {
        self.root.join(format!("{attachment_id}{BLOB_SUFFIX}"))
    }

    fn metadata_path_unchecked(&self, attachment_id: &str) -> PathBuf {
        self.root.join(format!("{attachment_id}{METADATA_SUFFIX}"))
    }

    fn pending_blob_path_unchecked(&self, attachment_id: &str) -> PathBuf {
        self.root.join(format!(
            "{PENDING_BLOB_PREFIX}{attachment_id}{PENDING_BLOB_SUFFIX}"
        ))
    }

    fn metadata_temp_path_unchecked(&self, attachment_id: &str) -> PathBuf {
        self.root.join(format!(
            "{METADATA_TEMP_PREFIX}{attachment_id}{METADATA_TEMP_SUFFIX}"
        ))
    }

    fn journal_path_unchecked(&self, attachment_id: &str) -> PathBuf {
        self.pending_directory
            .join(format!("{attachment_id}{JOURNAL_SUFFIX}"))
    }

    fn journal_temp_path_unchecked(&self, attachment_id: &str) -> PathBuf {
        self.pending_directory
            .join(format!("{attachment_id}{JOURNAL_TEMP_SUFFIX}"))
    }

    fn cleanup_block_marker_path_unchecked(&self, attachment_id: &str) -> PathBuf {
        self.pending_directory.join(format!(
            "{CLEANUP_BLOCK_PREFIX}{attachment_id}{CLEANUP_BLOCK_SUFFIX}"
        ))
    }

    fn cleanup_block_temp_path_unchecked(&self, attachment_id: &str) -> PathBuf {
        self.pending_directory.join(format!(
            "{CLEANUP_BLOCK_PREFIX}{attachment_id}{CLEANUP_BLOCK_TEMP_SUFFIX}"
        ))
    }
}

fn sanitize_cleanup_reason(reason: &str) -> String {
    let sanitized = reason
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        .take(64)
        .collect::<String>();
    if sanitized.is_empty() {
        "cleanup".to_string()
    } else {
        sanitized
    }
}

fn shared_workspace_lock(root: &Path) -> MediaResult<Arc<Mutex<()>>> {
    let registry = WORKSPACE_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut registry = registry
        .lock()
        .map_err(|_| DesktopNoteMediaError::LockPoisoned)?;
    registry.retain(|_, weak| weak.strong_count() > 0);
    if let Some(existing) = registry.get(root).and_then(Weak::upgrade) {
        return Ok(existing);
    }
    let lock = Arc::new(Mutex::new(()));
    registry.insert(root.to_path_buf(), Arc::downgrade(&lock));
    Ok(lock)
}

fn workspace_identity_fingerprint(identity: &str) -> MediaResult<String> {
    let identity = identity.trim();
    if identity.is_empty()
        || identity.len() > MAX_WORKSPACE_IDENTITY_BYTES
        || identity.chars().any(char::is_control)
    {
        return Err(DesktopNoteMediaError::InvalidWorkspaceIdentity);
    }
    let mut digest = Sha256::new();
    digest.update(IDENTITY_FINGERPRINT_DOMAIN);
    digest.update((identity.len() as u64).to_be_bytes());
    digest.update(identity.as_bytes());
    Ok(format!("{:x}", digest.finalize()))
}

fn verify_existing_workspace_identity_marker(root: &Path, fingerprint: &str) -> MediaResult<()> {
    validate_sha256(fingerprint)?;
    let marker_path = root.join(IDENTITY_MARKER_FILE);
    let temporary_path = root.join(IDENTITY_MARKER_TEMP_FILE);
    let mut observed = 0_usize;
    for path in [&marker_path, &temporary_path] {
        if !safe_regular_child_exists(root, path)? {
            continue;
        }
        observed += 1;
        let marker = read_workspace_identity_marker(path)?;
        if marker.workspace_identity_sha256 != fingerprint {
            return Err(DesktopNoteMediaError::WorkspaceIdentityMismatch);
        }
    }
    if observed == 0 {
        return Err(DesktopNoteMediaError::WorkspaceIdentityMarkerCorrupt);
    }
    Ok(())
}

fn read_workspace_identity_marker(path: &Path) -> MediaResult<WorkspaceIdentityMarker> {
    let encoded = read_limited_file(path, 4096)
        .map_err(|_| DesktopNoteMediaError::WorkspaceIdentityMarkerCorrupt)?;
    let marker: WorkspaceIdentityMarker = serde_json::from_slice(&encoded)
        .map_err(|_| DesktopNoteMediaError::WorkspaceIdentityMarkerCorrupt)?;
    if marker.state_version != IDENTITY_MARKER_STATE_VERSION
        || normalize_sha256(&marker.workspace_identity_sha256).is_err()
    {
        return Err(DesktopNoteMediaError::WorkspaceIdentityMarkerCorrupt);
    }
    Ok(marker)
}

fn validate_attachment_id(attachment_id: &str) -> MediaResult<()> {
    let bytes = attachment_id.as_bytes();
    let valid = (1..=128).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    if valid {
        Ok(())
    } else {
        Err(DesktopNoteMediaError::InvalidAttachmentId(
            attachment_id.to_string(),
        ))
    }
}

fn blob_file_name(attachment_id: &str) -> MediaResult<String> {
    validate_attachment_id(attachment_id)?;
    Ok(format!("{attachment_id}{BLOB_SUFFIX}"))
}

fn parse_blob_file_name(file_name: &str) -> Option<&str> {
    let attachment_id = file_name.strip_suffix(BLOB_SUFFIX)?;
    validate_attachment_id(attachment_id).ok()?;
    Some(attachment_id)
}

fn parse_root_temporary_attachment_id(file_name: &str) -> Option<&str> {
    let attachment_id = file_name
        .strip_prefix(PENDING_BLOB_PREFIX)
        .and_then(|value| value.strip_suffix(PENDING_BLOB_SUFFIX))
        .or_else(|| {
            file_name
                .strip_prefix(METADATA_TEMP_PREFIX)
                .and_then(|value| value.strip_suffix(METADATA_TEMP_SUFFIX))
        })?;
    validate_attachment_id(attachment_id).ok()?;
    Some(attachment_id)
}

fn parse_journal_temporary_attachment_id(file_name: &str) -> Option<&str> {
    if file_name.starts_with(CLEANUP_BLOCK_PREFIX) {
        return None;
    }
    let attachment_id = file_name.strip_suffix(JOURNAL_TEMP_SUFFIX)?;
    validate_attachment_id(attachment_id).ok()?;
    Some(attachment_id)
}

fn validate_sha256(value: &str) -> MediaResult<()> {
    normalize_sha256(value).map(|_| ())
}

fn normalize_sha256(value: &str) -> MediaResult<String> {
    let value = value.trim();
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(value.to_ascii_lowercase())
    } else {
        Err(DesktopNoteMediaError::InvalidSha256(value.to_string()))
    }
}

fn validate_expected_size(value: i64) -> MediaResult<u64> {
    if value <= 0 {
        return Err(DesktopNoteMediaError::InvalidExpectedSize(value));
    }
    let value = value as u64;
    if value > MAX_IMAGE_BYTES {
        return Err(DesktopNoteMediaError::ImageTooLarge {
            actual: value,
            maximum: MAX_IMAGE_BYTES,
        });
    }
    Ok(value)
}

fn validate_dimensions(width: u32, height: u32) -> MediaResult<()> {
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if width == 0
        || height == 0
        || width > MAX_IMAGE_DIMENSION
        || height > MAX_IMAGE_DIMENSION
        || pixels > MAX_IMAGE_PIXELS
    {
        return Err(DesktopNoteMediaError::ImageDimensionsTooLarge {
            width,
            height,
            maximum_dimension: MAX_IMAGE_DIMENSION,
            maximum_pixels: MAX_IMAGE_PIXELS,
        });
    }
    Ok(())
}

fn inspect_image(path: &Path) -> MediaResult<InspectedImage> {
    let file_length = fs::metadata(path)?.len();
    if file_length == 0 {
        return Err(DesktopNoteMediaError::EmptyImage);
    }
    if file_length > MAX_IMAGE_BYTES {
        return Err(DesktopNoteMediaError::ImageTooLarge {
            actual: file_length,
            maximum: MAX_IMAGE_BYTES,
        });
    }
    let prefix_length = file_length.min(MAX_IMAGE_HEADER_BYTES as u64) as usize;
    let mut prefix = Vec::with_capacity(prefix_length);
    File::open(path)?
        .take(prefix_length as u64)
        .read_to_end(&mut prefix)?;
    if prefix.len() != prefix_length {
        return Err(invalid_image(
            "the image header was truncated while reading",
        ));
    }
    inspect_image_header(&prefix, file_length)
}

fn inspect_image_header(header: &[u8], file_length: u64) -> MediaResult<InspectedImage> {
    if header.starts_with(b"\x89PNG\r\n\x1a\n") {
        inspect_png_header(header, file_length)
    } else if header.starts_with(&[0xff, 0xd8]) {
        inspect_jpeg_header(header, file_length)
    } else if header.starts_with(b"GIF87a") || header.starts_with(b"GIF89a") {
        inspect_gif_header(header, file_length)
    } else if header.starts_with(b"BM") {
        inspect_bmp_header(header, file_length)
    } else if header.len() >= 12 && &header[..4] == b"RIFF" && &header[8..12] == b"WEBP" {
        inspect_webp_header(header, file_length)
    } else {
        Err(DesktopNoteMediaError::UnsupportedImageFormat)
    }
}

fn inspect_png_header(header: &[u8], file_length: u64) -> MediaResult<InspectedImage> {
    const PNG_HEADER_BYTES: usize = 33;
    if file_length < PNG_HEADER_BYTES as u64 || header.len() < PNG_HEADER_BYTES {
        return Err(invalid_image("truncated PNG IHDR chunk"));
    }
    let chunk_length =
        read_u32_be(header, 8).ok_or_else(|| invalid_image("truncated PNG chunk length"))?;
    if chunk_length != 13 || &header[12..16] != b"IHDR" {
        return Err(invalid_image("PNG must begin with a 13-byte IHDR chunk"));
    }
    let expected_crc =
        read_u32_be(header, 29).ok_or_else(|| invalid_image("truncated PNG IHDR checksum"))?;
    if png_crc32(&header[12..29]) != expected_crc {
        return Err(invalid_image("PNG IHDR checksum is invalid"));
    }

    let width = read_u32_be(header, 16).ok_or_else(|| invalid_image("PNG width is missing"))?;
    let height = read_u32_be(header, 20).ok_or_else(|| invalid_image("PNG height is missing"))?;
    let bit_depth = header[24];
    let color_type = header[25];
    let valid_bit_depth = match color_type {
        0 => matches!(bit_depth, 1 | 2 | 4 | 8 | 16),
        2 => matches!(bit_depth, 8 | 16),
        3 => matches!(bit_depth, 1 | 2 | 4 | 8),
        4 | 6 => matches!(bit_depth, 8 | 16),
        _ => false,
    };
    if !valid_bit_depth || header[26] != 0 || header[27] != 0 || header[28] > 1 {
        return Err(invalid_image("PNG IHDR fields are invalid"));
    }
    inspected_image("image/png", width, height)
}

fn inspect_jpeg_header(header: &[u8], file_length: u64) -> MediaResult<InspectedImage> {
    if file_length < 4 || header.len() < 4 {
        return Err(invalid_image("truncated JPEG header"));
    }
    let mut offset = 2_usize;
    while (offset as u64) < file_length {
        if offset >= header.len() {
            return Err(invalid_image(
                "JPEG dimensions exceed the bounded header inspection limit",
            ));
        }
        if header[offset] != 0xff {
            return Err(invalid_image("JPEG marker prefix is invalid"));
        }
        while offset < header.len() && header[offset] == 0xff {
            offset += 1;
        }
        if offset >= header.len() {
            return Err(invalid_image("truncated JPEG marker"));
        }
        let marker = header[offset];
        offset += 1;
        if marker == 0x00 {
            return Err(invalid_image(
                "unexpected JPEG byte stuffing before image data",
            ));
        }
        if marker == 0xd9 || marker == 0xda {
            return Err(invalid_image("JPEG has no frame dimensions"));
        }
        if marker == 0x01 || (0xd0..=0xd8).contains(&marker) {
            continue;
        }
        let segment_length = read_u16_be(header, offset)
            .ok_or_else(|| invalid_image("truncated JPEG segment length"))?
            as usize;
        if segment_length < 2 {
            return Err(invalid_image("JPEG segment length is invalid"));
        }
        let segment_end = offset
            .checked_add(segment_length)
            .ok_or_else(|| invalid_image("JPEG segment length overflow"))?;
        if segment_end as u64 > file_length {
            return Err(invalid_image("truncated JPEG segment"));
        }
        if segment_end > header.len() {
            return Err(invalid_image(
                "JPEG dimensions exceed the bounded header inspection limit",
            ));
        }
        if is_jpeg_start_of_frame(marker) {
            if segment_length < 11 {
                return Err(invalid_image("JPEG frame header is too short"));
            }
            let precision = header[offset + 2];
            let height = u32::from(read_u16_be(header, offset + 3).unwrap_or(0));
            let width = u32::from(read_u16_be(header, offset + 5).unwrap_or(0));
            let components = header[offset + 7] as usize;
            if precision == 0
                || precision > 16
                || !(1..=4).contains(&components)
                || segment_length != 8 + components * 3
            {
                return Err(invalid_image("JPEG frame fields are invalid"));
            }
            for component in 0..components {
                let component_offset = offset + 8 + component * 3;
                let sampling = header[component_offset + 1];
                if header[component_offset] == 0
                    || sampling >> 4 == 0
                    || sampling & 0x0f == 0
                    || header[component_offset + 2] > 3
                {
                    return Err(invalid_image("JPEG component fields are invalid"));
                }
            }
            return inspected_image("image/jpeg", width, height);
        }
        offset = segment_end;
    }
    Err(invalid_image("JPEG has no frame dimensions"))
}

fn is_jpeg_start_of_frame(marker: u8) -> bool {
    matches!(
        marker,
        0xc0 | 0xc1 | 0xc2 | 0xc3 | 0xc5 | 0xc6 | 0xc7 | 0xc9 | 0xca | 0xcb | 0xcd | 0xce | 0xcf
    )
}

fn inspect_gif_header(header: &[u8], file_length: u64) -> MediaResult<InspectedImage> {
    const GIF_LOGICAL_SCREEN_BYTES: usize = 13;
    if file_length < GIF_LOGICAL_SCREEN_BYTES as u64 || header.len() < GIF_LOGICAL_SCREEN_BYTES {
        return Err(invalid_image("truncated GIF logical screen descriptor"));
    }
    let width = u32::from(read_u16_le(header, 6).unwrap_or(0));
    let height = u32::from(read_u16_le(header, 8).unwrap_or(0));
    let packed = header[10];
    if packed & 0x80 != 0 {
        let color_table_bytes = 3_u64 << (u32::from(packed & 0x07) + 1);
        if file_length < GIF_LOGICAL_SCREEN_BYTES as u64 + color_table_bytes {
            return Err(invalid_image("truncated GIF global color table"));
        }
    }
    inspected_image("image/gif", width, height)
}

fn inspect_bmp_header(header: &[u8], file_length: u64) -> MediaResult<InspectedImage> {
    if file_length < 26 || header.len() < 26 {
        return Err(invalid_image("truncated BMP header"));
    }
    let declared_file_size = u64::from(read_u32_le(header, 2).unwrap_or(0));
    let reserved_one = read_u16_le(header, 6).unwrap_or(u16::MAX);
    let reserved_two = read_u16_le(header, 8).unwrap_or(u16::MAX);
    let pixel_offset = u64::from(read_u32_le(header, 10).unwrap_or(0));
    let dib_size = read_u32_le(header, 14).unwrap_or(0) as usize;
    if declared_file_size < 26
        || declared_file_size > file_length
        || reserved_one != 0
        || reserved_two != 0
    {
        return Err(invalid_image("BMP file header is invalid or truncated"));
    }
    let dib_end = 14_usize
        .checked_add(dib_size)
        .ok_or_else(|| invalid_image("BMP DIB header length overflow"))?;
    if dib_end as u64 > file_length || dib_end > header.len() {
        return Err(invalid_image("truncated BMP DIB header"));
    }
    if pixel_offset < dib_end as u64
        || pixel_offset > declared_file_size
        || pixel_offset > file_length
    {
        return Err(invalid_image("BMP pixel offset is invalid"));
    }

    let (width, height, planes, bits_per_pixel, compression) = match dib_size {
        12 => (
            u32::from(read_u16_le(header, 18).unwrap_or(0)),
            u32::from(read_u16_le(header, 20).unwrap_or(0)),
            read_u16_le(header, 22).unwrap_or(0),
            read_u16_le(header, 24).unwrap_or(0),
            0,
        ),
        40 | 52 | 56 | 64 | 108 | 124 => {
            let signed_width = read_i32_le(header, 18).unwrap_or(0);
            let signed_height = read_i32_le(header, 22).unwrap_or(0);
            let width = u32::try_from(signed_width)
                .map_err(|_| invalid_image("BMP width must be positive"))?;
            let height = signed_height
                .checked_abs()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| invalid_image("BMP height is invalid"))?;
            (
                width,
                height,
                read_u16_le(header, 26).unwrap_or(0),
                read_u16_le(header, 28).unwrap_or(0),
                read_u32_le(header, 30).unwrap_or(u32::MAX),
            )
        }
        _ => return Err(invalid_image("unsupported BMP DIB header")),
    };
    if planes != 1 || !valid_bmp_encoding(bits_per_pixel, compression) {
        return Err(invalid_image(
            "BMP planes, bit depth, or compression is invalid",
        ));
    }
    inspected_image("image/bmp", width, height)
}

fn valid_bmp_encoding(bits_per_pixel: u16, compression: u32) -> bool {
    match compression {
        0 => matches!(bits_per_pixel, 1 | 4 | 8 | 16 | 24 | 32),
        1 => bits_per_pixel == 8,
        2 => bits_per_pixel == 4,
        3 | 6 => matches!(bits_per_pixel, 16 | 32),
        4 | 5 => bits_per_pixel == 0,
        _ => false,
    }
}

fn inspect_webp_header(header: &[u8], file_length: u64) -> MediaResult<InspectedImage> {
    if file_length < 20 || header.len() < 20 {
        return Err(invalid_image("truncated WebP RIFF header"));
    }
    let riff_size = u64::from(read_u32_le(header, 4).unwrap_or(0));
    let riff_end = riff_size
        .checked_add(8)
        .ok_or_else(|| invalid_image("WebP RIFF length overflow"))?;
    if riff_end < 20 || riff_end > file_length {
        return Err(invalid_image("WebP RIFF payload is truncated"));
    }

    let mut offset = 12_usize;
    while (offset as u64) < riff_end {
        if offset + 8 > header.len() {
            return Err(invalid_image(
                "WebP dimensions exceed the bounded header inspection limit",
            ));
        }
        let fourcc = &header[offset..offset + 4];
        let chunk_size = u64::from(read_u32_le(header, offset + 4).unwrap_or(0));
        let payload_start = offset + 8;
        let payload_end = (payload_start as u64)
            .checked_add(chunk_size)
            .ok_or_else(|| invalid_image("WebP chunk length overflow"))?;
        let padded_end = payload_end
            .checked_add(chunk_size & 1)
            .ok_or_else(|| invalid_image("WebP padded chunk length overflow"))?;
        if padded_end > riff_end || padded_end > file_length {
            return Err(invalid_image("truncated WebP chunk"));
        }
        let payload_end = usize::try_from(payload_end)
            .map_err(|_| invalid_image("WebP chunk length is unsupported"))?;
        if (fourcc == b"VP8X" || fourcc == b"VP8 " || fourcc == b"VP8L")
            && payload_end > header.len()
        {
            return Err(invalid_image("truncated WebP image header chunk"));
        }

        if fourcc == b"VP8X" {
            if chunk_size != 10 {
                return Err(invalid_image("WebP VP8X chunk length is invalid"));
            }
            let payload = &header[payload_start..payload_end];
            if payload[0] & 0xc1 != 0 || payload[1..4] != [0, 0, 0] {
                return Err(invalid_image(
                    "WebP VP8X flags or reserved bytes are invalid",
                ));
            }
            let width = 1 + read_u24_le(payload, 4).unwrap_or(u32::MAX);
            let height = 1 + read_u24_le(payload, 7).unwrap_or(u32::MAX);
            return inspected_image("image/webp", width, height);
        }
        if fourcc == b"VP8 " {
            if chunk_size < 10 {
                return Err(invalid_image("truncated WebP VP8 key frame header"));
            }
            let payload = &header[payload_start..payload_end];
            let version = (payload[0] >> 1) & 0x07;
            if payload[0] & 1 != 0 || version > 3 || payload[3..6] != [0x9d, 0x01, 0x2a] {
                return Err(invalid_image("WebP VP8 key frame header is invalid"));
            }
            let width = u32::from(read_u16_le(payload, 6).unwrap_or(0) & 0x3fff);
            let height = u32::from(read_u16_le(payload, 8).unwrap_or(0) & 0x3fff);
            return inspected_image("image/webp", width, height);
        }
        if fourcc == b"VP8L" {
            if chunk_size < 5 {
                return Err(invalid_image("truncated WebP VP8L header"));
            }
            let payload = &header[payload_start..payload_end];
            let bits = read_u32_le(payload, 1).unwrap_or(u32::MAX);
            if payload[0] != 0x2f || bits >> 29 != 0 {
                return Err(invalid_image("WebP VP8L header is invalid"));
            }
            let width = 1 + (bits & 0x3fff);
            let height = 1 + ((bits >> 14) & 0x3fff);
            return inspected_image("image/webp", width, height);
        }
        offset = usize::try_from(padded_end)
            .map_err(|_| invalid_image("WebP chunk offset is unsupported"))?;
    }
    Err(invalid_image("WebP has no image dimensions"))
}

fn inspected_image(
    mime_type: &'static str,
    width: u32,
    height: u32,
) -> MediaResult<InspectedImage> {
    validate_dimensions(width, height)?;
    Ok(InspectedImage {
        mime_type,
        width,
        height,
    })
}

fn invalid_image(message: &str) -> DesktopNoteMediaError {
    DesktopNoteMediaError::InvalidImage(message.to_string())
}

fn read_u16_be(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn read_u16_le(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn read_u24_le(bytes: &[u8], offset: usize) -> Option<u32> {
    let value = bytes.get(offset..offset + 3)?;
    Some(u32::from(value[0]) | (u32::from(value[1]) << 8) | (u32::from(value[2]) << 16))
}

fn read_u32_be(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn read_i32_le(bytes: &[u8], offset: usize) -> Option<i32> {
    Some(i32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn png_crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn canonical_mime(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "image/png" => Some("image/png"),
        "image/jpeg" | "image/jpg" | "image/pjpeg" => Some("image/jpeg"),
        "image/webp" => Some("image/webp"),
        "image/gif" => Some("image/gif"),
        "image/bmp" | "image/x-bmp" | "image/x-ms-bmp" => Some("image/bmp"),
        _ => None,
    }
}

fn sanitize_display_name(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|character| !character.is_control())
        .take(255)
        .collect::<String>()
        .trim()
        .to_string()
}

fn generate_attachment_id() -> String {
    let mut random = [0_u8; 16];
    OsRng.fill_bytes(&mut random);
    let mut id = String::with_capacity("attachment-".len() + random.len() * 2);
    id.push_str("attachment-");
    for byte in random {
        let _ = write!(id, "{byte:02x}");
    }
    id
}

fn current_time_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(1)
        .max(1)
}

fn copy_source_to_synced_temp(source: &Path, target: &Path) -> MediaResult<(u64, String)> {
    let mut input = BufReader::new(File::open(source)?);
    let output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)?;
    let mut output = BufWriter::new(output);
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; COPY_BUFFER_BYTES];
    let mut total = 0_u64;
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(read as u64);
        if total > MAX_IMAGE_BYTES {
            return Err(DesktopNoteMediaError::ImageTooLarge {
                actual: total,
                maximum: MAX_IMAGE_BYTES,
            });
        }
        digest.update(&buffer[..read]);
        output.write_all(&buffer[..read])?;
    }
    if total == 0 {
        return Err(DesktopNoteMediaError::EmptyImage);
    }
    output.flush()?;
    output.get_ref().sync_all()?;
    drop(output);
    let sha256 = format!("{:x}", digest.finalize());
    Ok((total, sha256))
}

fn write_synced_new_file(path: &Path, content: &[u8]) -> MediaResult<()> {
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let mut writer = BufWriter::new(file);
    writer.write_all(content)?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    drop(writer);
    Ok(())
}

fn read_limited_file(path: &Path, maximum: u64) -> MediaResult<Vec<u8>> {
    let length = fs::metadata(path)?.len();
    if length == 0 || length > maximum {
        return Err(DesktopNoteMediaError::PendingImportCorrupt(
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("unknown")
                .to_string(),
        ));
    }
    let mut content = Vec::with_capacity(length as usize);
    File::open(path)?.read_to_end(&mut content)?;
    if content.len() as u64 != length {
        return Err(DesktopNoteMediaError::PendingImportCorrupt(
            path.display().to_string(),
        ));
    }
    Ok(content)
}

fn sha256_file(path: &Path) -> MediaResult<String> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; COPY_BUFFER_BYTES];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn sha256_bytes(content: &[u8]) -> String {
    format!("{:x}", Sha256::digest(content))
}

fn manifest_entry(attachment: &NoteAttachmentMetadata) -> NoteMediaManifestEntry {
    NoteMediaManifestEntry {
        attachment_id: attachment.id.clone(),
        sha256: attachment.sha256.clone(),
        mime_type: attachment.mime_type.clone(),
        size_bytes: attachment.size_bytes,
        updated_at_epoch_millis: attachment.updated_at_epoch_millis,
    }
}

fn same_blob_identity(left: &NoteAttachmentMetadata, right: &NoteAttachmentMetadata) -> bool {
    left.id == right.id
        && left.kind == right.kind
        && left.file_name == right.file_name
        && left.size_bytes == right.size_bytes
        && left.width == right.width
        && left.height == right.height
        && normalize_sha256(&left.sha256).ok() == normalize_sha256(&right.sha256).ok()
        && canonical_storage_mime(&left.mime_type) == canonical_storage_mime(&right.mime_type)
}

fn merge_metadata_revision(
    stored: &NoteAttachmentMetadata,
    incoming: &NoteAttachmentMetadata,
) -> NoteAttachmentMetadata {
    let mut merged = if incoming.updated_at_epoch_millis > stored.updated_at_epoch_millis {
        incoming.clone()
    } else {
        stored.clone()
    };
    merged.created_at_epoch_millis = safe_earliest_timestamp(
        stored.created_at_epoch_millis,
        incoming.created_at_epoch_millis,
    );
    merged.updated_at_epoch_millis = stored
        .updated_at_epoch_millis
        .max(incoming.updated_at_epoch_millis)
        .max(1);
    merged
}

fn safe_earliest_timestamp(left: i64, right: i64) -> i64 {
    match (left > 0, right > 0) {
        (true, true) => left.min(right),
        (true, false) => left,
        (false, true) => right,
        (false, false) => 1,
    }
}

#[cfg(windows)]
fn atomic_replace_existing(source: &Path, target: &Path) -> MediaResult<()> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    extern "system" {
        fn ReplaceFileW(
            replaced_file_name: *const u16,
            replacement_file_name: *const u16,
            backup_file_name: *const u16,
            replace_flags: u32,
            exclude: *mut std::ffi::c_void,
            reserved: *mut std::ffi::c_void,
        ) -> i32;
    }

    let target_wide = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let source_wide = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let replaced = unsafe {
        ReplaceFileW(
            target_wide.as_ptr(),
            source_wide.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if replaced == 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(not(windows))]
fn atomic_replace_existing(source: &Path, target: &Path) -> MediaResult<()> {
    fs::rename(source, target)?;
    Ok(())
}

fn ensure_absent_or_remove_regular(parent: &Path, path: &Path) -> MediaResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(DesktopNoteMediaError::UnsafeStorageLayout(
                    path.to_path_buf(),
                ));
            }
            require_regular_child(parent, path)?;
            fs::remove_file(path)?;
            sync_directory(parent)?;
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn safe_regular_child_exists(parent: &Path, path: &Path) -> MediaResult<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(DesktopNoteMediaError::UnsafeStorageLayout(
                    path.to_path_buf(),
                ));
            }
            require_regular_child(parent, path)?;
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn require_regular_child(parent: &Path, path: &Path) -> MediaResult<()> {
    if path.parent() != Some(parent) {
        return Err(DesktopNoteMediaError::UnsafeStorageLayout(
            path.to_path_buf(),
        ));
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink()
        || metadata_is_reparse_point(&metadata)
        || !metadata.is_file()
    {
        return Err(DesktopNoteMediaError::UnsafeStorageLayout(
            path.to_path_buf(),
        ));
    }
    let canonical = path.canonicalize()?;
    if canonical.parent() != Some(parent) {
        return Err(DesktopNoteMediaError::UnsafeStorageLayout(canonical));
    }
    Ok(())
}

#[cfg(windows)]
fn metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(windows)]
fn sync_directory(path: &Path) -> MediaResult<()> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    let directory = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?;
    directory.sync_all()?;
    Ok(())
}

#[cfg(not(windows))]
fn sync_directory(path: &Path) -> MediaResult<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
fn path_is_single_normal_component(path: &Path) -> bool {
    let mut components = path.components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let sequence = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "gridtimer-note-media-{label}-{}-{sequence}",
                std::process::id()
            ));
            if path.exists() {
                fs::remove_dir_all(&path).expect("remove stale test directory");
            }
            fs::create_dir_all(&path).expect("create test directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn one_pixel_bmp() -> Vec<u8> {
        bmp_with_dimensions(1, 1)
    }

    fn bmp_with_dimensions(width: u32, height: u32) -> Vec<u8> {
        let row_bytes = ((width * 3 + 3) / 4) * 4;
        let image_bytes = row_bytes * height;
        let file_bytes = 54_u32 + image_bytes;
        let mut bytes = Vec::with_capacity(file_bytes as usize);
        bytes.extend_from_slice(b"BM");
        bytes.extend_from_slice(&file_bytes.to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&54_u32.to_le_bytes());
        bytes.extend_from_slice(&40_u32.to_le_bytes());
        bytes.extend_from_slice(&(width as i32).to_le_bytes());
        bytes.extend_from_slice(&(height as i32).to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&24_u16.to_le_bytes());
        bytes.extend_from_slice(&[0; 24]);
        bytes.resize(file_bytes as usize, 0x7f);
        bytes
    }

    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(33);
        bytes.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        bytes.extend_from_slice(&13_u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes.extend_from_slice(&png_crc32(&bytes[12..29]).to_be_bytes());
        bytes
    }

    fn jpeg_header(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = vec![0xff, 0xd8, 0xff, 0xc0];
        bytes.extend_from_slice(&11_u16.to_be_bytes());
        bytes.push(8);
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&[1, 1, 0x11, 0]);
        bytes
    }

    fn gif_header(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = b"GIF89a".to_vec();
        bytes.extend_from_slice(&width.to_le_bytes());
        bytes.extend_from_slice(&height.to_le_bytes());
        bytes.extend_from_slice(&[0, 0, 0]);
        bytes
    }

    fn webp_vp8x_header(width: u32, height: u32) -> Vec<u8> {
        assert!((1..=0x01_00_00_00).contains(&width));
        assert!((1..=0x01_00_00_00).contains(&height));
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&22_u32.to_le_bytes());
        bytes.extend_from_slice(b"WEBPVP8X");
        bytes.extend_from_slice(&10_u32.to_le_bytes());
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
        bytes.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
        bytes
    }

    fn write_fixture(directory: &TestDirectory, name: &str, content: &[u8]) -> PathBuf {
        let path = directory.path().join(name);
        fs::write(&path, content).expect("write image fixture");
        path
    }

    fn directory_tree_snapshot(root: &Path) -> Vec<(String, Option<Vec<u8>>)> {
        fn visit(root: &Path, directory: &Path, snapshot: &mut Vec<(String, Option<Vec<u8>>)>) {
            let mut entries = fs::read_dir(directory)
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                let path = entry.path();
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                let metadata = fs::symlink_metadata(&path).unwrap();
                if metadata.is_dir() {
                    snapshot.push((format!("{relative}/"), None));
                    visit(root, &path, snapshot);
                } else {
                    snapshot.push((relative, Some(fs::read(&path).unwrap())));
                }
            }
        }

        let mut snapshot = Vec::new();
        visit(root, root, &mut snapshot);
        snapshot
    }

    #[test]
    fn attachment_ids_cannot_escape_the_workspace() {
        let directory = TestDirectory::new("traversal");
        let store = DesktopNoteMediaStore::new(directory.path().canonicalize().unwrap()).unwrap();
        for unsafe_id in [
            "../outside",
            "..",
            ".hidden",
            "folder/name",
            "folder\\name",
            "C:drive",
            "名字",
            "",
        ] {
            assert!(matches!(
                store.blob_path(unsafe_id),
                Err(DesktopNoteMediaError::InvalidAttachmentId(_))
            ));
        }
        assert!(path_is_single_normal_component(Path::new("attachment-123")));
        assert!(!path_is_single_normal_component(Path::new(
            "../attachment-123"
        )));
    }

    #[test]
    fn bounded_header_parser_recognizes_all_five_supported_formats() {
        let fixtures = [
            (png_header(3, 2), "image/png", 3, 2),
            (jpeg_header(4, 3), "image/jpeg", 4, 3),
            (webp_vp8x_header(5, 4), "image/webp", 5, 4),
            (gif_header(6, 5), "image/gif", 6, 5),
            (bmp_with_dimensions(7, 6), "image/bmp", 7, 6),
        ];
        for (bytes, expected_mime, expected_width, expected_height) in fixtures {
            let inspected = inspect_image_header(&bytes, bytes.len() as u64).unwrap();
            assert_eq!(expected_mime, inspected.mime_type);
            assert_eq!(
                (expected_width, expected_height),
                (inspected.width, inspected.height)
            );
        }
    }

    #[test]
    fn bounded_header_parser_rejects_truncated_and_invalid_headers() {
        let mut broken_png = png_header(3, 2);
        broken_png.truncate(20);

        let mut broken_jpeg = jpeg_header(4, 3);
        broken_jpeg.pop();

        let mut broken_webp = webp_vp8x_header(5, 4);
        broken_webp[4..8].copy_from_slice(&100_u32.to_le_bytes());

        let mut broken_gif = gif_header(6, 5);
        broken_gif[6..8].copy_from_slice(&0_u16.to_le_bytes());

        let mut broken_bmp = bmp_with_dimensions(7, 6);
        broken_bmp.truncate(30);

        for bytes in [broken_png, broken_jpeg, broken_webp, broken_gif, broken_bmp] {
            assert!(inspect_image_header(&bytes, bytes.len() as u64).is_err());
        }
    }

    #[test]
    fn import_computes_hash_dimensions_mime_and_app_data_json() {
        let directory = TestDirectory::new("hash");
        let source = write_fixture(&directory, "原图.bmp", &one_pixel_bmp());
        let media_root = directory.path().join("account-media");
        let store = DesktopNoteMediaStore::new(&media_root).unwrap();
        let pending = store.begin_import(&source).unwrap();
        let attachment = pending.attachment();
        assert_eq!("IMAGE", attachment.kind);
        assert_eq!("image/bmp", attachment.mime_type);
        assert_eq!((1, 1), (attachment.width, attachment.height));
        assert_eq!(one_pixel_bmp().len() as i64, attachment.size_bytes);
        assert_eq!(sha256_bytes(&one_pixel_bmp()), attachment.sha256);
        assert_eq!(format!("{}.blob", attachment.id), attachment.file_name);
        assert_eq!("原图.bmp", attachment.display_name);
        let json: serde_json::Value =
            serde_json::from_str(&pending.attachment_json().unwrap()).unwrap();
        assert_eq!(Some("IMAGE"), json["kind"].as_str());
        assert_eq!(Some("image/bmp"), json["mimeType"].as_str());
        assert!(json.get("sizeBytes").is_some());
    }

    #[test]
    fn import_rejects_byte_and_dimension_limits() {
        let directory = TestDirectory::new("limits");
        let oversized = directory.path().join("oversized.bin");
        let file = File::create(&oversized).unwrap();
        file.set_len(MAX_IMAGE_BYTES + 1).unwrap();
        file.sync_all().unwrap();
        let store = DesktopNoteMediaStore::new(directory.path().join("media")).unwrap();
        assert!(matches!(
            store.begin_import(&oversized),
            Err(DesktopNoteMediaError::ImageTooLarge { .. })
        ));

        let dimensions = bmp_with_dimensions(MAX_IMAGE_DIMENSION + 1, 1);
        let dimensions_path = write_fixture(&directory, "wide.bmp", &dimensions);
        assert!(matches!(
            store.begin_import(&dimensions_path),
            Err(DesktopNoteMediaError::ImageDimensionsTooLarge { .. })
        ));
        assert!(matches!(
            validate_dimensions(8_193, 8_193),
            Err(DesktopNoteMediaError::ImageDimensionsTooLarge { .. })
        ));
    }

    #[test]
    fn crash_recovery_publishes_a_valid_journaled_blob() {
        let directory = TestDirectory::new("recovery");
        let source = write_fixture(&directory, "recovery.bmp", &one_pixel_bmp());
        let store = DesktopNoteMediaStore::new(directory.path().join("media")).unwrap();
        let pending = store
            .begin_import_internal(
                &source,
                Some("recovery.bmp"),
                100,
                Some("attachment-recovery"),
            )
            .unwrap();
        assert!(!store.blob_exists(pending.attachment_id()).unwrap());

        let reopened = DesktopNoteMediaStore::new(store.root()).unwrap();
        let referenced = HashSet::from([pending.attachment_id().to_string()]);
        let report = reopened.recover_pending_referenced(&referenced).unwrap();
        assert_eq!(1, report.recovered_count);
        assert_eq!(0, report.malformed_journal_count);
        assert!(reopened.blob_exists(pending.attachment_id()).unwrap());
        assert_eq!(1, reopened.scan_manifest().unwrap().len());
        assert!(!reopened
            .journal_path_unchecked(pending.attachment_id())
            .exists());
    }

    #[test]
    fn crash_before_app_data_commit_never_publishes_an_unreferenced_blob() {
        let directory = TestDirectory::new("unreferenced-crash");
        let source = write_fixture(&directory, "unreferenced.bmp", &one_pixel_bmp());
        let store = DesktopNoteMediaStore::new(directory.path().join("media")).unwrap();
        let pending = store
            .begin_import_internal(
                &source,
                Some("unreferenced.bmp"),
                110,
                Some("attachment-unreferenced"),
            )
            .unwrap();

        let report = store.recover_pending_referenced(&HashSet::new()).unwrap();
        assert_eq!(1, report.unreferenced_pending_count);
        assert!(!store.blob_exists(pending.attachment_id()).unwrap());
        assert!(store
            .journal_path_unchecked(pending.attachment_id())
            .exists());

        let cleanup = store.cleanup_unreferenced_pending(&HashSet::new()).unwrap();
        assert_eq!(1, cleanup.cleaned_count);
        assert!(cleanup.is_clear());
        assert!(!store
            .journal_path_unchecked(pending.attachment_id())
            .exists());
        assert!(!store
            .pending_blob_path_unchecked(pending.attachment_id())
            .exists());
    }

    #[test]
    fn cleanup_failure_leaves_a_durable_encryption_blocker_until_retry_succeeds() {
        let directory = TestDirectory::new("cleanup-blocker");
        let source = write_fixture(&directory, "blocked.bmp", &one_pixel_bmp());
        let store = DesktopNoteMediaStore::new(directory.path().join("media")).unwrap();
        let pending = store
            .begin_import_internal(
                &source,
                Some("blocked.bmp"),
                120,
                Some("attachment-blocked"),
            )
            .unwrap();

        let injected = store.guarded_cleanup_attachment_with(
            pending.attachment_id(),
            "injected-failure",
            |_store, _attachment_id| {
                Err(DesktopNoteMediaError::Io(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected cleanup failure",
                )))
            },
        );
        assert!(injected.is_err());
        assert!(store
            .cleanup_block_marker_path_unchecked(pending.attachment_id())
            .exists());
        let blocked = store.encryption_blockers(&HashSet::new()).unwrap();
        assert!(blocked.cleanup_marker_count >= 1);
        assert!(!blocked.is_clear());

        let recovered = store
            .retry_cleanup_blocks(
                &crate::desktop_media_references::DesktopMediaReferenceScope::from_snapshot(
                    &crate::app_data::default_app_data_json(0),
                    &crate::desktop_state_store::DesktopPrivacyPolicy::default(),
                ),
            )
            .unwrap();
        assert_eq!(1, recovered.cleaned_count);
        assert!(recovered.is_clear());
        assert!(store
            .encryption_blockers(&HashSet::new())
            .unwrap()
            .is_clear());
    }

    #[test]
    fn unreferenced_committed_blob_blocks_encryption_until_it_is_authoritative() {
        let directory = TestDirectory::new("orphan-encryption-blocker");
        let store = DesktopNoteMediaStore::new(directory.path().join("media")).unwrap();
        let image = one_pixel_bmp();
        let attachment_id = "attachment-orphan";
        store
            .write_download_blob(
                attachment_id,
                &sha256_bytes(&image),
                image.len() as i64,
                "image/bmp",
                125,
                &image,
            )
            .unwrap();

        let blocked = store.encryption_blockers(&HashSet::new()).unwrap();
        assert_eq!(1, blocked.orphan_attachment_count);
        assert!(!blocked.is_clear());

        let referenced = HashSet::from([attachment_id.to_string()]);
        assert!(store.encryption_blockers(&referenced).unwrap().is_clear());
    }

    #[test]
    fn unreferenced_committed_cleanup_preserves_historical_references_and_is_idempotent() {
        let directory = TestDirectory::new("committed-gc");
        let store = DesktopNoteMediaStore::new(directory.path().join("media")).unwrap();
        let image = one_pixel_bmp();
        let sha = sha256_bytes(&image);
        for attachment_id in ["attachment-orphan", "attachment-history"] {
            store
                .write_download_blob(
                    attachment_id,
                    &sha,
                    image.len() as i64,
                    "image/bmp",
                    130,
                    &image,
                )
                .unwrap();
        }
        let historical_references = HashSet::from(["attachment-history".to_string()]);

        let first = store
            .cleanup_unreferenced_committed(&historical_references)
            .unwrap();

        assert_eq!(1, first.cleaned_count);
        assert!(first.is_clear());
        assert!(!store.blob_exists("attachment-orphan").unwrap());
        assert!(store.blob_exists("attachment-history").unwrap());
        assert!(store
            .encryption_blockers(&historical_references)
            .unwrap()
            .is_clear());

        let second = store
            .cleanup_unreferenced_committed(&historical_references)
            .unwrap();
        assert_eq!(0, second.cleaned_count);
        assert!(second.is_clear());
    }

    #[test]
    fn pre_journal_and_unknown_root_artifacts_fail_closed_for_encryption() {
        let directory = TestDirectory::new("pre-journal-encryption-blocker");
        let store = DesktopNoteMediaStore::new(directory.path().join("media")).unwrap();
        let attachment_id = "attachment-pre-journal";
        let staged_blob = store.pending_blob_path_unchecked(attachment_id);
        fs::write(&staged_blob, one_pixel_bmp()).unwrap();

        let blocked = store.encryption_blockers(&HashSet::new()).unwrap();
        assert_eq!(1, blocked.pending_artifact_count);
        assert!(!blocked.is_clear());
        let cleaned = store.cleanup_unreferenced_pending(&HashSet::new()).unwrap();
        assert_eq!(1, cleaned.cleaned_count);
        assert!(cleaned.is_clear());
        assert!(!staged_blob.exists());

        let metadata_temp = store.metadata_temp_path_unchecked(attachment_id);
        fs::write(&metadata_temp, b"partial-metadata").unwrap();
        let blocked = store.encryption_blockers(&HashSet::new()).unwrap();
        assert_eq!(1, blocked.pending_artifact_count);
        assert!(!blocked.is_clear());
        let cleaned = store.cleanup_unreferenced_pending(&HashSet::new()).unwrap();
        assert_eq!(1, cleaned.cleaned_count);
        assert!(!metadata_temp.exists());

        let journal_window_id = "attachment-journal-temp-window";
        let staged_blob = store.pending_blob_path_unchecked(journal_window_id);
        let journal_temp = store.journal_temp_path_unchecked(journal_window_id);
        fs::write(&staged_blob, one_pixel_bmp()).unwrap();
        fs::write(&journal_temp, b"partial-journal").unwrap();
        assert!(!store
            .encryption_blockers(&HashSet::new())
            .unwrap()
            .is_clear());
        let cleaned = store.cleanup_unreferenced_pending(&HashSet::new()).unwrap();
        assert_eq!(1, cleaned.cleaned_count);
        assert!(!staged_blob.exists());
        assert!(!journal_temp.exists());

        fs::write(store.root().join("unexpected-plaintext.bin"), b"plaintext").unwrap();
        assert!(matches!(
            store.encryption_blockers(&HashSet::new()),
            Err(DesktopNoteMediaError::UnsafeStorageLayout(_))
        ));
    }

    #[test]
    fn pre_journal_cleanup_failure_keeps_a_durable_retry_marker() {
        let directory = TestDirectory::new("pre-journal-cleanup-failure");
        let store = DesktopNoteMediaStore::new(directory.path().join("media")).unwrap();
        let attachment_id = "attachment-pre-journal-cleanup-failure";
        let staged_blob = store.pending_blob_path_unchecked(attachment_id);
        fs::write(&staged_blob, one_pixel_bmp()).unwrap();

        let failed = store.guarded_cleanup_attachment_with(
            attachment_id,
            "orphan-import-temporary",
            |_store, _attachment_id| {
                Err(DesktopNoteMediaError::Io(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected orphan cleanup failure",
                )))
            },
        );
        assert!(failed.is_err());
        assert!(store
            .cleanup_block_marker_path_unchecked(attachment_id)
            .exists());
        assert!(!store
            .encryption_blockers(&HashSet::new())
            .unwrap()
            .is_clear());

        let retried = store
            .retry_cleanup_blocks(
                &crate::desktop_media_references::DesktopMediaReferenceScope::from_snapshot(
                    &crate::app_data::default_app_data_json(0),
                    &crate::desktop_state_store::DesktopPrivacyPolicy::default(),
                ),
            )
            .unwrap();
        assert_eq!(1, retried.cleaned_count);
        assert!(retried.is_clear());
        assert!(!staged_blob.exists());
        assert!(store
            .encryption_blockers(&HashSet::new())
            .unwrap()
            .is_clear());
    }

    #[test]
    fn failed_commit_keeps_the_journal_for_reference_bound_recovery() {
        let directory = TestDirectory::new("commit-failure-journal");
        let source = write_fixture(&directory, "commit.bmp", &one_pixel_bmp());
        let store = DesktopNoteMediaStore::new(directory.path().join("media")).unwrap();
        let pending = store
            .begin_import_internal(
                &source,
                Some("commit.bmp"),
                130,
                Some("attachment-commit-failure"),
            )
            .unwrap();
        fs::remove_file(store.pending_blob_path_unchecked(pending.attachment_id())).unwrap();

        assert!(store.commit_import(&pending).is_err());
        assert!(store
            .journal_path_unchecked(pending.attachment_id())
            .exists());
        let referenced = HashSet::from([pending.attachment_id().to_string()]);
        assert!(!store.encryption_blockers(&referenced).unwrap().is_clear());
    }

    #[test]
    fn commit_is_atomic_and_download_refuses_to_overwrite_different_bytes() {
        let directory = TestDirectory::new("atomic");
        let image = one_pixel_bmp();
        let source = write_fixture(&directory, "atomic.bmp", &image);
        let store = DesktopNoteMediaStore::new(directory.path().join("media")).unwrap();
        let pending = store
            .begin_import_internal(&source, Some("atomic.bmp"), 200, Some("attachment-atomic"))
            .unwrap();
        let final_path = store.blob_path(pending.attachment_id()).unwrap();
        assert!(!final_path.exists());
        let attachment = store.commit_import(&pending).unwrap();
        assert_eq!(image, fs::read(&final_path).unwrap());
        assert!(!store
            .pending_blob_path_unchecked(pending.attachment_id())
            .exists());

        let different = bmp_with_dimensions(2, 1);
        let error = store
            .write_download_blob(
                pending.attachment_id(),
                &sha256_bytes(&different),
                different.len() as i64,
                "image/bmp",
                300,
                &different,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            DesktopNoteMediaError::ExistingBlobConflict(_)
        ));
        assert_eq!(image, fs::read(&final_path).unwrap());
        assert_eq!(attachment.sha256, sha256_file(&final_path).unwrap());
    }

    #[test]
    fn download_write_and_verified_read_round_trip() {
        let directory = TestDirectory::new("download");
        let store = DesktopNoteMediaStore::new(directory.path().join("media")).unwrap();
        let image = bmp_with_dimensions(2, 3);
        let sha256 = sha256_bytes(&image);
        let manifest = store
            .write_download_blob(
                "attachment-download",
                &sha256,
                image.len() as i64,
                "image/x-ms-bmp",
                1234,
                &image,
            )
            .unwrap();
        assert_eq!("attachment-download", manifest.attachment_id);
        assert_eq!("image/bmp", manifest.mime_type);
        assert_eq!(
            image,
            store
                .read_blob("attachment-download", &sha256, manifest.size_bytes)
                .unwrap()
        );
        assert_eq!(vec![manifest], store.scan_manifest().unwrap());
    }

    #[test]
    fn idempotent_download_repairs_final_blob_without_metadata_after_crash() {
        let directory = TestDirectory::new("download-final-before-metadata");
        let store = DesktopNoteMediaStore::new(directory.path().join("media")).unwrap();
        let image = one_pixel_bmp();
        let sha256 = sha256_bytes(&image);
        let attachment_id = "attachment-final-before-metadata";
        let final_path = store.blob_path(attachment_id).unwrap();
        write_synced_new_file(&final_path, &image).unwrap();
        assert!(matches!(
            store.manifest_entry_for(attachment_id),
            Err(DesktopNoteMediaError::PendingImportCorrupt(_))
        ));

        let repaired = store
            .write_download_blob(
                attachment_id,
                &sha256,
                image.len() as i64,
                "image/bmp",
                900,
                &image,
            )
            .unwrap();

        assert_eq!(attachment_id, repaired.attachment_id);
        assert_eq!(sha256, repaired.sha256);
        assert_eq!(
            image,
            store
                .read_blob(attachment_id, &repaired.sha256, repaired.size_bytes)
                .unwrap()
        );
        assert_eq!(
            Some(repaired),
            store.manifest_entry_for(attachment_id).unwrap()
        );
    }

    #[test]
    fn malformed_journal_does_not_block_other_recovery() {
        let directory = TestDirectory::new("malformed");
        let source = write_fixture(&directory, "valid.bmp", &one_pixel_bmp());
        let store = DesktopNoteMediaStore::new(directory.path().join("media")).unwrap();
        store
            .begin_import_internal(&source, Some("valid.bmp"), 500, Some("attachment-valid"))
            .unwrap();
        fs::write(store.pending_directory.join("malformed.json"), b"{not-json")
            .expect("write malformed evidence");
        let report = store.recover_pending().unwrap();
        assert_eq!(1, report.recovered_count);
        assert_eq!(1, report.malformed_journal_count);
        assert!(store.pending_directory.join("malformed.json").exists());
    }

    #[test]
    fn explicit_delete_does_not_prune_other_or_historical_blobs() {
        let directory = TestDirectory::new("delete");
        let store = DesktopNoteMediaStore::new(directory.path().join("media")).unwrap();
        let image = one_pixel_bmp();
        let sha = sha256_bytes(&image);
        for id in ["attachment-current", "attachment-history"] {
            store
                .write_download_blob(id, &sha, image.len() as i64, "image/bmp", 100, &image)
                .unwrap();
        }
        assert!(store.delete_blob("attachment-current").unwrap());
        assert!(!store.blob_exists("attachment-current").unwrap());
        assert!(store.blob_exists("attachment-history").unwrap());
    }

    #[test]
    fn bound_store_rejects_namespace_reuse_without_persisting_identity_text() {
        let directory = TestDirectory::new("identity");
        let root = directory.path().join("media");
        let first = DesktopNoteMediaStore::new_bound(&root, "account-namespace-alpha").unwrap();
        let marker_path = first.root().join(IDENTITY_MARKER_FILE);
        let original_marker = fs::read(&marker_path).unwrap();
        assert!(!String::from_utf8_lossy(&original_marker).contains("account-namespace-alpha"));
        DesktopNoteMediaStore::new_bound(&root, "account-namespace-alpha").unwrap();
        assert_eq!(original_marker, fs::read(&marker_path).unwrap());
        assert!(matches!(
            DesktopNoteMediaStore::new_bound(&root, "account-namespace-beta"),
            Err(DesktopNoteMediaError::WorkspaceIdentityMismatch)
        ));
        // The legacy constructor remains available, but independently opened
        // instances still share the same normalized-root operation lock.
        let legacy = DesktopNoteMediaStore::new(&root).unwrap();
        assert!(Arc::ptr_eq(&first.operation_lock, &legacy.operation_lock));
        let held = first.operation_lock.lock().unwrap();
        assert!(legacy.operation_lock.try_lock().is_err());
        drop(held);
    }

    #[test]
    fn read_only_bound_probe_never_creates_or_changes_workspace_files() {
        let directory = TestDirectory::new("read-only-probe");
        let missing_root = directory.path().join("missing-media");
        assert!(matches!(
            ReadOnlyDesktopNoteMediaStore::probe_existing_bound(
                &missing_root,
                "account-namespace-alpha"
            )
            .unwrap(),
            ExistingBoundNoteMediaProbe::Missing
        ));
        assert!(!missing_root.exists());

        let root = directory.path().join("media");
        let image = one_pixel_bmp();
        let sha256 = sha256_bytes(&image);
        let writable = DesktopNoteMediaStore::new_bound(&root, "account-namespace-alpha").unwrap();
        writable
            .write_download_blob(
                "attachment-read-only",
                &sha256,
                image.len() as i64,
                "image/bmp",
                100,
                &image,
            )
            .unwrap();
        drop(writable);
        let before = directory_tree_snapshot(&root);

        let ExistingBoundNoteMediaProbe::Present(read_only) =
            ReadOnlyDesktopNoteMediaStore::probe_existing_bound(&root, "account-namespace-alpha")
                .unwrap()
        else {
            panic!("existing media root was reported missing");
        };
        let entry = read_only
            .manifest_entry_for("attachment-read-only")
            .unwrap()
            .unwrap();
        assert_eq!(sha256, entry.sha256);
        assert_eq!(
            image,
            read_only
                .read_blob("attachment-read-only", &entry.sha256, entry.size_bytes,)
                .unwrap()
        );
        assert_eq!(before, directory_tree_snapshot(&root));

        assert!(matches!(
            ReadOnlyDesktopNoteMediaStore::probe_existing_bound(&root, "account-namespace-beta"),
            Err(DesktopNoteMediaError::WorkspaceIdentityMismatch)
        ));
        assert_eq!(before, directory_tree_snapshot(&root));
    }

    #[test]
    fn metadata_sidecar_advances_monotonically_for_the_same_blob() {
        let directory = TestDirectory::new("metadata-revision");
        let source = write_fixture(&directory, "revision.bmp", &one_pixel_bmp());
        let store = DesktopNoteMediaStore::new(directory.path().join("media")).unwrap();
        let pending = store
            .begin_import_internal(
                &source,
                Some("older display.bmp"),
                200,
                Some("attachment-revision"),
            )
            .unwrap();
        let original = store.commit_import(&pending).unwrap();

        let mut newer = original.clone();
        newer.display_name = "newer display.bmp".to_string();
        newer.created_at_epoch_millis = 150;
        newer.updated_at_epoch_millis = 300;
        let promoted = store.ensure_metadata_file(&newer).unwrap();
        assert_eq!("newer display.bmp", promoted.display_name);
        assert_eq!(150, promoted.created_at_epoch_millis);
        assert_eq!(300, promoted.updated_at_epoch_millis);

        let mut stale = original.clone();
        stale.display_name = "stale display.bmp".to_string();
        stale.created_at_epoch_millis = 100;
        stale.updated_at_epoch_millis = 250;
        let preserved = store.ensure_metadata_file(&stale).unwrap();
        assert_eq!("newer display.bmp", preserved.display_name);
        assert_eq!(100, preserved.created_at_epoch_millis);
        assert_eq!(300, preserved.updated_at_epoch_millis);
        assert!(!store.metadata_temp_path_unchecked(&original.id).exists());

        let mut conflict = newer;
        conflict.sha256 = "0".repeat(64);
        assert!(matches!(
            store.ensure_metadata_file(&conflict),
            Err(DesktopNoteMediaError::ExistingBlobConflict(_))
        ));
    }

    #[test]
    fn single_manifest_lookup_isolates_an_unrelated_corrupt_sidecar() {
        let directory = TestDirectory::new("manifest-isolation");
        let store = DesktopNoteMediaStore::new(directory.path().join("media")).unwrap();
        let image = one_pixel_bmp();
        let sha = sha256_bytes(&image);
        let expected = store
            .write_download_blob(
                "attachment-good",
                &sha,
                image.len() as i64,
                "image/bmp",
                100,
                &image,
            )
            .unwrap();
        store
            .write_download_blob(
                "attachment-bad",
                &sha,
                image.len() as i64,
                "image/bmp",
                100,
                &image,
            )
            .unwrap();
        fs::write(
            store.metadata_path_unchecked("attachment-bad"),
            b"{broken-json",
        )
        .unwrap();

        assert_eq!(
            Some(expected),
            store.manifest_entry_for("attachment-good").unwrap()
        );
        assert!(store
            .manifest_entry_for("attachment-missing")
            .unwrap()
            .is_none());
        assert!(store.manifest_entry_for("attachment-bad").is_err());
        assert!(store.scan_manifest().is_err());
    }
}

fn general_media_kind(mime: &str) -> Option<&'static str> {
    match mime {
        "audio/mpeg" | "audio/wav" | "audio/ogg" | "audio/flac" | "audio/mp4" => Some("AUDIO"),
        "video/mp4" | "video/webm" | "video/ogg" => Some("VIDEO"),
        "application/octet-stream"
        | "application/pdf"
        | "text/plain"
        | "text/csv"
        | "application/json"
        | "application/zip" => Some("FILE"),
        _ => None,
    }
}
fn general_media_mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "flac" => "audio/flac",
        "m4a" => "audio/mp4",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "ogv" => "video/ogg",
        "pdf" => "application/pdf",
        "txt" | "md" => "text/plain",
        "csv" => "text/csv",
        "json" => "application/json",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}
fn canonical_storage_mime(mime: &str) -> Option<&'static str> {
    canonical_mime(mime).or_else(|| match mime {
        "audio/mpeg" => Some("audio/mpeg"),
        "audio/wav" => Some("audio/wav"),
        "audio/ogg" => Some("audio/ogg"),
        "audio/flac" => Some("audio/flac"),
        "audio/mp4" => Some("audio/mp4"),
        "video/mp4" => Some("video/mp4"),
        "video/webm" => Some("video/webm"),
        "video/ogg" => Some("video/ogg"),
        "application/octet-stream" => Some("application/octet-stream"),
        "application/pdf" => Some("application/pdf"),
        "text/plain" => Some("text/plain"),
        "text/csv" => Some("text/csv"),
        "application/json" => Some("application/json"),
        "application/zip" => Some("application/zip"),
        _ => None,
    })
}
fn inspect_stored_media(path: &Path, mime: &str) -> MediaResult<InspectedImage> {
    if canonical_mime(mime).is_some() || mime.trim().is_empty() {
        return inspect_image(path);
    }
    if mime == "application/octet-stream" {
        if let Ok(image) = inspect_image(path) {
            return Ok(image);
        }
    }
    let mime = canonical_storage_mime(mime).ok_or(DesktopNoteMediaError::UnsupportedImageFormat)?;
    Ok(InspectedImage {
        mime_type: mime,
        width: 0,
        height: 0,
    })
}

impl DesktopNoteMediaStore {
    pub fn validate_import_blob(
        attachment: &NoteAttachmentMetadata,
        content: &[u8],
    ) -> MediaResult<()> {
        attachment.validate()?;
        if attachment.size_bytes != content.len() as i64 {
            return Err(DesktopNoteMediaError::SizeMismatch {
                expected: attachment.size_bytes as u64,
                actual: content.len() as u64,
            });
        }
        let actual = sha256_bytes(content);
        if actual != attachment.sha256 {
            return Err(DesktopNoteMediaError::Sha256Mismatch {
                expected: attachment.sha256.clone(),
                actual,
            });
        }
        if attachment.kind == "IMAGE" {
            let image = inspect_image_header(
                &content[..content.len().min(MAX_IMAGE_HEADER_BYTES)],
                content.len() as u64,
            )?;
            if canonical_mime(&attachment.mime_type) != Some(image.mime_type)
                || attachment.width != image.width as i32
                || attachment.height != image.height as i32
            {
                return Err(DesktopNoteMediaError::InvalidImage(
                    "portable image metadata mismatch".into(),
                ));
            }
        }
        Ok(())
    }
    /// Stage a verified portable attachment before its containing pages commit.
    pub fn begin_imported_blob(
        &self,
        attachment: &NoteAttachmentMetadata,
        content: &[u8],
    ) -> MediaResult<PendingMediaImport> {
        Self::validate_import_blob(attachment, content)?;
        let _guard = self.lock()?;
        if self.blob_path_unchecked(&attachment.id).exists()
            || self.pending_blob_path_unchecked(&attachment.id).exists()
        {
            return Err(DesktopNoteMediaError::ExistingBlobConflict(
                attachment.id.clone(),
            ));
        }
        let path = self.pending_blob_path_unchecked(&attachment.id);
        write_synced_new_file(&path, content)?;
        if let Err(error) = self
            .verify_blob(&path, attachment)
            .and_then(|_| sync_directory(&self.root))
            .and_then(|_| {
                self.write_journal(&PendingJournal {
                    state_version: JOURNAL_STATE_VERSION,
                    attachment: attachment.clone(),
                    journaled_at_epoch_millis: current_time_millis(),
                })
            })
        {
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        Ok(PendingMediaImport {
            store_root: self.root.clone(),
            attachment: attachment.clone(),
        })
    }
}
