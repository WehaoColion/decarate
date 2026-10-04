// Frozen pre-change shared-data decoder, used only by the compatibility gate.
// v2.22.54 - Skip non-timer documents during projection parsing while retaining unknown fields.
// v2.22.52 - Preserve structured knowledge pages through editing and persistence.
// v2.22.38 - Preserve Unicode and unrelated content during attachment deletion.
use crate::finance_profile::FinanceProfile;
use crate::note_documents::{
    build_note_document_text_digest, NoteBlockTextInput, NoteDocumentTextInput,
};
use crate::rich_text_security::sanitize_rich_text_html_for_storage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Reverse;
use std::collections::{hash_map::DefaultHasher, BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};

pub const APP_DATA_SCHEMA_VERSION: i32 = 15;
const DEFAULT_SLOT_COUNT: i32 = 14;
const NOTE_ENCRYPTION_FORMAT_VERSION: i32 = 1;
const NOTE_ENCRYPTION_CIPHER_SUITE: &str = "AES-256-GCM";
const NOTE_ENCRYPTION_KDF: &str = "Argon2id";
const MAX_SYNC_CONFLICT_HISTORY: usize = 256;
const DEFAULT_CATEGORIES: [(&str, &str, &str); 4] = [
    ("category-work", "工作", "red"),
    ("category-study", "学习", "blue"),
    ("category-sport", "运动", "green"),
    ("category-life", "生活", "amber"),
];
pub(crate) const TOMBSTONE_ENTITY_SESSION: &str = "session";
pub(crate) const TOMBSTONE_ENTITY_ARCHIVED_TASK: &str = "archivedTask";
pub(crate) const TOMBSTONE_ENTITY_NOTE: &str = "note";
pub(crate) const TOMBSTONE_ENTITY_NOTE_FOLDER: &str = "noteFolder";
pub(crate) const TOMBSTONE_ENTITY_NOTE_ATTACHMENT: &str = "noteAttachment";
pub(crate) const TOMBSTONE_ENTITY_NOTE_MEDIA: &str = "noteMedia";
pub(crate) const TOMBSTONE_ENTITY_FINANCE_DAY_LEDGER: &str = "financeDayLedger";
pub(crate) const TOMBSTONE_ENTITY_FINANCE_MONTH_SNAPSHOT: &str = "financeMonthSnapshot";
const MICRO_BREAK_FOCUS_MIN_MILLIS: i64 = 3 * 60 * 1_000;
const MICRO_BREAK_FOCUS_MAX_MILLIS: i64 = 5 * 60 * 1_000;
const MICRO_BREAK_FOCUS_STEP_MILLIS: i64 = 15_000;
const MICRO_BREAK_FOCUS_VARIANT_COUNT: i64 =
    ((MICRO_BREAK_FOCUS_MAX_MILLIS - MICRO_BREAK_FOCUS_MIN_MILLIS) / MICRO_BREAK_FOCUS_STEP_MILLIS)
        + 1;
const MICRO_BREAK_REST_MILLIS: i64 = 15_000;
pub const MAX_MICRO_BREAK_DETAIL_SESSIONS: usize = 64;
const MAX_EXACT_MICRO_BREAK_PHASE_TRANSITIONS: usize = 256;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum ThemeMode {
    System,
    Light,
    Dark,
}

impl Default for ThemeMode {
    fn default() -> Self {
        Self::System
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum MicroBreakPhase {
    Focus,
    Break,
}

impl Default for MicroBreakPhase {
    fn default() -> Self {
        Self::Focus
    }
}

/// Public, forward-compatible phase used by desktop timer projections.
///
/// `Unknown` prevents a newer persisted phase from being silently presented as
/// a focus period by an older Windows client. The original value remains
/// available in [`TimerView::source_micro_break_phase`].
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TimerViewPhase {
    Focus,
    Break,
    Unknown,
}

/// Read-only timer state projected to a specific wall-clock instant.
///
/// `accumulated_millis` includes the currently elapsed focus segment, while
/// `stored_accumulated_millis` is the value present in the persisted snapshot.
/// This distinction lets a UI render a live total without rewriting storage.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TimerView {
    pub id: i32,
    pub title: String,
    pub category_id: Option<String>,
    pub note: String,
    pub stored_accumulated_millis: i64,
    pub accumulated_millis: i64,
    pub running_since_epoch_millis: Option<i64>,
    pub active_segment_started_at_epoch_millis: Option<i64>,
    pub active_run_id: String,
    pub is_running: bool,
    pub micro_break_phase: TimerViewPhase,
    pub source_micro_break_phase: String,
    pub micro_break_cycle_index: i32,
    pub micro_break_phase_progress_millis: i64,
    pub micro_break_phase_target_millis: i64,
    pub micro_break_phase_remaining_millis: i64,
    pub catch_up_compacted: bool,
    #[serde(default)]
    pub source_extra_fields: serde_json::Map<String, serde_json::Value>,
}

/// Timer-only projection of an app-data document.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TimerProjection {
    pub source_schema_version: Option<serde_json::Value>,
    pub projected_at_epoch_millis: i64,
    pub slot_order: Vec<i32>,
    pub slots: Vec<TimerView>,
    #[serde(default)]
    pub source_extra_fields: serde_json::Map<String, serde_json::Value>,
}

/// Parsed timer input that can be cached by a desktop client and projected on
/// each timer tick without reparsing the complete app-data JSON document.
#[derive(Clone, Debug)]
pub struct TimerProjector {
    source_schema_version: Option<serde_json::Value>,
    slot_order: Vec<i32>,
    slots: Vec<TimerProjectionSlotSource>,
    source_extra_fields: serde_json::Map<String, serde_json::Value>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimerProjectionError {
    message: String,
}

impl TimerProjectionError {
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for TimerProjectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for TimerProjectionError {}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum NoteSortMode {
    UpdatedDesc,
    CreatedDesc,
    CreatedAsc,
    TitleAsc,
}

impl Default for NoteSortMode {
    fn default() -> Self {
        Self::UpdatedDesc
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum NoteAttachmentKind {
    Image,
    File,
    Audio,
    Video,
}

impl Default for NoteAttachmentKind {
    fn default() -> Self {
        Self::Image
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum NoteEntryKind {
    Sticky,
    Document,
}

impl Default for NoteEntryKind {
    fn default() -> Self {
        Self::Sticky
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum NoteBlockType {
    Text,
    Image,
    Contact,
    Call,
}

impl Default for NoteBlockType {
    fn default() -> Self {
        Self::Text
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum NoteCallDirection {
    Unknown,
    Ongoing,
    Incoming,
    Outgoing,
    Missed,
}

impl Default for NoteCallDirection {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Category {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default = "default_red")]
    accent_seed: String,
    #[serde(default)]
    updated_at_epoch_millis: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct TimerSlot {
    #[serde(default)]
    id: i32,
    #[serde(default)]
    title: String,
    #[serde(default)]
    category_id: Option<String>,
    #[serde(default)]
    note: String,
    #[serde(default)]
    accumulated_millis: i64,
    #[serde(default)]
    running_since_epoch_millis: Option<i64>,
    #[serde(default)]
    active_run_id: String,
    #[serde(default)]
    micro_break_phase: MicroBreakPhase,
    #[serde(default)]
    micro_break_cycle_index: i32,
    #[serde(default)]
    micro_break_phase_progress_millis: i64,
    #[serde(default)]
    updated_at: i64,
    #[serde(default)]
    title_updated_at_epoch_millis: i64,
    #[serde(default)]
    category_updated_at_epoch_millis: i64,
    #[serde(default)]
    note_updated_at_epoch_millis: i64,
    #[serde(default)]
    accumulated_updated_at_epoch_millis: i64,
    #[serde(default)]
    running_updated_at_epoch_millis: i64,
    #[serde(default)]
    micro_break_updated_at_epoch_millis: i64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TimerProjectionSource {
    #[serde(default)]
    schema_version: Option<serde_json::Value>,
    #[serde(default)]
    slots: Vec<TimerProjectionSlotSource>,
    #[serde(default)]
    slot_order: Vec<i32>,
    // Skip known non-timer values at the token stream, before flatten can allocate them.
    #[serde(default, rename = "categories")]
    _ignored_0: serde::de::IgnoredAny,
    #[serde(default, rename = "slotOrderUpdatedAtEpochMillis")]
    _ignored_1: serde::de::IgnoredAny,
    #[serde(default, rename = "sessions")]
    _ignored_2: serde::de::IgnoredAny,
    #[serde(default, rename = "archivedTasks")]
    _ignored_3: serde::de::IgnoredAny,
    #[serde(default, rename = "noteFolders")]
    _ignored_4: serde::de::IgnoredAny,
    #[serde(default, rename = "notes")]
    _ignored_5: serde::de::IgnoredAny,
    #[serde(default, rename = "notePreferences")]
    _ignored_6: serde::de::IgnoredAny,
    #[serde(default, rename = "notePreferencesUpdatedAtEpochMillis")]
    _ignored_7: serde::de::IgnoredAny,
    #[serde(default, rename = "financeProfile")]
    _ignored_8: serde::de::IgnoredAny,
    #[serde(default, rename = "financeProfileUpdatedAtEpochMillis")]
    _ignored_9: serde::de::IgnoredAny,
    #[serde(default, rename = "financeDayLedgerRevisions")]
    _ignored_10: serde::de::IgnoredAny,
    #[serde(default, rename = "financeMonthSnapshotRevisions")]
    _ignored_11: serde::de::IgnoredAny,
    #[serde(default, rename = "themeMode")]
    _ignored_12: serde::de::IgnoredAny,
    #[serde(default, rename = "oledThemeEnabled")]
    _ignored_13: serde::de::IgnoredAny,
    #[serde(default, rename = "themeModeUpdatedAtEpochMillis")]
    _ignored_14: serde::de::IgnoredAny,
    #[serde(default, rename = "tombstones")]
    _ignored_15: serde::de::IgnoredAny,
    #[serde(default, rename = "syncConflictHistory")]
    _ignored_16: serde::de::IgnoredAny,
    #[serde(flatten)]
    source_extra_fields: serde_json::Map<String, serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TimerProjectionSlotSource {
    #[serde(default)]
    id: i32,
    #[serde(default)]
    title: String,
    #[serde(default)]
    category_id: Option<String>,
    #[serde(default)]
    note: String,
    #[serde(default)]
    accumulated_millis: i64,
    #[serde(default)]
    running_since_epoch_millis: Option<i64>,
    #[serde(default)]
    active_run_id: String,
    #[serde(default)]
    micro_break_phase: Option<String>,
    #[serde(default)]
    micro_break_cycle_index: i32,
    #[serde(default)]
    micro_break_phase_progress_millis: i64,
    #[serde(flatten)]
    source_extra_fields: serde_json::Map<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct TimerSession {
    #[serde(default)]
    id: String,
    #[serde(default)]
    slot_id: i32,
    #[serde(default)]
    slot_title: String,
    #[serde(default)]
    category_id: Option<String>,
    #[serde(default)]
    started_at_epoch_millis: i64,
    #[serde(default)]
    ended_at_epoch_millis: i64,
    #[serde(default)]
    duration_millis: i64,
    #[serde(default)]
    updated_at_epoch_millis: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct ArchivedTask {
    #[serde(default)]
    id: String,
    #[serde(default)]
    original_slot_id: i32,
    #[serde(default)]
    title: String,
    #[serde(default)]
    category_id: Option<String>,
    #[serde(default)]
    note: String,
    #[serde(default)]
    accumulated_millis: i64,
    #[serde(default)]
    archived_at_epoch_millis: i64,
    #[serde(default)]
    updated_at_epoch_millis: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct NoteFolder {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    created_at_epoch_millis: i64,
    #[serde(default)]
    updated_at_epoch_millis: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct NoteAttachment {
    #[serde(default)]
    id: String,
    #[serde(default)]
    kind: NoteAttachmentKind,
    #[serde(default)]
    file_name: String,
    #[serde(default)]
    display_name: String,
    #[serde(default = "default_image_mime")]
    mime_type: String,
    #[serde(default)]
    width: i32,
    #[serde(default)]
    height: i32,
    #[serde(default)]
    size_bytes: i64,
    #[serde(default)]
    sha256: String,
    #[serde(default)]
    created_at_epoch_millis: i64,
    #[serde(default)]
    updated_at_epoch_millis: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct NoteContactPhone {
    #[serde(default)]
    label: String,
    #[serde(default)]
    number: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct NoteBlock {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    knowledge: Option<crate::knowledge::KnowledgeBlock>,
    #[serde(default)]
    id: String,
    #[serde(default, rename = "type")]
    block_type: NoteBlockType,
    #[serde(default)]
    text: String,
    #[serde(default)]
    attachment_id: Option<String>,
    #[serde(default)]
    caption: String,
    #[serde(default)]
    contact_name: String,
    #[serde(default)]
    contact_organization: String,
    #[serde(default)]
    contact_phones: Vec<NoteContactPhone>,
    #[serde(default)]
    call_phone_number: String,
    #[serde(default)]
    call_contact_name: String,
    #[serde(default)]
    call_direction: NoteCallDirection,
    #[serde(default)]
    call_occurred_at_epoch_millis: Option<i64>,
    #[serde(default)]
    call_duration_millis: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct NoteDocument {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    knowledge: Option<crate::knowledge::KnowledgePage>,
    #[serde(default)]
    markdown_enabled: bool,
    #[serde(default)]
    rich_text_enabled: bool,
    #[serde(default)]
    rich_text_plain_text: String,
    #[serde(default)]
    blocks: Vec<NoteBlock>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct NoteRevisionSnapshot {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    label: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    kind: NoteEntryKind,
    #[serde(default)]
    document: NoteDocument,
    #[serde(default = "default_amber")]
    accent_seed: String,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    folder_id: Option<String>,
    #[serde(default)]
    attachment_ids: Vec<String>,
    #[serde(default)]
    attachments: Vec<NoteAttachment>,
    #[serde(default)]
    captured_at_epoch_millis: i64,
    #[serde(default)]
    updated_at_epoch_millis: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct NoteVersionSnapshot {
    #[serde(default)]
    id: String,
    #[serde(default)]
    note_id: String,
    #[serde(default)]
    sequence: i64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    kind: NoteEntryKind,
    #[serde(default)]
    document: NoteDocument,
    #[serde(default = "default_amber")]
    accent_seed: String,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    folder_id: Option<String>,
    #[serde(default)]
    attachment_ids: Vec<String>,
    #[serde(default)]
    attachments: Vec<NoteAttachment>,
    #[serde(default)]
    created_at_epoch_millis: i64,
    #[serde(default)]
    updated_at_epoch_millis: i64,
    #[serde(default)]
    is_latest: bool,
    #[serde(default)]
    deleted_at_epoch_millis: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NoteEncryptionEnvelope {
    #[serde(default)]
    pub(crate) format_version: i32,
    #[serde(default)]
    pub(crate) key_id: String,
    #[serde(default)]
    pub(crate) protection_revision: i64,
    #[serde(default)]
    pub(crate) cipher_suite: String,
    #[serde(default)]
    pub(crate) kdf: String,
    #[serde(default, rename = "memoryKiB")]
    pub(crate) memory_kib: i32,
    #[serde(default)]
    pub(crate) iterations: i32,
    #[serde(default)]
    pub(crate) parallelism: i32,
    #[serde(default)]
    pub(crate) salt_base64: String,
    #[serde(default)]
    pub(crate) key_nonce_base64: String,
    #[serde(default)]
    pub(crate) wrapped_key_base64: String,
    #[serde(default)]
    pub(crate) content_nonce_base64: String,
    #[serde(default)]
    pub(crate) ciphertext_base64: String,
}

impl NoteEncryptionEnvelope {
    fn repair_legacy_omitted_metadata(&mut self) {
        // A pre-release Kotlin encoder omitted exactly these three default-valued
        // protocol identifiers, after which this bridge represented them as
        // 0/empty strings. Repair only that exact legacy triplet. Any partial or
        // explicitly unsupported metadata is preserved for the crypto layer to
        // reject rather than being silently coerced.
        if self.format_version == 0 && self.cipher_suite.is_empty() && self.kdf.is_empty() {
            self.format_version = NOTE_ENCRYPTION_FORMAT_VERSION;
            self.cipher_suite = NOTE_ENCRYPTION_CIPHER_SUITE.to_string();
            self.kdf = NOTE_ENCRYPTION_KDF.to_string();
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct NoteEntry {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    kind: NoteEntryKind,
    #[serde(default)]
    document: NoteDocument,
    #[serde(default = "default_amber")]
    accent_seed: String,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    folder_id: Option<String>,
    #[serde(default)]
    attachments: Vec<NoteAttachment>,
    #[serde(default)]
    revisions: Vec<NoteRevisionSnapshot>,
    #[serde(default)]
    versions: Vec<NoteVersionSnapshot>,
    #[serde(default)]
    latest_version_id: String,
    #[serde(default)]
    encryption: Option<NoteEncryptionEnvelope>,
    #[serde(default)]
    protection_state_revision: i64,
    #[serde(default)]
    created_at_epoch_millis: i64,
    #[serde(default)]
    updated_at_epoch_millis: i64,
    #[serde(default)]
    deleted_at_epoch_millis: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct NotePreferences {
    #[serde(default)]
    sort_mode: NoteSortMode,
    #[serde(default)]
    selected_folder_id: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DataTombstone {
    #[serde(default)]
    pub(crate) entity_type: String,
    #[serde(default)]
    pub(crate) entity_id: String,
    #[serde(default)]
    pub(crate) deleted_at_epoch_millis: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppData {
    #[serde(default)]
    schema_version: i32,
    #[serde(default)]
    categories: Vec<Category>,
    #[serde(default)]
    slots: Vec<TimerSlot>,
    #[serde(default)]
    slot_order: Vec<i32>,
    #[serde(default)]
    slot_order_updated_at_epoch_millis: i64,
    #[serde(default)]
    sessions: Vec<TimerSession>,
    #[serde(default)]
    archived_tasks: Vec<ArchivedTask>,
    #[serde(default)]
    note_folders: Vec<NoteFolder>,
    #[serde(default)]
    notes: Vec<NoteEntry>,
    #[serde(default)]
    note_preferences: NotePreferences,
    #[serde(default)]
    note_preferences_updated_at_epoch_millis: i64,
    #[serde(default)]
    finance_profile: FinanceProfile,
    #[serde(default)]
    finance_profile_updated_at_epoch_millis: i64,
    #[serde(default)]
    finance_day_ledger_revisions: BTreeMap<String, i64>,
    #[serde(default)]
    finance_month_snapshot_revisions: BTreeMap<String, i64>,
    #[serde(default)]
    theme_mode: ThemeMode,
    #[serde(default)]
    oled_theme_enabled: bool,
    #[serde(default)]
    theme_mode_updated_at_epoch_millis: i64,
    #[serde(default)]
    tombstones: Vec<DataTombstone>,
    #[serde(default)]
    sync_conflict_history: Vec<SyncConflictRecord>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyncConflictRecord {
    #[serde(default)]
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) entity_type: String,
    #[serde(default)]
    pub(crate) entity_id: String,
    #[serde(default)]
    pub(crate) losing_revision_epoch_millis: i64,
    #[serde(default)]
    pub(crate) captured_at_epoch_millis: i64,
    #[serde(default)]
    pub(crate) payload: serde_json::Value,
}

pub fn default_app_data_json(_now: i64) -> String {
    let categories = default_categories();
    let slots = (1..=DEFAULT_SLOT_COUNT)
        .map(|slot_id| TimerSlot {
            id: slot_id,
            // A synthesized empty slot must not look like a user-authored clear.
            // Intentional clears carry their real mutation timestamp and can then
            // win a cross-device merge without stale content being resurrected.
            updated_at: 0,
            ..TimerSlot::default()
        })
        .collect::<Vec<_>>();
    let data = AppData {
        schema_version: APP_DATA_SCHEMA_VERSION,
        categories,
        slots,
        slot_order: (1..=DEFAULT_SLOT_COUNT).collect(),
        slot_order_updated_at_epoch_millis: 0,
        sessions: Vec::new(),
        archived_tasks: Vec::new(),
        note_folders: Vec::new(),
        notes: Vec::new(),
        note_preferences: NotePreferences::default(),
        note_preferences_updated_at_epoch_millis: 0,
        finance_profile: FinanceProfile::default(),
        finance_profile_updated_at_epoch_millis: 0,
        finance_day_ledger_revisions: BTreeMap::new(),
        finance_month_snapshot_revisions: BTreeMap::new(),
        theme_mode: ThemeMode::System,
        oled_theme_enabled: false,
        theme_mode_updated_at_epoch_millis: 0,
        tombstones: Vec::new(),
        sync_conflict_history: Vec::new(),
    };
    serde_json::to_string(&data).unwrap_or_else(|_| "{}".to_string())
}

fn default_categories() -> Vec<Category> {
    DEFAULT_CATEGORIES
        .iter()
        .copied()
        .map(|(id, name, accent_seed)| Category {
            id: id.to_string(),
            name: name.to_string(),
            accent_seed: accent_seed.to_string(),
            updated_at_epoch_millis: 0,
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppDataJsonCompatibility {
    LegacyMigratable,
    CurrentKnown,
    CurrentUnknown,
    Future,
    Invalid,
}

pub fn app_data_json_compatibility(raw: &str, now: i64) -> AppDataJsonCompatibility {
    classify_and_sanitize_app_data_json(raw, now).0
}

/// Projects every timer slot without rewriting or sanitizing the source JSON.
///
/// The parser intentionally ignores unrelated app-data sections and retains
/// future top-level and slot-level fields in `source_extra_fields`. A phase
/// unknown to this build is exposed as `TimerViewPhase::Unknown` instead of
/// being guessed.
pub fn project_timer_views(
    raw: &str,
    now_epoch_millis: i64,
) -> Result<TimerProjection, TimerProjectionError> {
    Ok(TimerProjector::parse(raw)?.project(now_epoch_millis))
}

pub fn project_timer_views_json(
    raw: &str,
    now_epoch_millis: i64,
) -> Result<String, TimerProjectionError> {
    let projection = project_timer_views(raw, now_epoch_millis)?;
    serde_json::to_string(&projection)
        .map_err(|error| TimerProjectionError::invalid(format!("timer projection failed: {error}")))
}

impl TimerProjector {
    pub fn parse(raw: &str) -> Result<Self, TimerProjectionError> {
        let source = serde_json::from_str::<TimerProjectionSource>(raw).map_err(|error| {
            TimerProjectionError::invalid(format!("invalid app-data JSON: {error}"))
        })?;
        Ok(Self {
            source_schema_version: source.schema_version,
            slot_order: source.slot_order,
            slots: source.slots,
            source_extra_fields: source.source_extra_fields,
        })
    }

    pub fn project(&self, now_epoch_millis: i64) -> TimerProjection {
        let projected_at_epoch_millis = now_epoch_millis.max(0);
        TimerProjection {
            source_schema_version: self.source_schema_version.clone(),
            projected_at_epoch_millis,
            slot_order: self.slot_order.clone(),
            slots: self
                .slots
                .iter()
                .cloned()
                .map(|slot| project_timer_slot(slot, projected_at_epoch_millis))
                .collect(),
            source_extra_fields: self.source_extra_fields.clone(),
        }
    }
}

fn project_timer_slot(mut source: TimerProjectionSlotSource, now_epoch_millis: i64) -> TimerView {
    for known_field in [
        "updatedAt",
        "titleUpdatedAtEpochMillis",
        "categoryUpdatedAtEpochMillis",
        "noteUpdatedAtEpochMillis",
        "accumulatedUpdatedAtEpochMillis",
        "runningUpdatedAtEpochMillis",
        "microBreakUpdatedAtEpochMillis",
    ] {
        source.source_extra_fields.remove(known_field);
    }
    let source_phase = source
        .micro_break_phase
        .unwrap_or_else(|| "FOCUS".to_string());
    let parsed_phase = match source_phase.as_str() {
        "FOCUS" => Some(MicroBreakPhase::Focus),
        "BREAK" => Some(MicroBreakPhase::Break),
        _ => None,
    };
    let stored_accumulated_millis = sanitize_tracked_duration(source.accumulated_millis);
    let running_since_epoch_millis = source
        .running_since_epoch_millis
        .map(|running_since| running_since.max(0));
    let cycle_index = source.micro_break_cycle_index.max(0);
    let mut active_run_id = source.active_run_id;
    if let Some(running_since) = running_since_epoch_millis {
        if active_run_id.is_empty() {
            active_run_id = deterministic_timer_run_id(source.id, running_since);
        }
    } else {
        active_run_id.clear();
    }

    let Some(phase) = parsed_phase else {
        return TimerView {
            id: source.id,
            title: source.title,
            category_id: source.category_id,
            note: source.note,
            stored_accumulated_millis,
            accumulated_millis: stored_accumulated_millis,
            running_since_epoch_millis,
            active_segment_started_at_epoch_millis: running_since_epoch_millis,
            active_run_id,
            is_running: running_since_epoch_millis.is_some(),
            micro_break_phase: TimerViewPhase::Unknown,
            source_micro_break_phase: source_phase,
            micro_break_cycle_index: cycle_index,
            micro_break_phase_progress_millis: source.micro_break_phase_progress_millis.max(0),
            micro_break_phase_target_millis: 0,
            micro_break_phase_remaining_millis: 0,
            catch_up_compacted: false,
            source_extra_fields: source.source_extra_fields,
        };
    };

    let phase_target = micro_break_phase_target_millis(source.id, &phase, cycle_index);
    let phase_progress = source
        .micro_break_phase_progress_millis
        .clamp(0, phase_target);
    let slot = TimerSlot {
        id: source.id,
        title: source.title,
        category_id: source.category_id,
        note: source.note,
        accumulated_millis: stored_accumulated_millis,
        running_since_epoch_millis,
        active_run_id,
        micro_break_phase: phase,
        micro_break_cycle_index: cycle_index,
        micro_break_phase_progress_millis: phase_progress,
        ..TimerSlot::default()
    };

    let (
        projected_phase,
        projected_cycle,
        projected_progress,
        projected_target,
        projected_total,
        active_segment_started_at_epoch_millis,
        catch_up_compacted,
    ) = if let Some(running_since) = running_since_epoch_millis {
        let advanced = advance_micro_break_state(&slot, running_since, now_epoch_millis, false);
        let target =
            micro_break_phase_target_millis(slot.id, &advanced.phase, advanced.cycle_index);
        let implicit_elapsed = advanced
            .safe_now
            .saturating_sub(advanced.active_segment_start)
            .max(0);
        let progress = advanced
            .phase_progress
            .saturating_add(implicit_elapsed)
            .clamp(0, target);
        let total = if advanced.phase == MicroBreakPhase::Focus {
            advanced.accumulated_millis.saturating_add(implicit_elapsed)
        } else {
            advanced.accumulated_millis
        };
        (
            advanced.phase,
            advanced.cycle_index,
            progress,
            target,
            sanitize_tracked_duration(total),
            Some(advanced.active_segment_start),
            advanced.catch_up_compacted,
        )
    } else {
        (
            phase,
            cycle_index,
            phase_progress,
            phase_target,
            stored_accumulated_millis,
            None,
            false,
        )
    };

    TimerView {
        id: slot.id,
        title: slot.title,
        category_id: slot.category_id,
        note: slot.note,
        stored_accumulated_millis,
        accumulated_millis: projected_total,
        running_since_epoch_millis,
        active_segment_started_at_epoch_millis,
        active_run_id: slot.active_run_id,
        is_running: running_since_epoch_millis.is_some(),
        micro_break_phase: match projected_phase {
            MicroBreakPhase::Focus => TimerViewPhase::Focus,
            MicroBreakPhase::Break => TimerViewPhase::Break,
        },
        source_micro_break_phase: source_phase,
        micro_break_cycle_index: projected_cycle,
        micro_break_phase_progress_millis: projected_progress,
        micro_break_phase_target_millis: projected_target,
        micro_break_phase_remaining_millis: projected_target.saturating_sub(projected_progress),
        catch_up_compacted,
        source_extra_fields: source.source_extra_fields,
    }
}

pub fn sanitize_app_data_json(raw: &str, now: i64) -> Option<String> {
    let (compatibility, sanitized) = classify_and_sanitize_app_data_json(raw, now);
    matches!(
        compatibility,
        AppDataJsonCompatibility::LegacyMigratable | AppDataJsonCompatibility::CurrentKnown
    )
    .then_some(sanitized)
    .flatten()
}

fn classify_and_sanitize_app_data_json(
    raw: &str,
    now: i64,
) -> (AppDataJsonCompatibility, Option<String>) {
    let Ok(input @ serde_json::Value::Object(_)) = serde_json::from_str::<serde_json::Value>(raw)
    else {
        return (AppDataJsonCompatibility::Invalid, None);
    };
    let Some(root) = input.as_object() else {
        return (AppDataJsonCompatibility::Invalid, None);
    };
    let schema_version = match root.get("schemaVersion") {
        None => None,
        Some(serde_json::Value::Number(number)) => number
            .as_i64()
            .map(i128::from)
            .or_else(|| number.as_u64().map(i128::from)),
        Some(_) => return (AppDataJsonCompatibility::Invalid, None),
    };
    let Some(schema_version) = schema_version.or(Some(0)) else {
        return (AppDataJsonCompatibility::Invalid, None);
    };
    if schema_version > i128::from(APP_DATA_SCHEMA_VERSION) {
        return (AppDataJsonCompatibility::Future, None);
    }

    let mut ignored_paths = Vec::<String>::new();
    let mut deserializer = serde_json::Deserializer::from_str(raw);
    let parsed: Result<AppData, _> = serde_ignored::deserialize(&mut deserializer, |path| {
        ignored_paths.push(path.to_string());
    });
    let Some(mut data) = parsed.ok() else {
        return if schema_version == i128::from(APP_DATA_SCHEMA_VERSION) {
            (AppDataJsonCompatibility::CurrentUnknown, None)
        } else {
            (AppDataJsonCompatibility::Invalid, None)
        };
    };
    if schema_version == i128::from(APP_DATA_SCHEMA_VERSION) && !ignored_paths.is_empty() {
        return (AppDataJsonCompatibility::CurrentUnknown, None);
    }
    repair_legacy_note_revision_kinds(&mut data, raw);
    let Some(sanitized) = data
        .sanitized(now)
        .and_then(|data| serde_json::to_string(&data).ok())
    else {
        return if schema_version == i128::from(APP_DATA_SCHEMA_VERSION) {
            (AppDataJsonCompatibility::CurrentUnknown, None)
        } else {
            (AppDataJsonCompatibility::Invalid, None)
        };
    };
    if schema_version == i128::from(APP_DATA_SCHEMA_VERSION) {
        (AppDataJsonCompatibility::CurrentKnown, Some(sanitized))
    } else {
        (AppDataJsonCompatibility::LegacyMigratable, Some(sanitized))
    }
}

fn repair_legacy_note_revision_kinds(data: &mut AppData, raw: &str) {
    let Some(raw_notes) = serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|root| {
            root.get("notes")
                .and_then(serde_json::Value::as_array)
                .cloned()
        })
    else {
        return;
    };
    for note in &mut data.notes {
        let Some(raw_note) = raw_notes.iter().find(|raw_note| {
            raw_note.get("id").and_then(serde_json::Value::as_str) == Some(note.id.as_str())
        }) else {
            continue;
        };
        let Some(raw_revisions) = raw_note
            .get("revisions")
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for (index, revision) in note.revisions.iter_mut().enumerate() {
            let raw_revision = raw_revisions
                .iter()
                .find(|raw_revision| {
                    !revision.id.is_empty()
                        && raw_revision.get("id").and_then(serde_json::Value::as_str)
                            == Some(revision.id.as_str())
                })
                .or_else(|| raw_revisions.get(index));
            let kind_was_explicit = raw_revision
                .and_then(|raw_revision| raw_revision.get("kind"))
                .and_then(serde_json::Value::as_str)
                .is_some_and(|kind| !kind.trim().is_empty());
            if !kind_was_explicit {
                revision.kind = note.kind.clone();
            }
        }
    }
}

fn parse_app_data_for_mutation(raw: &str, now: i64) -> Option<AppData> {
    match app_data_json_compatibility(raw, now) {
        AppDataJsonCompatibility::LegacyMigratable | AppDataJsonCompatibility::CurrentKnown => {
            let mut data = serde_json::from_str::<AppData>(raw).ok()?;
            repair_legacy_note_revision_kinds(&mut data, raw);
            data.sanitized(now)
        }
        AppDataJsonCompatibility::CurrentUnknown
        | AppDataJsonCompatibility::Future
        | AppDataJsonCompatibility::Invalid => None,
    }
}

pub fn unsupported_future_app_data_schema(raw: &str) -> Option<i32> {
    let value = serde_json::from_str::<serde_json::Value>(raw).ok()?;
    let schema_version = value.as_object()?.get("schemaVersion")?;
    if let Some(schema_version) = schema_version.as_i64() {
        return (schema_version > i64::from(APP_DATA_SCHEMA_VERSION))
            .then(|| i32::try_from(schema_version).unwrap_or(i32::MAX));
    }
    let schema_version = schema_version.as_u64()?;
    (schema_version > APP_DATA_SCHEMA_VERSION as u64).then_some(i32::MAX)
}

pub fn delete_session_app_data_json(raw: &str, session_id: &str, now: i64) -> Option<String> {
    let mut data = parse_app_data_for_mutation(raw, now)?;
    let record_revision = data
        .sessions
        .iter()
        .filter(|session| session.id == session_id)
        .map(session_revision_epoch_millis)
        .max()
        .unwrap_or(0);
    let deletion_revision = next_deletion_revision(
        &data.tombstones,
        TOMBSTONE_ENTITY_SESSION,
        session_id,
        record_revision,
        now,
    )?;
    data.sessions.retain(|session| session.id != session_id);
    upsert_data_tombstone(
        &mut data.tombstones,
        TOMBSTONE_ENTITY_SESSION,
        session_id,
        deletion_revision,
    );
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn delete_archived_task_app_data_json(
    raw: &str,
    archived_task_id: &str,
    now: i64,
) -> Option<String> {
    let mut data = parse_app_data_for_mutation(raw, now)?;
    let record_revision = data
        .archived_tasks
        .iter()
        .filter(|task| task.id == archived_task_id)
        .map(|task| task.updated_at_epoch_millis)
        .max()
        .unwrap_or(0);
    let deletion_revision = next_deletion_revision(
        &data.tombstones,
        TOMBSTONE_ENTITY_ARCHIVED_TASK,
        archived_task_id,
        record_revision,
        now,
    )?;
    data.archived_tasks
        .retain(|archived_task| archived_task.id != archived_task_id);
    upsert_data_tombstone(
        &mut data.tombstones,
        TOMBSTONE_ENTITY_ARCHIVED_TASK,
        archived_task_id,
        deletion_revision,
    );
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NoteUpsertRejectionReason {
    ProtectionStateConflict,
    InvalidMutation,
}

impl NoteUpsertRejectionReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProtectionStateConflict => "protection_state_conflict",
            Self::InvalidMutation => "invalid_mutation",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NoteUpsertResult {
    Applied(String),
    Rejected(NoteUpsertRejectionReason),
}

pub fn upsert_note_app_data_json_typed(raw: &str, note_json: &str, now: i64) -> NoteUpsertResult {
    let Some(mut data) = parse_app_data_for_mutation(raw, now) else {
        return NoteUpsertResult::Rejected(NoteUpsertRejectionReason::InvalidMutation);
    };
    let Ok(note) = serde_json::from_str::<NoteEntry>(note_json) else {
        return NoteUpsertResult::Rejected(NoteUpsertRejectionReason::InvalidMutation);
    };
    if note.id.is_empty() {
        return NoteUpsertResult::Rejected(NoteUpsertRejectionReason::InvalidMutation);
    }
    let existing = data
        .notes
        .iter()
        .find(|existing| existing.id == note.id)
        .cloned();
    if validate_note_protection_state_transition(existing.as_ref(), &note).is_none() {
        return NoteUpsertResult::Rejected(NoteUpsertRejectionReason::ProtectionStateConflict);
    }
    let Some(normalized) = normalize_note_for_save(&data, note, existing.as_ref(), now) else {
        return NoteUpsertResult::Rejected(NoteUpsertRejectionReason::InvalidMutation);
    };
    data.notes.retain(|note| note.id != normalized.id);
    data.notes.insert(0, normalized);
    let Some(encoded) = data
        .sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
    else {
        return NoteUpsertResult::Rejected(NoteUpsertRejectionReason::InvalidMutation);
    };
    NoteUpsertResult::Applied(encoded)
}

pub fn upsert_note_app_data_json(raw: &str, note_json: &str, now: i64) -> Option<String> {
    match upsert_note_app_data_json_typed(raw, note_json, now) {
        NoteUpsertResult::Applied(encoded) => Some(encoded),
        NoteUpsertResult::Rejected(_) => None,
    }
}

/// Saves the caller's current draft and appends one explicit, full note version.
///
/// `expected_latest_version_id` is a compare-and-swap guard. Two taps that both
/// started from the same visible latest version cannot both append. `request_id`
/// is an idempotency key: retrying an already completed request returns the
/// existing state instead of creating another version. An empty
/// `source_version_id` means "copy the latest version"; passing a historical
/// version id implements "create a new version from this version" without ever
/// mutating the historical snapshot.
pub fn create_note_version_app_data_json(
    raw: &str,
    note_json: &str,
    source_version_id: &str,
    expected_latest_version_id: &str,
    request_id: &str,
    now: i64,
) -> Option<String> {
    let expected_latest_version_id = expected_latest_version_id.trim();
    let request_id = request_id.trim();
    if expected_latest_version_id.is_empty() || request_id.is_empty() || request_id.len() > 1024 {
        return None;
    }

    let mut data = parse_app_data_for_mutation(raw, now)?;
    let incoming = serde_json::from_str::<NoteEntry>(note_json).ok()?;
    if incoming.id.is_empty() {
        return None;
    }
    let note_index = data.notes.iter().position(|note| note.id == incoming.id)?;
    if data.notes[note_index].deleted_at_epoch_millis.is_some()
        || data.notes[note_index].encryption.is_some()
        || incoming.encryption.is_some()
    {
        // Encrypted notes must be sealed first. The sealed outer record contains
        // no plaintext version payload, so mutating it here would lose content.
        return None;
    }

    let requested_version_id = requested_note_version_id(&incoming.id, request_id);
    if data.notes[note_index]
        .versions
        .iter()
        .any(|version| version.id == requested_version_id)
    {
        return data
            .sanitized(now)
            .and_then(|sanitized| serde_json::to_string(&sanitized).ok());
    }
    if data.notes[note_index].latest_version_id != expected_latest_version_id {
        return None;
    }

    let existing = data.notes[note_index].clone();
    let mut normalized = normalize_note_for_save(&data, incoming, Some(&existing), now)?;
    normalized.repair_version_stack(now);
    if normalized.latest_version_id != expected_latest_version_id {
        return None;
    }
    normalized = append_explicit_note_version(
        &data,
        normalized,
        source_version_id,
        expected_latest_version_id,
        request_id,
        now,
    )?;

    data.notes.remove(note_index);
    data.notes.insert(0, normalized);
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

/// Creates a version inside an authenticated, unlocked encrypted-note value.
///
/// The returned note is still plaintext and must immediately be passed to the
/// encrypted-note sealing API before it is persisted. Keeping this step in the
/// Rust data layer gives encrypted and unencrypted notes identical CAS,
/// idempotency, numbering, snapshot, and attachment-tombstone behavior.
pub fn create_unlocked_note_version_json(
    raw: &str,
    note_json: &str,
    source_version_id: &str,
    expected_latest_version_id: &str,
    request_id: &str,
    now: i64,
) -> Option<String> {
    let expected_latest_version_id = expected_latest_version_id.trim();
    let request_id = request_id.trim();
    if expected_latest_version_id.is_empty() || request_id.is_empty() || request_id.len() > 1024 {
        return None;
    }

    let data = parse_app_data_for_mutation(raw, now)?;
    let mut note = serde_json::from_str::<NoteEntry>(note_json).ok()?;
    if note.id.is_empty() || note.deleted_at_epoch_millis.is_some() {
        return None;
    }
    let stored = data.notes.iter().find(|stored| stored.id == note.id)?;
    if stored.deleted_at_epoch_millis.is_some() {
        return None;
    }
    let stored_state_revision = effective_note_protection_state_revision(stored)?;
    let note_state_revision = effective_note_protection_state_revision(&note)?;
    if note_state_revision != stored_state_revision {
        return None;
    }
    note.protection_state_revision = note_state_revision;
    let stored_encryption = stored.encryption.as_ref()?;
    let encryption = note.encryption.take()?;
    if stored_encryption.key_id != encryption.key_id
        || stored_encryption.protection_revision != encryption.protection_revision
    {
        return None;
    }

    let requested_version_id = requested_note_version_id(&note.id, request_id);
    if note
        .versions
        .iter()
        .any(|version| version.id == requested_version_id)
    {
        note.encryption = Some(encryption);
        return serde_json::to_string(&note).ok();
    }
    if note.latest_version_id != expected_latest_version_id {
        return None;
    }

    note.created_at_epoch_millis = stored.created_at_epoch_millis;
    note.deleted_at_epoch_millis = None;
    let valid_folder_ids = data
        .note_folders
        .iter()
        .map(|folder| folder.id.clone())
        .collect::<HashSet<_>>();
    note = note.sanitized(&valid_folder_ids, now)?;
    note.repair_version_stack(now);
    note = append_explicit_note_version(
        &data,
        note,
        source_version_id,
        expected_latest_version_id,
        request_id,
        now,
    )?;
    note.encryption = Some(encryption);
    serde_json::to_string(&note).ok()
}

pub fn history_deletion_summary_values(
    raw: &str,
    session_id: &str,
    archived_task_id: &str,
    now: i64,
) -> Option<[i64; 6]> {
    let data = parse_app_data_for_mutation(raw, now)?;
    let sessions_before = data.sessions.len() as i64;
    let archived_before = data.archived_tasks.len() as i64;
    let sessions_after = data
        .sessions
        .iter()
        .filter(|session| session_id.is_empty() || session.id != session_id)
        .count() as i64;
    let archived_after = data
        .archived_tasks
        .iter()
        .filter(|archived_task| {
            archived_task_id.trim().is_empty() || archived_task.id != archived_task_id
        })
        .count() as i64;
    Some([
        sessions_before,
        sessions_after,
        sessions_before.saturating_sub(sessions_after),
        archived_before,
        archived_after,
        archived_before.saturating_sub(archived_after),
    ])
}

pub fn update_slot_title_app_data_json(
    raw: &str,
    slot_id: i32,
    title: &str,
    now: i64,
) -> Option<String> {
    let mut data = parse_app_data_for_mutation(raw, now)?;
    for slot in &mut data.slots {
        if slot.id == slot_id {
            let revision = next_mutation_revision(slot.title_updated_at_epoch_millis, now)?;
            slot.title = title.to_string();
            slot.updated_at = slot.updated_at.max(revision);
            slot.title_updated_at_epoch_millis = revision;
            break;
        }
    }
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn update_slot_note_app_data_json(
    raw: &str,
    slot_id: i32,
    note: &str,
    now: i64,
) -> Option<String> {
    let mut data = parse_app_data_for_mutation(raw, now)?;
    for slot in &mut data.slots {
        if slot.id == slot_id {
            let revision = next_mutation_revision(slot.note_updated_at_epoch_millis, now)?;
            slot.note = note.to_string();
            slot.updated_at = slot.updated_at.max(revision);
            slot.note_updated_at_epoch_millis = revision;
            break;
        }
    }
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn set_slot_category_app_data_json(
    raw: &str,
    slot_id: i32,
    category_id: &str,
    now: i64,
) -> Option<String> {
    let mut data = parse_app_data_for_mutation(raw, now)?;
    let next_category_id = if category_id.is_empty() {
        None
    } else {
        Some(category_id.to_string())
    };
    for slot in &mut data.slots {
        if slot.id == slot_id {
            let revision = next_mutation_revision(slot.category_updated_at_epoch_millis, now)?;
            slot.category_id = next_category_id.clone();
            slot.updated_at = slot.updated_at.max(revision);
            slot.category_updated_at_epoch_millis = revision;
            break;
        }
    }
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn set_slot_order_app_data_json(raw: &str, slot_order: &[i32], now: i64) -> Option<String> {
    let mut data = parse_app_data_for_mutation(raw, now)?;
    data.slot_order = normalize_slot_order(slot_order);
    data.slot_order_updated_at_epoch_millis =
        next_mutation_revision(data.slot_order_updated_at_epoch_millis, now)?;
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn add_category_and_assign_app_data_json(
    raw: &str,
    slot_id: i32,
    category_id: &str,
    name: &str,
    now: i64,
) -> Option<String> {
    let safe_name = name.to_string();
    if safe_name.trim().is_empty() {
        return None;
    }
    let mut data = parse_app_data_for_mutation(raw, now)?;
    let existing_category = data
        .categories
        .iter()
        .find(|category| category.name.eq_ignore_ascii_case(&safe_name))
        .cloned();
    let selected_category = existing_category.unwrap_or_else(|| Category {
        id: if category_id.is_empty() {
            format!("category-{now}-{}", data.categories.len().saturating_add(1))
        } else {
            category_id.to_string()
        },
        name: safe_name,
        accent_seed: accent_seed_for_category_index(data.categories.len()),
        updated_at_epoch_millis: now,
    });

    if !data
        .categories
        .iter()
        .any(|category| category.id == selected_category.id)
    {
        data.categories.push(selected_category.clone());
    }

    if (1..=DEFAULT_SLOT_COUNT).contains(&slot_id) {
        for slot in &mut data.slots {
            if slot.id == slot_id {
                let revision = next_mutation_revision(slot.category_updated_at_epoch_millis, now)?;
                slot.category_id = Some(selected_category.id.clone());
                slot.updated_at = slot.updated_at.max(revision);
                slot.category_updated_at_epoch_millis = revision;
                break;
            }
        }
    }

    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn start_slot_app_data_json(raw: &str, slot_id: i32, now: i64) -> Option<String> {
    let data = parse_app_data_for_mutation(raw, now)?;
    let mut data = resolve_micro_breaks_for_app_data(data, now)?;
    let valid_category_ids = data
        .categories
        .iter()
        .map(|category| category.id.clone())
        .collect::<HashSet<_>>();
    for slot in &mut data.slots {
        if slot.id == slot_id && slot.running_since_epoch_millis.is_none() {
            *slot = slot.clone().sanitized(&valid_category_ids, now);
            let revision = next_mutation_revision(slot.running_updated_at_epoch_millis, now)?;
            slot.running_since_epoch_millis = Some(now);
            slot.active_run_id = format!(
                "{}-{:032x}",
                deterministic_timer_run_id(slot.id, now),
                rand::random::<u128>()
            );
            slot.updated_at = slot.updated_at.max(revision);
            slot.running_updated_at_epoch_millis = revision;
            break;
        }
    }
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn pause_slots_app_data_json(raw: &str, slot_ids: &[i32], now: i64) -> Option<String> {
    let target_slot_ids = slot_ids.iter().copied().collect::<HashSet<_>>();
    if target_slot_ids.is_empty() {
        return None;
    }
    let data = parse_app_data_for_mutation(raw, now)?;
    let mut data = resolve_micro_breaks_for_app_data(data, now)?;
    let mut pause_sessions = Vec::<TimerSession>::new();
    let valid_category_ids = data
        .categories
        .iter()
        .map(|category| category.id.clone())
        .collect::<HashSet<_>>();
    for slot in &mut data.slots {
        if target_slot_ids.contains(&slot.id) {
            let (next_slot, session) =
                pause_slot_micro_break(slot.clone().sanitized(&valid_category_ids, now), now)?;
            *slot = next_slot;
            if let Some(session) = session {
                pause_sessions.push(session);
            }
        }
    }
    pause_sessions.sort_by_key(|session| Reverse(session.ended_at_epoch_millis));
    if !pause_sessions.is_empty() {
        let mut sessions = pause_sessions;
        sessions.extend(data.sessions);
        data.sessions = sessions;
    }
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn reset_slot_app_data_json(raw: &str, slot_id: i32, now: i64) -> Option<String> {
    let data = parse_app_data_for_mutation(raw, now)?;
    let mut data = resolve_micro_breaks_for_app_data(data, now)?;
    let valid_category_ids = data
        .categories
        .iter()
        .map(|category| category.id.clone())
        .collect::<HashSet<_>>();
    let mut session_to_add = None;
    for slot in &mut data.slots {
        if slot.id == slot_id {
            let (paused_slot, session) =
                pause_slot_micro_break(slot.clone().sanitized(&valid_category_ids, now), now)?;
            let mut reset_slot = paused_slot.cleared_micro_break_tracking(now)?;
            let accumulated_revision =
                next_mutation_revision(reset_slot.accumulated_updated_at_epoch_millis, now)?;
            reset_slot.accumulated_millis = 0;
            reset_slot.updated_at = reset_slot.updated_at.max(accumulated_revision);
            reset_slot.accumulated_updated_at_epoch_millis = accumulated_revision;
            *slot = reset_slot;
            session_to_add = session;
            break;
        }
    }
    if let Some(session) = session_to_add {
        data.sessions.insert(0, session);
    }
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn create_note_folder_app_data_json(
    raw: &str,
    folder_id: &str,
    name: &str,
    now: i64,
) -> Option<String> {
    let safe_name = name.to_string();
    if safe_name.trim().is_empty() {
        return None;
    }
    let mut data = parse_app_data_for_mutation(raw, now)?;
    if let Some(existing) = data
        .note_folders
        .iter()
        .find(|folder| folder.name.eq_ignore_ascii_case(&safe_name))
    {
        data.note_preferences.selected_folder_id = Some(existing.id.clone());
    } else {
        let id = if folder_id.is_empty() {
            format!("folder-{now}-{}", data.note_folders.len().saturating_add(1))
        } else {
            folder_id.to_string()
        };
        let folder_revision = next_entity_mutation_revision(
            &data.tombstones,
            TOMBSTONE_ENTITY_NOTE_FOLDER,
            &id,
            0,
            now,
        )?;
        data.note_folders.insert(
            0,
            NoteFolder {
                id: id.clone(),
                name: safe_name,
                created_at_epoch_millis: now,
                updated_at_epoch_millis: folder_revision,
            },
        );
        data.note_preferences.selected_folder_id = Some(id);
    }
    data.note_preferences_updated_at_epoch_millis =
        next_mutation_revision(data.note_preferences_updated_at_epoch_millis, now)?;
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn rename_note_folder_app_data_json(
    raw: &str,
    folder_id: &str,
    name: &str,
    now: i64,
) -> Option<String> {
    let safe_name = name.to_string();
    if safe_name.trim().is_empty() {
        return None;
    }
    let mut data = parse_app_data_for_mutation(raw, now)?;
    for folder in &mut data.note_folders {
        if folder.id == folder_id {
            let revision = next_mutation_revision(note_folder_revision_epoch_millis(folder), now)?;
            folder.name = safe_name;
            folder.updated_at_epoch_millis = revision;
            break;
        }
    }
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn delete_note_folder_app_data_json(raw: &str, folder_id: &str, now: i64) -> Option<String> {
    let mut data = parse_app_data_for_mutation(raw, now)?;
    let record_revision = data
        .note_folders
        .iter()
        .filter(|folder| folder.id == folder_id)
        .map(note_folder_revision_epoch_millis)
        .max()
        .unwrap_or(0);
    let deletion_revision = next_deletion_revision(
        &data.tombstones,
        TOMBSTONE_ENTITY_NOTE_FOLDER,
        folder_id,
        record_revision,
        now,
    )?;
    data.note_folders.retain(|folder| folder.id != folder_id);
    upsert_data_tombstone(
        &mut data.tombstones,
        TOMBSTONE_ENTITY_NOTE_FOLDER,
        folder_id,
        deletion_revision,
    );
    for note in &mut data.notes {
        if note.folder_id.as_deref() == Some(folder_id) {
            note.folder_id = None;
            note.updated_at_epoch_millis =
                next_mutation_revision(note_revision_epoch_millis(note), now)?;
        }
    }
    if data.note_preferences.selected_folder_id.as_deref() == Some(folder_id) {
        data.note_preferences.selected_folder_id = None;
        data.note_preferences_updated_at_epoch_millis =
            next_mutation_revision(data.note_preferences_updated_at_epoch_millis, now)?;
    }
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn set_selected_note_folder_app_data_json(
    raw: &str,
    folder_id: &str,
    now: i64,
) -> Option<String> {
    let mut data = parse_app_data_for_mutation(raw, now)?;
    data.note_preferences.selected_folder_id = if folder_id.is_empty() {
        None
    } else {
        data.note_folders
            .iter()
            .any(|folder| folder.id == folder_id)
            .then(|| folder_id.to_string())
    };
    data.note_preferences_updated_at_epoch_millis =
        next_mutation_revision(data.note_preferences_updated_at_epoch_millis, now)?;
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn set_note_sort_mode_app_data_json(
    raw: &str,
    sort_mode_code: i32,
    now: i64,
) -> Option<String> {
    let mut data = parse_app_data_for_mutation(raw, now)?;
    data.note_preferences.sort_mode = match sort_mode_code {
        0 => NoteSortMode::UpdatedDesc,
        1 => NoteSortMode::CreatedDesc,
        2 => NoteSortMode::CreatedAsc,
        3 => NoteSortMode::TitleAsc,
        _ => return None,
    };
    data.note_preferences_updated_at_epoch_millis =
        next_mutation_revision(data.note_preferences_updated_at_epoch_millis, now)?;
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn move_note_to_folder_app_data_json(
    raw: &str,
    note_id: &str,
    folder_id: &str,
    now: i64,
) -> Option<String> {
    let mut data = parse_app_data_for_mutation(raw, now)?;
    let safe_folder_id = if folder_id.is_empty() {
        None
    } else {
        data.note_folders
            .iter()
            .any(|folder| folder.id == folder_id)
            .then(|| folder_id.to_string())
    };
    for note in &mut data.notes {
        if note.id == note_id {
            note.folder_id = safe_folder_id.clone();
            note.updated_at_epoch_millis =
                next_mutation_revision(note_revision_epoch_millis(note), now)?;
            break;
        }
    }
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn restore_note_app_data_json(raw: &str, note_id: &str, now: i64) -> Option<String> {
    let mut data = parse_app_data_for_mutation(raw, now)?;
    for note in &mut data.notes {
        if note.id == note_id {
            let revision = next_mutation_revision(note_revision_epoch_millis(note), now)?;
            note.deleted_at_epoch_millis = None;
            note.updated_at_epoch_millis = revision;
            break;
        }
    }
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn delete_note_app_data_json(raw: &str, note_id: &str, now: i64) -> Option<String> {
    let mut data = parse_app_data_for_mutation(raw, now)?;
    for note in &mut data.notes {
        if note.id == note_id {
            let revision = next_mutation_revision(note_revision_epoch_millis(note), now)?;
            note.pinned = false;
            note.deleted_at_epoch_millis = Some(now.max(0));
            note.updated_at_epoch_millis = revision;
            break;
        }
    }
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn set_note_pinned_app_data_json(
    raw: &str,
    note_id: &str,
    pinned: bool,
    now: i64,
) -> Option<String> {
    let mut data = parse_app_data_for_mutation(raw, now)?;
    for note in &mut data.notes {
        if note.id == note_id {
            let revision = next_mutation_revision(note_revision_epoch_millis(note), now)?;
            note.pinned = pinned && note.deleted_at_epoch_millis.is_none();
            note.updated_at_epoch_millis = revision;
            break;
        }
    }
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn capture_note_revision_app_data_json(raw: &str, note_id: &str, now: i64) -> Option<String> {
    let mut data = parse_app_data_for_mutation(raw, now)?;
    for note in &mut data.notes {
        if note.id == note_id {
            if note.encryption.is_some() {
                break;
            }
            let previous = note.clone();
            let revision = next_mutation_revision(note_revision_epoch_millis(&previous), now)?;
            let snapshot = previous.snapshot_for_history(now.max(0), revision);
            let mut revisions = Vec::with_capacity(note.revisions.len() + 1);
            revisions.push(snapshot);
            revisions.extend(
                note.revisions
                    .iter()
                    .filter(|revision| {
                        !revision.label.is_empty() || !revision.matches_note(&previous)
                    })
                    .cloned(),
            );
            note.revisions = revisions;
            note.updated_at_epoch_millis = revision;
            break;
        }
    }
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn restore_note_revision_app_data_json(
    raw: &str,
    note_id: &str,
    revision_id: &str,
    now: i64,
) -> Option<String> {
    let revision_kind_was_explicit = serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|root| {
            root.get("notes")?
                .as_array()?
                .iter()
                .find(|note| note.get("id").and_then(serde_json::Value::as_str) == Some(note_id))?
                .get("revisions")?
                .as_array()?
                .iter()
                .find(|revision| {
                    revision.get("id").and_then(serde_json::Value::as_str) == Some(revision_id)
                })?
                .get("kind")?
                .as_str()
                .map(|kind| !kind.trim().is_empty())
        })
        .unwrap_or(false);
    let mut data = parse_app_data_for_mutation(raw, now)?;
    let Some(note_index) = data.notes.iter().position(|note| note.id == note_id) else {
        return serde_json::to_string(&data).ok();
    };
    let target = data.notes[note_index].clone();
    if target.encryption.is_some() {
        return serde_json::to_string(&data).ok();
    }
    let Some(revision) = target
        .revisions
        .iter()
        .find(|revision| revision.id == revision_id)
        .cloned()
    else {
        return serde_json::to_string(&data).ok();
    };
    let mutation_revision = next_mutation_revision(note_revision_epoch_millis(&target), now)?;
    let mut restored_attachments = target.attachments.clone();
    restored_attachments.extend(revision.attachments.iter().cloned());
    let explicitly_restored_attachment_ids = revision
        .attachments
        .iter()
        .map(|attachment| attachment.id.clone())
        .chain(revision.attachment_ids.iter().cloned())
        .collect::<HashSet<_>>();
    for attachment in &mut restored_attachments {
        if explicitly_restored_attachment_ids.contains(&attachment.id) {
            attachment.updated_at_epoch_millis = next_entity_mutation_revision(
                &data.tombstones,
                TOMBSTONE_ENTITY_NOTE_ATTACHMENT,
                &attachment.id,
                attachment.updated_at_epoch_millis,
                now,
            )?;
        }
    }
    restored_attachments = distinct_by_latest(
        restored_attachments,
        |attachment| attachment.id.clone(),
        |attachment| attachment.updated_at_epoch_millis,
    );
    let valid_attachment_ids = restored_attachments
        .iter()
        .map(|attachment| attachment.id.clone())
        .chain(revision.attachment_ids.iter().cloned())
        .collect::<HashSet<_>>();
    let restored_document = revision
        .document
        .clone()
        .sanitized(&valid_attachment_ids, now);
    let snapshot = target.snapshot_for_history(now.max(0), mutation_revision);
    let mut restored = target.clone();
    restored.title = revision.title;
    restored.content = revision.content;
    restored.kind = if revision_kind_was_explicit {
        revision.kind
    } else {
        target.kind.clone()
    };
    restored.document = restored_document;
    if restored.document.knowledge.is_none()
        && (target.document.knowledge.is_some()
            || target.document.blocks.iter().any(|b| b.knowledge.is_some()))
    {
        // Explicit restoration keeps the current format marker so synchronization
        // can distinguish it from an older editor dropping unsupported metadata.
        restored.document.knowledge = Some(crate::knowledge::KnowledgePage {
            parent_id: target
                .document
                .knowledge
                .as_ref()
                .and_then(|m| m.parent_id.clone()),
            ..Default::default()
        });
    }
    restored.accent_seed = revision.accent_seed;
    restored.pinned = revision.pinned;
    restored.folder_id = revision.folder_id;
    restored.attachments = restored_attachments;
    restored.deleted_at_epoch_millis = None;
    restored.updated_at_epoch_millis = mutation_revision;
    restored.revisions = std::iter::once(snapshot)
        .chain(
            target
                .revisions
                .into_iter()
                .filter(|existing| existing.id != revision_id),
        )
        .collect();

    data.notes.remove(note_index);
    data.notes.insert(0, restored);
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn delete_note_permanently_app_data_json(raw: &str, note_id: &str, now: i64) -> Option<String> {
    let mut data = parse_app_data_for_mutation(raw, now)?;
    let removed_note = data.notes.iter().find(|note| note.id == note_id).cloned();
    let record_revision = removed_note
        .as_ref()
        .map(note_revision_epoch_millis)
        .unwrap_or(0);
    let deletion_revision = next_deletion_revision(
        &data.tombstones,
        TOMBSTONE_ENTITY_NOTE,
        note_id,
        record_revision,
        now,
    )?;
    let removed_attachment_revisions = removed_note
        .as_ref()
        .map(note_attachment_revision_index)
        .unwrap_or_default();
    data.notes.retain(|note| note.id != note_id);
    upsert_data_tombstone(
        &mut data.tombstones,
        TOMBSTONE_ENTITY_NOTE,
        note_id,
        deletion_revision,
    );
    for (attachment_id, attachment_revision) in removed_attachment_revisions {
        if !notes_reference_attachment(&data.notes, &attachment_id) {
            let deletion_revision = next_deletion_revision(
                &data.tombstones,
                TOMBSTONE_ENTITY_NOTE_ATTACHMENT,
                &attachment_id,
                attachment_revision,
                now,
            )?;
            upsert_data_tombstone(
                &mut data.tombstones,
                TOMBSTONE_ENTITY_NOTE_ATTACHMENT,
                &attachment_id,
                deletion_revision,
            );
            let media_deletion_revision = next_deletion_revision(
                &data.tombstones,
                TOMBSTONE_ENTITY_NOTE_MEDIA,
                &attachment_id,
                attachment_revision,
                now,
            )?;
            upsert_data_tombstone(
                &mut data.tombstones,
                TOMBSTONE_ENTITY_NOTE_MEDIA,
                &attachment_id,
                media_deletion_revision,
            );
        }
    }
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn empty_note_trash_app_data_json(raw: &str, now: i64) -> Option<String> {
    let mut data = parse_app_data_for_mutation(raw, now)?;
    let removed_notes = data
        .notes
        .iter()
        .filter(|note| note.deleted_at_epoch_millis.is_some())
        .cloned()
        .collect::<Vec<_>>();
    data.notes
        .retain(|note| note.deleted_at_epoch_millis.is_none());
    let mut removed_attachment_revisions = HashMap::<String, i64>::new();
    for note in removed_notes {
        let deletion_revision = next_deletion_revision(
            &data.tombstones,
            TOMBSTONE_ENTITY_NOTE,
            &note.id,
            note_revision_epoch_millis(&note),
            now,
        )?;
        upsert_data_tombstone(
            &mut data.tombstones,
            TOMBSTONE_ENTITY_NOTE,
            &note.id,
            deletion_revision,
        );
        for (attachment_id, revision) in note_attachment_revision_index(&note) {
            removed_attachment_revisions
                .entry(attachment_id)
                .and_modify(|existing| *existing = (*existing).max(revision))
                .or_insert(revision);
        }
    }
    for (attachment_id, attachment_revision) in removed_attachment_revisions {
        if !notes_reference_attachment(&data.notes, &attachment_id) {
            let deletion_revision = next_deletion_revision(
                &data.tombstones,
                TOMBSTONE_ENTITY_NOTE_ATTACHMENT,
                &attachment_id,
                attachment_revision,
                now,
            )?;
            upsert_data_tombstone(
                &mut data.tombstones,
                TOMBSTONE_ENTITY_NOTE_ATTACHMENT,
                &attachment_id,
                deletion_revision,
            );
            let media_deletion_revision = next_deletion_revision(
                &data.tombstones,
                TOMBSTONE_ENTITY_NOTE_MEDIA,
                &attachment_id,
                attachment_revision,
                now,
            )?;
            upsert_data_tombstone(
                &mut data.tombstones,
                TOMBSTONE_ENTITY_NOTE_MEDIA,
                &attachment_id,
                media_deletion_revision,
            );
        }
    }
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn set_theme_mode_app_data_json(raw: &str, theme_mode_code: i32, now: i64) -> Option<String> {
    let mut data = parse_app_data_for_mutation(raw, now)?;
    let (theme_mode, oled_theme_enabled) = match theme_mode_code {
        0 => (ThemeMode::System, false),
        1 => (ThemeMode::Light, false),
        2 => (ThemeMode::Dark, false),
        // Keep the persisted enum DARK so older clients can still decode the
        // snapshot. They ignore the additive OLED flag and render normal dark.
        3 => (ThemeMode::Dark, true),
        _ => return None,
    };
    data.theme_mode = theme_mode;
    data.oled_theme_enabled = oled_theme_enabled;
    data.theme_mode_updated_at_epoch_millis =
        next_mutation_revision(data.theme_mode_updated_at_epoch_millis, now)?;
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn note_folder_count_pairs(raw: &str) -> Option<Vec<(String, i32)>> {
    let data = serde_json::from_str::<AppData>(raw).ok()?;
    let mut pairs = Vec::<(String, i32)>::new();
    for note in data.notes {
        if note.deleted_at_epoch_millis.is_some() {
            continue;
        }
        let Some(folder_id) = note.folder_id else {
            continue;
        };
        if let Some((_, count)) = pairs.iter_mut().find(|(id, _)| id == &folder_id) {
            *count = count.saturating_add(1);
        } else {
            pairs.push((folder_id, 1));
        }
    }
    Some(pairs)
}

pub fn note_visibility_indices(raw: &str, deleted: bool) -> Option<Vec<i32>> {
    let data = serde_json::from_str::<AppData>(raw).ok()?;
    Some(
        data.notes
            .iter()
            .enumerate()
            .filter_map(|(index, note)| {
                (note.deleted_at_epoch_millis.is_some() == deleted).then_some(index as i32)
            })
            .collect(),
    )
}

pub fn is_note_blank_draft_json(raw: &str, now: i64) -> Option<bool> {
    let note = serde_json::from_str::<NoteEntry>(raw).ok()?;
    let valid_attachment_ids = note.attachment_ids();
    let document = note
        .resolved_document()
        .sanitized(&valid_attachment_ids, now);
    Some(note.title.trim().is_empty() && document.is_blank() && note.attachments.is_empty())
}

pub fn update_finance_profile_app_data_json(
    raw: &str,
    profile_json: &str,
    now: i64,
) -> Option<String> {
    let mut data = parse_app_data_for_mutation(raw, now)?;
    let next_profile = serde_json::from_str::<FinanceProfile>(profile_json)
        .ok()?
        .sanitized();

    for day_key in data.finance_profile.daily_ledgers.keys() {
        if !next_profile.daily_ledgers.contains_key(day_key) {
            let record_revision = data
                .finance_day_ledger_revisions
                .get(day_key)
                .copied()
                .unwrap_or(data.finance_profile_updated_at_epoch_millis);
            let deletion_revision = next_deletion_revision(
                &data.tombstones,
                TOMBSTONE_ENTITY_FINANCE_DAY_LEDGER,
                day_key,
                record_revision,
                now,
            )?;
            upsert_data_tombstone(
                &mut data.tombstones,
                TOMBSTONE_ENTITY_FINANCE_DAY_LEDGER,
                day_key,
                deletion_revision,
            );
            data.finance_day_ledger_revisions.remove(day_key);
        }
    }
    for (day_key, next_ledger) in &next_profile.daily_ledgers {
        if data.finance_profile.daily_ledgers.get(day_key) != Some(next_ledger) {
            let existing_revision = data
                .finance_day_ledger_revisions
                .get(day_key)
                .copied()
                .unwrap_or(data.finance_profile_updated_at_epoch_millis);
            let revision = next_entity_mutation_revision(
                &data.tombstones,
                TOMBSTONE_ENTITY_FINANCE_DAY_LEDGER,
                day_key,
                existing_revision,
                now,
            )?;
            data.finance_day_ledger_revisions
                .insert(day_key.clone(), revision);
        }
    }
    for month_key in data.finance_profile.monthly_snapshots.keys() {
        if !next_profile.monthly_snapshots.contains_key(month_key) {
            let record_revision = data
                .finance_month_snapshot_revisions
                .get(month_key)
                .copied()
                .unwrap_or(data.finance_profile_updated_at_epoch_millis);
            let deletion_revision = next_deletion_revision(
                &data.tombstones,
                TOMBSTONE_ENTITY_FINANCE_MONTH_SNAPSHOT,
                month_key,
                record_revision,
                now,
            )?;
            upsert_data_tombstone(
                &mut data.tombstones,
                TOMBSTONE_ENTITY_FINANCE_MONTH_SNAPSHOT,
                month_key,
                deletion_revision,
            );
            data.finance_month_snapshot_revisions.remove(month_key);
        }
    }
    for (month_key, next_snapshot) in &next_profile.monthly_snapshots {
        if data.finance_profile.monthly_snapshots.get(month_key) != Some(next_snapshot) {
            let existing_revision = data
                .finance_month_snapshot_revisions
                .get(month_key)
                .copied()
                .unwrap_or(data.finance_profile_updated_at_epoch_millis);
            let revision = next_entity_mutation_revision(
                &data.tombstones,
                TOMBSTONE_ENTITY_FINANCE_MONTH_SNAPSHOT,
                month_key,
                existing_revision,
                now,
            )?;
            data.finance_month_snapshot_revisions
                .insert(month_key.clone(), revision);
        }
    }

    data.finance_profile = next_profile;
    data.finance_profile_updated_at_epoch_millis =
        next_mutation_revision(data.finance_profile_updated_at_epoch_millis, now)?;
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

/// Imports a finance profile without treating periods omitted by the import as
/// deletions. Existing periods win on key collisions, while periods present
/// only in the imported profile are restored with fresh revisions. This is the
/// safe backend primitive for restoring an older or partial finance backup.
pub fn merge_finance_profile_app_data_json(
    raw: &str,
    profile_json: &str,
    now: i64,
) -> Option<String> {
    let data = parse_app_data_for_mutation(raw, now)?;
    let imported = serde_json::from_str::<FinanceProfile>(profile_json)
        .ok()?
        .sanitized();
    let merged = merge_finance_profiles_for_restore(&data.finance_profile, &imported);
    let merged_json = serde_json::to_string(&merged).ok()?;
    update_finance_profile_app_data_json(raw, &merged_json, now)
}

fn merge_finance_profiles_for_restore(
    current: &FinanceProfile,
    imported: &FinanceProfile,
) -> FinanceProfile {
    if current == &FinanceProfile::default() {
        return imported.clone();
    }

    let mut merged = current.clone();
    for (day_key, imported_ledger) in &imported.daily_ledgers {
        if let Some(current_ledger) = merged.daily_ledgers.get_mut(day_key) {
            current_ledger.incomes = merge_finance_rows_for_restore(
                &current_ledger.incomes,
                &imported_ledger.incomes,
                |entry| entry.id.trim().to_string(),
            );
            current_ledger.expenses = merge_finance_rows_for_restore(
                &current_ledger.expenses,
                &imported_ledger.expenses,
                |entry| entry.id.trim().to_string(),
            );
            if current_ledger.note.trim().is_empty() {
                current_ledger.note = imported_ledger.note.clone();
            }
            current_ledger.confirmed_at_epoch_millis = current_ledger
                .confirmed_at_epoch_millis
                .max(imported_ledger.confirmed_at_epoch_millis);
        } else {
            merged
                .daily_ledgers
                .insert(day_key.clone(), imported_ledger.clone());
        }
    }
    for (month_key, imported_snapshot) in &imported.monthly_snapshots {
        if let Some(current_snapshot) = merged.monthly_snapshots.get_mut(month_key) {
            current_snapshot.assets = merge_finance_rows_for_restore(
                &current_snapshot.assets,
                &imported_snapshot.assets,
                |entry| entry.id.trim().to_string(),
            );
            current_snapshot.liabilities = merge_finance_rows_for_restore(
                &current_snapshot.liabilities,
                &imported_snapshot.liabilities,
                |entry| entry.id.trim().to_string(),
            );
            if current_snapshot.note.trim().is_empty() {
                current_snapshot.note = imported_snapshot.note.clone();
            }
            current_snapshot.confirmed_at_epoch_millis = current_snapshot
                .confirmed_at_epoch_millis
                .max(imported_snapshot.confirmed_at_epoch_millis);
        } else {
            merged
                .monthly_snapshots
                .insert(month_key.clone(), imported_snapshot.clone());
        }
    }

    // Legacy scalar-only backups remain useful when the current profile has
    // never populated those fields. Once a scalar group has current data, it
    // is preserved as the safer side of a non-destructive import.
    let current_legacy_amounts_are_empty = current.active_income_monthly == 0
        && current.asset_income_monthly == 0
        && current.living_expense_monthly == 0
        && current.liability_payment_monthly == 0
        && current.cash_reserve == 0
        && current.productive_asset_value == 0
        && current.liability_balance == 0;
    if current_legacy_amounts_are_empty {
        merged.active_income_monthly = imported.active_income_monthly;
        merged.asset_income_monthly = imported.asset_income_monthly;
        merged.living_expense_monthly = imported.living_expense_monthly;
        merged.liability_payment_monthly = imported.liability_payment_monthly;
        merged.cash_reserve = imported.cash_reserve;
        merged.productive_asset_value = imported.productive_asset_value;
        merged.liability_balance = imported.liability_balance;
    }
    if current.acquisition_focus.trim().is_empty() {
        merged.acquisition_focus = imported.acquisition_focus.clone();
    }
    if current.liability_focus.trim().is_empty() {
        merged.liability_focus = imported.liability_focus.clone();
    }
    merged
}

fn merge_finance_rows_for_restore<T>(
    current: &[T],
    imported: &[T],
    id_of: impl Fn(&T) -> String,
) -> Vec<T>
where
    T: Clone + PartialEq,
{
    let mut merged = current.to_vec();
    let mut known_ids = current
        .iter()
        .filter_map(|entry| {
            let id = id_of(entry);
            (!id.is_empty()).then_some(id)
        })
        .collect::<HashSet<_>>();

    for imported_entry in imported {
        let id = id_of(imported_entry);
        if !id.is_empty() {
            if known_ids.insert(id) {
                merged.push(imported_entry.clone());
            }
            continue;
        }

        let current_count = current
            .iter()
            .filter(|entry| id_of(entry).is_empty() && *entry == imported_entry)
            .count();
        let imported_count = imported
            .iter()
            .filter(|entry| id_of(entry).is_empty() && *entry == imported_entry)
            .count();
        let merged_count = merged
            .iter()
            .filter(|entry| id_of(entry).is_empty() && *entry == imported_entry)
            .count();
        if merged_count < current_count.max(imported_count) {
            merged.push(imported_entry.clone());
        }
    }
    merged
}

pub fn archive_slot_app_data_json(
    raw: &str,
    slot_id: i32,
    archived_task_id: &str,
    now: i64,
) -> Option<String> {
    if archived_task_id.trim().is_empty() {
        return None;
    }
    let mut data = parse_app_data_for_mutation(raw, now)?;
    let Some(slot_index) = data.slots.iter().position(|slot| slot.id == slot_id) else {
        return serde_json::to_string(&data).ok();
    };
    let slot = data.slots[slot_index].clone();
    if slot.running_since_epoch_millis.is_some() || slot.is_blank_slate() {
        return serde_json::to_string(&data).ok();
    }

    let existing_archive_revision = data
        .archived_tasks
        .iter()
        .filter(|task| task.id == archived_task_id)
        .map(|task| task.updated_at_epoch_millis)
        .max()
        .unwrap_or(0);
    let archive_revision = next_entity_mutation_revision(
        &data.tombstones,
        TOMBSTONE_ENTITY_ARCHIVED_TASK,
        archived_task_id,
        existing_archive_revision,
        now,
    )?;
    data.archived_tasks.insert(
        0,
        ArchivedTask {
            id: archived_task_id.to_string(),
            original_slot_id: slot.id,
            title: slot.title,
            category_id: slot.category_id,
            note: slot.note,
            accumulated_millis: slot.accumulated_millis,
            archived_at_epoch_millis: now.max(0),
            updated_at_epoch_millis: archive_revision,
        },
    );
    data.slots[slot_index] = data.slots[slot_index]
        .clone()
        .cleared_micro_break_tracking(now)?
        .as_blank_task(now)?;

    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn restore_archived_task_app_data_json(
    raw: &str,
    archived_task_id: &str,
    now: i64,
) -> Option<String> {
    let mut data = parse_app_data_for_mutation(raw, now)?;
    let Some(task_index) = data
        .archived_tasks
        .iter()
        .position(|archived_task| archived_task.id == archived_task_id)
    else {
        return serde_json::to_string(&data).ok();
    };
    let archived_task = data.archived_tasks[task_index].clone();
    let Some(target_slot_id) =
        restore_target_slot_id_for_task(&data.slots, archived_task.original_slot_id)
    else {
        return serde_json::to_string(&data).ok();
    };

    for slot in &mut data.slots {
        if slot.id == target_slot_id {
            *slot = slot
                .clone()
                .cleared_micro_break_tracking(now)?
                .with_restored_archived_task(&archived_task, now)?;
            break;
        }
    }
    data.archived_tasks.remove(task_index);
    let deletion_revision = next_deletion_revision(
        &data.tombstones,
        TOMBSTONE_ENTITY_ARCHIVED_TASK,
        archived_task_id,
        archived_task.updated_at_epoch_millis,
        now,
    )?;
    upsert_data_tombstone(
        &mut data.tombstones,
        TOMBSTONE_ENTITY_ARCHIVED_TASK,
        archived_task_id,
        deletion_revision,
    );

    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn import_note_image_attachment_app_data_json(
    raw: &str,
    note_id: &str,
    attachment_json: &str,
    now: i64,
) -> Option<String> {
    let mut data = parse_app_data_for_mutation(raw, now)?;
    let mut attachment = serde_json::from_str::<NoteAttachment>(attachment_json).ok()?;
    attachment = attachment.sanitized(now);
    attachment.updated_at_epoch_millis = next_entity_mutation_revision(
        &data.tombstones,
        TOMBSTONE_ENTITY_NOTE_ATTACHMENT,
        &attachment.id,
        attachment.updated_at_epoch_millis,
        now,
    )?;
    if attachment.id.is_empty() {
        return None;
    }
    let Some(note) = data.notes.iter_mut().find(|note| note.id == note_id) else {
        return serde_json::to_string(&data).ok();
    };
    if note.deleted_at_epoch_millis.is_some() {
        return serde_json::to_string(&data).ok();
    }
    if note
        .attachments
        .iter()
        .any(|existing| existing.id == attachment.id)
    {
        return serde_json::to_string(&data).ok();
    }

    note.attachments.push(attachment.clone());
    let mut document = note.resolved_document();
    if !document.blocks.iter().any(|block| {
        matches!(&block.block_type, NoteBlockType::Image)
            && block.attachment_id.as_deref() == Some(attachment.id.as_str())
    }) {
        document.blocks.push(NoteBlock {
            id: format!("image-block-{}", attachment.id),
            block_type: NoteBlockType::Image,
            attachment_id: Some(attachment.id.clone()),
            caption: attachment.display_name.clone(),
            ..NoteBlock::default()
        });
    }
    note.document = document.sanitized(&note.attachment_ids(), now);
    note.content = note.document.storage_content();
    note.updated_at_epoch_millis = next_mutation_revision(note_revision_epoch_millis(note), now)?;

    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn delete_note_attachment_app_data_json(
    raw: &str,
    note_id: &str,
    attachment_id: &str,
    now: i64,
) -> Option<String> {
    if attachment_id.is_empty() {
        return None;
    }
    let mut data = parse_app_data_for_mutation(raw, now)?;
    let Some(note) = data.notes.iter_mut().find(|note| note.id == note_id) else {
        return serde_json::to_string(&data).ok();
    };
    if !note
        .attachments
        .iter()
        .any(|attachment| attachment.id == attachment_id)
    {
        return serde_json::to_string(&data).ok();
    }

    if note.document.knowledge.as_ref().is_some_and(|m| m.locked) {
        return None;
    }
    let attachment_revision = note
        .attachments
        .iter()
        .filter(|attachment| attachment.id == attachment_id)
        .map(|attachment| attachment.updated_at_epoch_millis)
        .max()
        .unwrap_or(0);
    let deletion_revision = next_deletion_revision(
        &data.tombstones,
        TOMBSTONE_ENTITY_NOTE_ATTACHMENT,
        attachment_id,
        attachment_revision,
        now,
    )?;
    upsert_data_tombstone(
        &mut data.tombstones,
        TOMBSTONE_ENTITY_NOTE_ATTACHMENT,
        attachment_id,
        deletion_revision,
    );
    let note_mutation_revision = next_mutation_revision(note_revision_epoch_millis(note), now)?;
    let snapshot = note.snapshot_for_history(now.max(0), note_mutation_revision);
    if !note
        .revisions
        .iter()
        .any(|revision| revision.id == snapshot.id)
    {
        note.revisions.insert(0, snapshot);
    }
    note.attachments
        .retain(|attachment| attachment.id != attachment_id);
    let mut document = note.resolved_document();
    let document_rich_text_enabled = document.rich_text_enabled;
    document.blocks = document
        .blocks
        .into_iter()
        .filter_map(|mut block| match &block.block_type {
            _ if block.attachment_id.as_deref() == Some(attachment_id) => None,
            NoteBlockType::Text if document_rich_text_enabled => {
                block.text =
                    remove_rich_text_attachment_reference_for_app_data(&block.text, attachment_id);
                Some(block)
            }
            _ => Some(block),
        })
        .collect();
    note.document = document.sanitized(&note.attachment_ids(), now);
    note.content = remove_rich_text_attachment_reference_for_app_data(&note.content, attachment_id);
    note.updated_at_epoch_millis = note_mutation_revision;

    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn note_image_import_policy(
    note_exists: bool,
    note_deleted: bool,
    attachment_count: i32,
    max_attachment_count: i32,
) -> i32 {
    if !note_exists || note_deleted {
        1
    } else if attachment_count >= max_attachment_count.max(0) {
        2
    } else {
        0
    }
}

pub fn should_delete_unattached_imported_note_image(imported: bool, attached: bool) -> bool {
    imported && !attached
}

pub fn repository_persist_plan_flags(state_file_exists: bool, backup_file_exists: bool) -> i32 {
    let mut flags = PERSIST_PLAN_ENSURE_BACKUP_AFTER_STATE;
    if state_file_exists {
        flags |= PERSIST_PLAN_COPY_STATE_TO_BACKUP;
    } else if !backup_file_exists {
        flags |= PERSIST_PLAN_WRITE_BACKUP_FROM_ENCODED;
    }
    flags
}

pub const PERSIST_PLAN_COPY_STATE_TO_BACKUP: i32 = 1;
pub const PERSIST_PLAN_WRITE_BACKUP_FROM_ENCODED: i32 = 1 << 1;
pub const PERSIST_PLAN_ENSURE_BACKUP_AFTER_STATE: i32 = 1 << 2;

impl AppData {
    fn sanitized(mut self, now: i64) -> Option<Self> {
        // An older client cannot safely round-trip fields introduced by a
        // newer schema because serde intentionally ignores unknown fields.
        // Refuse the write instead of silently downgrading the database.
        if self.schema_version > APP_DATA_SCHEMA_VERSION {
            return None;
        }
        if self.categories.is_empty() {
            // Categories are reference data. A missing category array must not
            // make otherwise recoverable sessions, notes, or finance history
            // fail validation and be replaced with an empty default database.
            self.categories = default_categories();
        }

        let tombstones = normalize_data_tombstones(std::mem::take(&mut self.tombstones), now);
        let mut tombstone_index = HashMap::<String, HashMap<String, i64>>::new();
        for tombstone in &tombstones {
            tombstone_index
                .entry(tombstone.entity_type.clone())
                .or_default()
                .insert(
                    tombstone.entity_id.clone(),
                    tombstone.deleted_at_epoch_millis,
                );
        }

        let mut categories = std::mem::take(&mut self.categories)
            .into_iter()
            .map(|mut category| {
                category.updated_at_epoch_millis =
                    sanitize_timestamp(category.updated_at_epoch_millis, now);
                category
            })
            .collect::<Vec<_>>();
        assign_missing_recovery_ids(
            &mut categories,
            "category",
            |category| &category.id,
            |category, id| category.id = id,
        );
        categories = distinct_by_latest(
            categories,
            |category| category.id.clone(),
            |category| category.updated_at_epoch_millis,
        );
        let valid_category_ids = categories
            .iter()
            .map(|category| category.id.clone())
            .collect::<HashSet<_>>();

        let mut note_folders = std::mem::take(&mut self.note_folders)
            .into_iter()
            .map(|folder| {
                let created_at = sanitize_timestamp(folder.created_at_epoch_millis, now);
                let updated_at = sanitize_timestamp(folder.updated_at_epoch_millis, now);
                let normalized = NoteFolder {
                    name: folder.name.clone(),
                    created_at_epoch_millis: created_at,
                    updated_at_epoch_millis: updated_at,
                    ..folder
                };
                normalized
            })
            .collect::<Vec<_>>();
        assign_missing_recovery_ids(
            &mut note_folders,
            "note-folder",
            |folder| &folder.id,
            |folder, id| folder.id = id,
        );
        note_folders = distinct_by_latest(
            note_folders,
            |folder| folder.id.clone(),
            note_folder_revision_epoch_millis,
        );
        note_folders.retain(|folder| {
            record_is_newer_than_tombstone(
                &tombstone_index,
                TOMBSTONE_ENTITY_NOTE_FOLDER,
                &folder.id,
                note_folder_revision_epoch_millis(folder),
            )
        });
        note_folders.sort_by_key(|folder| Reverse(folder.updated_at_epoch_millis));
        let valid_folder_ids = note_folders
            .iter()
            .map(|folder| folder.id.clone())
            .collect::<HashSet<_>>();

        let slot_candidates = distinct_by_latest(
            std::mem::take(&mut self.slots)
                .into_iter()
                .map(|slot| slot.sanitized(&valid_category_ids, now))
                .collect::<Vec<_>>(),
            |slot| slot.id,
            |slot| slot.updated_at,
        );
        let mut slots = (1..=DEFAULT_SLOT_COUNT)
            .map(|slot_id| {
                slot_candidates
                    .iter()
                    .find(|slot| slot.id == slot_id)
                    .cloned()
                    .unwrap_or_else(|| TimerSlot {
                        id: slot_id,
                        // Missing slots are structural defaults, not mutations.
                        updated_at: 0,
                        ..TimerSlot::default()
                    })
            })
            .collect::<Vec<_>>();
        let mut out_of_range_slots = slot_candidates
            .iter()
            .filter(|slot| !(1..=DEFAULT_SLOT_COUNT).contains(&slot.id))
            .cloned()
            .collect::<Vec<_>>();
        out_of_range_slots.sort_by_key(|slot| slot.id);
        slots.extend(out_of_range_slots);

        let mut sessions = std::mem::take(&mut self.sessions)
            .into_iter()
            .map(|session| session.sanitized(&valid_category_ids, now))
            .collect::<Vec<_>>();
        assign_missing_recovery_ids(
            &mut sessions,
            "session",
            |session| &session.id,
            |session, id| session.id = id,
        );
        sessions = distinct_by_latest(
            sessions,
            |session| session.id.clone(),
            session_revision_epoch_millis,
        );
        sessions.retain(|session| {
            record_is_newer_than_tombstone(
                &tombstone_index,
                TOMBSTONE_ENTITY_SESSION,
                &session.id,
                session_revision_epoch_millis(session),
            )
        });
        sessions.sort_by_key(|session| Reverse(session_revision_epoch_millis(session)));

        let mut archived_tasks = std::mem::take(&mut self.archived_tasks)
            .into_iter()
            .map(|task| task.sanitized(&valid_category_ids, now))
            .collect::<Vec<_>>();
        assign_missing_recovery_ids(
            &mut archived_tasks,
            "archived-task",
            |task| &task.id,
            |task, id| task.id = id,
        );
        archived_tasks = distinct_by_latest(
            archived_tasks,
            |task| task.id.clone(),
            |task| task.updated_at_epoch_millis,
        );
        archived_tasks.retain(|task| {
            record_is_newer_than_tombstone(
                &tombstone_index,
                TOMBSTONE_ENTITY_ARCHIVED_TASK,
                &task.id,
                task.updated_at_epoch_millis,
            )
        });
        archived_tasks.sort_by_key(|task| Reverse(task.archived_at_epoch_millis));

        let mut notes = std::mem::take(&mut self.notes);
        // A migrated version-1 id is derived from its owning note id. Recover a
        // missing note id first so every device bootstraps the exact same
        // version identity for the same legacy record.
        assign_missing_recovery_ids(&mut notes, "note", |note| &note.id, |note, id| note.id = id);
        let mut notes = notes
            .into_iter()
            .map(|note| note.sanitized(&valid_folder_ids, now))
            .collect::<Option<Vec<_>>>()?;
        notes = merge_duplicate_notes(notes)?;
        if !knowledge_hierarchy_valid(&notes) {
            return None;
        }
        for note in &mut notes {
            apply_current_attachment_tombstones(note, &tombstone_index, now);
            note.repair_version_stack(now);
        }
        notes.retain(|note| {
            record_is_newer_than_tombstone(
                &tombstone_index,
                TOMBSTONE_ENTITY_NOTE,
                &note.id,
                note_revision_epoch_millis(note),
            )
        });
        notes.sort_by(|left, right| {
            (
                left.deleted_at_epoch_millis.is_some(),
                !left.pinned,
                Reverse(note_revision_epoch_millis(left)),
                Reverse(left.created_at_epoch_millis),
            )
                .cmp(&(
                    right.deleted_at_epoch_millis.is_some(),
                    !right.pinned,
                    Reverse(note_revision_epoch_millis(right)),
                    Reverse(right.created_at_epoch_millis),
                ))
        });

        let mut note_preferences = self.note_preferences;
        note_preferences.selected_folder_id = note_preferences
            .selected_folder_id
            .take()
            .filter(|folder_id| !folder_id.is_empty());

        let finance_profile_updated_at_epoch_millis =
            sanitize_timestamp(self.finance_profile_updated_at_epoch_millis, now);
        let mut finance_profile = self.finance_profile.sanitized();
        let mut finance_day_ledger_revisions = BTreeMap::new();
        finance_profile.daily_ledgers.retain(|day_key, _| {
            let revision = self
                .finance_day_ledger_revisions
                .get(day_key)
                .copied()
                .map(|value| sanitize_timestamp(value, now))
                .unwrap_or(finance_profile_updated_at_epoch_millis);
            let keep = record_is_newer_than_tombstone(
                &tombstone_index,
                TOMBSTONE_ENTITY_FINANCE_DAY_LEDGER,
                day_key,
                revision,
            );
            if keep {
                finance_day_ledger_revisions.insert(day_key.clone(), revision);
            }
            keep
        });
        let mut finance_month_snapshot_revisions = BTreeMap::new();
        finance_profile.monthly_snapshots.retain(|month_key, _| {
            let revision = self
                .finance_month_snapshot_revisions
                .get(month_key)
                .copied()
                .map(|value| sanitize_timestamp(value, now))
                .unwrap_or(finance_profile_updated_at_epoch_millis);
            let keep = record_is_newer_than_tombstone(
                &tombstone_index,
                TOMBSTONE_ENTITY_FINANCE_MONTH_SNAPSHOT,
                month_key,
                revision,
            );
            if keep {
                finance_month_snapshot_revisions.insert(month_key.clone(), revision);
            }
            keep
        });

        let mut sync_conflict_history = std::mem::take(&mut self.sync_conflict_history)
            .into_iter()
            .map(|mut conflict| {
                conflict.losing_revision_epoch_millis =
                    sanitize_timestamp(conflict.losing_revision_epoch_millis, now);
                conflict.captured_at_epoch_millis =
                    sanitize_timestamp(conflict.captured_at_epoch_millis, now);
                conflict
            })
            .collect::<Vec<_>>();
        assign_missing_recovery_ids(
            &mut sync_conflict_history,
            "sync-conflict",
            |conflict| &conflict.id,
            |conflict, id| conflict.id = id,
        );
        sync_conflict_history.sort_by_key(|conflict| Reverse(conflict.captured_at_epoch_millis));
        sync_conflict_history.truncate(MAX_SYNC_CONFLICT_HISTORY);

        Some(Self {
            schema_version: APP_DATA_SCHEMA_VERSION,
            categories,
            slots,
            slot_order: normalize_slot_order(&self.slot_order),
            slot_order_updated_at_epoch_millis: sanitize_timestamp(
                self.slot_order_updated_at_epoch_millis,
                now,
            ),
            sessions,
            archived_tasks,
            note_folders,
            notes,
            note_preferences,
            note_preferences_updated_at_epoch_millis: sanitize_timestamp(
                self.note_preferences_updated_at_epoch_millis,
                now,
            ),
            finance_profile,
            finance_profile_updated_at_epoch_millis,
            finance_day_ledger_revisions,
            finance_month_snapshot_revisions,
            theme_mode: self.theme_mode,
            oled_theme_enabled: self.oled_theme_enabled,
            theme_mode_updated_at_epoch_millis: sanitize_timestamp(
                self.theme_mode_updated_at_epoch_millis,
                now,
            ),
            tombstones,
            sync_conflict_history,
        })
    }
}

impl TimerSlot {
    fn sanitized(mut self, _valid_category_ids: &HashSet<String>, now: i64) -> Self {
        let legacy_revision = sanitize_timestamp(self.updated_at, now);
        self.title_updated_at_epoch_millis =
            slot_field_revision(self.title_updated_at_epoch_millis, legacy_revision, now);
        self.category_updated_at_epoch_millis =
            slot_field_revision(self.category_updated_at_epoch_millis, legacy_revision, now);
        self.note_updated_at_epoch_millis =
            slot_field_revision(self.note_updated_at_epoch_millis, legacy_revision, now);
        self.accumulated_updated_at_epoch_millis = slot_field_revision(
            self.accumulated_updated_at_epoch_millis,
            legacy_revision,
            now,
        );
        self.running_updated_at_epoch_millis =
            slot_field_revision(self.running_updated_at_epoch_millis, legacy_revision, now);
        self.micro_break_updated_at_epoch_millis = slot_field_revision(
            self.micro_break_updated_at_epoch_millis,
            legacy_revision,
            now,
        );
        let cycle_index = self.micro_break_cycle_index.max(0);
        let phase_target = match self.micro_break_phase {
            MicroBreakPhase::Focus => compute_micro_break_target_millis(self.id, cycle_index),
            MicroBreakPhase::Break => MICRO_BREAK_REST_MILLIS,
        };
        self.accumulated_millis = sanitize_tracked_duration(self.accumulated_millis);
        self.running_since_epoch_millis = self.running_since_epoch_millis.map(|value| value.max(0));
        match self.running_since_epoch_millis {
            Some(running_since) if self.active_run_id.is_empty() => {
                self.active_run_id = deterministic_timer_run_id(self.id, running_since);
            }
            None => self.active_run_id.clear(),
            Some(_) => {}
        }
        self.micro_break_cycle_index = cycle_index;
        self.micro_break_phase_progress_millis =
            sanitize_tracked_duration(self.micro_break_phase_progress_millis)
                .clamp(0, phase_target);
        self.updated_at = legacy_revision
            .max(self.title_updated_at_epoch_millis)
            .max(self.category_updated_at_epoch_millis)
            .max(self.note_updated_at_epoch_millis)
            .max(self.accumulated_updated_at_epoch_millis)
            .max(self.running_updated_at_epoch_millis)
            .max(self.micro_break_updated_at_epoch_millis);
        self
    }

    fn is_blank_slate(&self) -> bool {
        self.title.trim().is_empty()
            && self.category_id.is_none()
            && self.note.trim().is_empty()
            && self.accumulated_millis == 0
            && self.running_since_epoch_millis.is_none()
            && self.micro_break_phase == MicroBreakPhase::Focus
            && self.micro_break_cycle_index == 0
            && self.micro_break_phase_progress_millis == 0
    }

    fn cleared_micro_break_tracking(mut self, updated_at: i64) -> Option<Self> {
        let running_revision =
            next_mutation_revision(self.running_updated_at_epoch_millis, updated_at)?;
        let micro_break_revision =
            next_mutation_revision(self.micro_break_updated_at_epoch_millis, updated_at)?;
        self.running_since_epoch_millis = None;
        self.active_run_id.clear();
        self.micro_break_phase = MicroBreakPhase::Focus;
        self.micro_break_cycle_index = 0;
        self.micro_break_phase_progress_millis = 0;
        self.updated_at = self
            .updated_at
            .max(running_revision)
            .max(micro_break_revision);
        self.running_updated_at_epoch_millis = running_revision;
        self.micro_break_updated_at_epoch_millis = micro_break_revision;
        Some(self)
    }

    fn as_blank_task(mut self, updated_at: i64) -> Option<Self> {
        let title_revision =
            next_mutation_revision(self.title_updated_at_epoch_millis, updated_at)?;
        let category_revision =
            next_mutation_revision(self.category_updated_at_epoch_millis, updated_at)?;
        let note_revision = next_mutation_revision(self.note_updated_at_epoch_millis, updated_at)?;
        let accumulated_revision =
            next_mutation_revision(self.accumulated_updated_at_epoch_millis, updated_at)?;
        self.title.clear();
        self.category_id = None;
        self.note.clear();
        self.accumulated_millis = 0;
        self.updated_at = self
            .updated_at
            .max(title_revision)
            .max(category_revision)
            .max(note_revision)
            .max(accumulated_revision);
        self.title_updated_at_epoch_millis = title_revision;
        self.category_updated_at_epoch_millis = category_revision;
        self.note_updated_at_epoch_millis = note_revision;
        self.accumulated_updated_at_epoch_millis = accumulated_revision;
        Some(self)
    }

    fn with_restored_archived_task(
        mut self,
        archived_task: &ArchivedTask,
        updated_at: i64,
    ) -> Option<Self> {
        let title_revision =
            next_mutation_revision(self.title_updated_at_epoch_millis, updated_at)?;
        let category_revision =
            next_mutation_revision(self.category_updated_at_epoch_millis, updated_at)?;
        let note_revision = next_mutation_revision(self.note_updated_at_epoch_millis, updated_at)?;
        let accumulated_revision =
            next_mutation_revision(self.accumulated_updated_at_epoch_millis, updated_at)?;
        self.title = archived_task.title.clone();
        self.category_id = archived_task.category_id.clone();
        self.note = archived_task.note.clone();
        self.accumulated_millis = archived_task.accumulated_millis;
        self.updated_at = self
            .updated_at
            .max(title_revision)
            .max(category_revision)
            .max(note_revision)
            .max(accumulated_revision);
        self.title_updated_at_epoch_millis = title_revision;
        self.category_updated_at_epoch_millis = category_revision;
        self.note_updated_at_epoch_millis = note_revision;
        self.accumulated_updated_at_epoch_millis = accumulated_revision;
        Some(self)
    }
}

impl TimerSession {
    fn sanitized(mut self, _valid_category_ids: &HashSet<String>, now: i64) -> Self {
        let started_at = sanitize_timestamp(self.started_at_epoch_millis, now);
        let ended_at = sanitize_timestamp(self.ended_at_epoch_millis, now);
        self.started_at_epoch_millis = started_at;
        self.ended_at_epoch_millis = ended_at;
        self.duration_millis = sanitize_tracked_duration(self.duration_millis);
        self.updated_at_epoch_millis = if self.updated_at_epoch_millis <= 0 {
            started_at.max(ended_at)
        } else {
            sanitize_timestamp(self.updated_at_epoch_millis, now)
        };
        self
    }
}

impl ArchivedTask {
    fn sanitized(mut self, _valid_category_ids: &HashSet<String>, now: i64) -> Self {
        self.accumulated_millis = sanitize_tracked_duration(self.accumulated_millis);
        self.archived_at_epoch_millis = sanitize_timestamp(self.archived_at_epoch_millis, now);
        self.updated_at_epoch_millis = if self.updated_at_epoch_millis <= 0 {
            self.archived_at_epoch_millis
        } else {
            sanitize_timestamp(self.updated_at_epoch_millis, now)
        };
        self
    }
}

include!("knowledge_app_data.rs");

impl NoteEntry {
    fn sanitized(mut self, _valid_folder_ids: &HashSet<String>, now: i64) -> Option<Self> {
        if self.encryption.is_none() && !knowledge_note_valid(&self) {
            return None;
        }
        // The envelope is the sole recoverable content for a protected note.
        // Fail closed even for an unknown/malformed envelope: never let a
        // mixed encrypted/plaintext record round-trip private fields.
        if let Some(encryption) = self.encryption.as_mut() {
            encryption.repair_legacy_omitted_metadata();
            if encryption.protection_revision <= 0 {
                return None;
            }
            if self.protection_state_revision == 0 {
                self.protection_state_revision = encryption.protection_revision;
            } else if self.protection_state_revision != encryption.protection_revision {
                // The top-level counter and envelope identify one indivisible
                // protection generation. Accepting a split-brain record here
                // would let later duplicate selection silently choose either
                // side of a password/protection transition.
                return None;
            }
            self.title.clear();
            self.content.clear();
            self.document = NoteDocument::default();
            self.attachments.clear();
            self.revisions.clear();
            self.versions.clear();
            self.latest_version_id.clear();
        } else if self.protection_state_revision < 0 {
            return None;
        }
        let created_at = sanitize_timestamp(self.created_at_epoch_millis, now);
        let mut updated_at = sanitize_timestamp(self.updated_at_epoch_millis, now);
        let deleted_at = self
            .deleted_at_epoch_millis
            .map(|value| sanitize_timestamp(value, now));
        updated_at = updated_at.max(deleted_at.unwrap_or(0));
        let mut attachments = std::mem::take(&mut self.attachments);
        assign_missing_recovery_ids(
            &mut attachments,
            "note-attachment",
            |attachment| &attachment.id,
            |attachment, id| attachment.id = id,
        );
        attachments = attachments
            .into_iter()
            .map(|attachment| attachment.sanitized(now))
            .collect();
        attachments = distinct_by_latest(
            attachments,
            |attachment| attachment.id.clone(),
            |attachment| attachment.updated_at_epoch_millis,
        );
        let attachment_index = attachments
            .iter()
            .map(|attachment| (attachment.id.clone(), attachment.clone()))
            .collect::<HashMap<_, _>>();
        let valid_attachment_ids = attachment_index.keys().cloned().collect::<HashSet<_>>();
        let mut revisions = std::mem::take(&mut self.revisions)
            .into_iter()
            .map(|revision| revision.sanitized(&attachment_index, now))
            .collect::<Vec<_>>();
        assign_missing_recovery_ids(
            &mut revisions,
            "note-revision",
            |revision| &revision.id,
            |revision, id| revision.id = id,
        );
        revisions = distinct_by_latest(
            revisions,
            |revision| revision.id.clone(),
            |revision| revision.updated_at_epoch_millis,
        );
        revisions.sort_by_key(|revision| Reverse(revision.captured_at_epoch_millis));

        let mut versions = std::mem::take(&mut self.versions)
            .into_iter()
            .map(|version| version.sanitized(&attachment_index, &self.id, now))
            .collect::<Vec<_>>();
        assign_missing_recovery_ids(
            &mut versions,
            "note-version",
            |version| &version.id,
            |version, id| version.id = id,
        );
        versions = distinct_by_latest(
            versions,
            |version| version.id.clone(),
            |version| version.updated_at_epoch_millis,
        );
        versions.sort_by_key(|version| (version.sequence, version.created_at_epoch_millis));

        self.document = std::mem::take(&mut self.document).sanitized(&valid_attachment_ids, now);
        self.accent_seed = sanitize_accent_seed(&self.accent_seed);
        self.attachments = attachments;
        self.revisions = revisions;
        self.versions = versions;
        self.created_at_epoch_millis = created_at;
        self.updated_at_epoch_millis = updated_at;
        self.deleted_at_epoch_millis = deleted_at;
        Some(self)
    }

    fn attachment_ids(&self) -> HashSet<String> {
        self.attachments
            .iter()
            .map(|attachment| attachment.id.clone())
            .collect()
    }

    fn resolved_document(&self) -> NoteDocument {
        if self.document.blocks.is_empty() && self.document.knowledge.is_none() {
            legacy_note_document(&self.content, &self.attachments)
        } else {
            self.document.clone()
        }
    }

    fn snapshot_for_history(
        &self,
        captured_at_epoch_millis: i64,
        updated_at_epoch_millis: i64,
    ) -> NoteRevisionSnapshot {
        let resolved_document = self.resolved_document();
        let snapshot_content = if self.document.blocks.is_empty() {
            self.content.clone()
        } else {
            resolved_document.storage_content()
        };
        // Product versions are complete snapshots. Preserve every current
        // attachment, including rich-text media not represented by IMAGE blocks.
        let attachment_ids = distinct_values(
            self.attachments
                .iter()
                .map(|attachment| attachment.id.clone())
                .chain(self.document_image_attachment_ids())
                .collect(),
        );
        let attachments = self.attachments.clone();
        NoteRevisionSnapshot {
            label: String::new(),
            id: revision_snapshot_id(
                &self.id,
                captured_at_epoch_millis,
                &self.updated_at_epoch_millis,
            ),
            title: self.title.clone(),
            content: snapshot_content,
            kind: self.kind.clone(),
            document: resolved_document,
            accent_seed: self.accent_seed.clone(),
            pinned: self.pinned,
            folder_id: self.folder_id.clone(),
            attachment_ids,
            attachments,
            captured_at_epoch_millis,
            updated_at_epoch_millis,
        }
    }

    fn snapshot_for_version(
        &self,
        id: String,
        sequence: i64,
        created_at_epoch_millis: i64,
        updated_at_epoch_millis: i64,
        is_latest: bool,
    ) -> NoteVersionSnapshot {
        let resolved_document = self.resolved_document();
        let snapshot_content = if self.document.blocks.is_empty() {
            self.content.clone()
        } else {
            resolved_document.storage_content()
        };
        // Product versions are complete snapshots. Preserve every current
        // attachment, including rich-text media not represented by IMAGE blocks.
        let attachment_ids = distinct_values(
            self.attachments
                .iter()
                .map(|attachment| attachment.id.clone())
                .chain(self.document_image_attachment_ids())
                .collect(),
        );
        let attachments = self.attachments.clone();
        NoteVersionSnapshot {
            id,
            note_id: self.id.clone(),
            sequence,
            title: self.title.clone(),
            content: snapshot_content,
            kind: self.kind.clone(),
            document: resolved_document,
            accent_seed: self.accent_seed.clone(),
            pinned: self.pinned,
            folder_id: self.folder_id.clone(),
            attachment_ids,
            attachments,
            created_at_epoch_millis,
            updated_at_epoch_millis,
            is_latest,
            deleted_at_epoch_millis: None,
        }
    }

    fn repair_version_stack(&mut self, now: i64) {
        if self.encryption.is_some() {
            self.versions.clear();
            self.latest_version_id.clear();
            return;
        }

        let attachment_index = self
            .attachments
            .iter()
            .map(|attachment| (attachment.id.clone(), attachment.clone()))
            .collect::<HashMap<_, _>>();
        let mut versions = std::mem::take(&mut self.versions)
            .into_iter()
            .map(|version| version.sanitized(&attachment_index, &self.id, now))
            .collect::<Vec<_>>();
        versions = distinct_note_versions(versions);

        let mut used_sequences = HashSet::new();
        let mut next_sequence = versions
            .iter()
            .map(|version| version.sequence.max(0))
            .max()
            .unwrap_or(0)
            .saturating_add(1)
            .max(1);
        versions.sort_by(|left, right| {
            (
                left.sequence.max(0),
                left.created_at_epoch_millis,
                left.id.as_str(),
            )
                .cmp(&(
                    right.sequence.max(0),
                    right.created_at_epoch_millis,
                    right.id.as_str(),
                ))
        });
        for version in &mut versions {
            if version.sequence <= 0 || !used_sequences.insert(version.sequence) {
                version.sequence = next_sequence;
                used_sequences.insert(next_sequence);
                next_sequence = next_sequence.saturating_add(1);
            }
        }

        let has_active_version = versions
            .iter()
            .any(|version| version.deleted_at_epoch_millis.is_none());
        if !has_active_version {
            let sequence = versions
                .iter()
                .map(|version| version.sequence.max(0))
                .max()
                .unwrap_or(0)
                .saturating_add(1)
                .max(1);
            let created_at = self.created_at_epoch_millis;
            let version_id = note_version_id(&self.id, sequence);
            versions.push(self.snapshot_for_version(
                version_id,
                sequence,
                created_at,
                self.updated_at_epoch_millis,
                true,
            ));
        } else {
            let latest_index = versions
                .iter()
                .enumerate()
                .filter(|(_, version)| version.deleted_at_epoch_millis.is_none())
                .max_by_key(|(_, version)| (version.sequence, version.created_at_epoch_millis))
                .map(|(index, _)| index)
                .unwrap_or(0);
            if !versions[latest_index].matches_note_content(self) {
                let latest = &versions[latest_index];
                let latest_id = latest.id.clone();
                let latest_sequence = latest.sequence;
                let latest_created_at = latest.created_at_epoch_millis;
                versions[latest_index] = self.snapshot_for_version(
                    latest_id,
                    latest_sequence,
                    latest_created_at,
                    self.updated_at_epoch_millis,
                    true,
                );
            }
        }

        versions.sort_by(|left, right| {
            (
                left.sequence,
                left.created_at_epoch_millis,
                left.id.as_str(),
            )
                .cmp(&(
                    right.sequence,
                    right.created_at_epoch_millis,
                    right.id.as_str(),
                ))
        });
        let latest_index = versions
            .iter()
            .enumerate()
            .filter(|(_, version)| version.deleted_at_epoch_millis.is_none())
            .max_by_key(|(_, version)| (version.sequence, version.created_at_epoch_millis))
            .map(|(index, _)| index)
            .unwrap_or(0);
        for (index, version) in versions.iter_mut().enumerate() {
            version.note_id = self.id.clone();
            version.is_latest = index == latest_index && version.deleted_at_epoch_millis.is_none();
        }
        self.latest_version_id = versions[latest_index].id.clone();
        self.versions = versions;
    }

    fn document_image_attachment_ids(&self) -> Vec<String> {
        let mut ids = self
            .resolved_document()
            .blocks
            .into_iter()
            .filter(|block| matches!(block.block_type, NoteBlockType::Image))
            .filter_map(|block| block.attachment_id)
            .collect::<Vec<_>>();
        ids = distinct_values(ids);
        ids
    }
}

impl NoteAttachment {
    fn sanitized(mut self, now: i64) -> Self {
        // fileName is a persisted media reference. Rewriting legal characters
        // (for example spaces or non-ASCII names) without renaming the file
        // disconnects metadata from its bytes, so preserve every non-empty
        // value exactly and apply path-safety only when resolving it.
        let safe_file_name = if self.file_name.is_empty() {
            sanitize_attachment_file_name("", &self.id)
        } else {
            self.file_name.clone()
        };
        self.file_name = safe_file_name;
        if self.mime_type.is_empty() {
            self.mime_type = default_image_mime();
        }
        self.width = self.width.max(0);
        self.height = self.height.max(0);
        self.size_bytes = self.size_bytes.max(0);
        self.sha256 = self.sha256.trim().to_ascii_lowercase();
        if self.sha256.len() != 64 || !self.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            self.sha256.clear();
        }
        self.created_at_epoch_millis = sanitize_timestamp(self.created_at_epoch_millis, now);
        self.updated_at_epoch_millis = if self.updated_at_epoch_millis == 0 {
            self.created_at_epoch_millis
        } else {
            sanitize_timestamp(self.updated_at_epoch_millis, now)
        };
        self
    }
}

impl NoteRevisionSnapshot {
    fn sanitized(
        mut self,
        current_attachment_index: &HashMap<String, NoteAttachment>,
        now: i64,
    ) -> Self {
        let attachment_ids = distinct_values(self.attachment_ids.clone());
        let mut attachments = std::mem::take(&mut self.attachments);
        for attachment_id in &attachment_ids {
            if attachments
                .iter()
                .all(|attachment| attachment.id != *attachment_id)
            {
                if let Some(attachment) = current_attachment_index.get(attachment_id) {
                    attachments.push(attachment.clone());
                }
            }
        }
        assign_missing_recovery_ids(
            &mut attachments,
            "note-attachment",
            |attachment| &attachment.id,
            |attachment, id| attachment.id = id,
        );
        attachments = attachments
            .into_iter()
            .map(|attachment| attachment.sanitized(now))
            .collect();
        attachments = distinct_by_latest(
            attachments,
            |attachment| attachment.id.clone(),
            |attachment| attachment.updated_at_epoch_millis,
        );
        let revision_attachment_ids = attachments
            .iter()
            .map(|attachment| attachment.id.clone())
            .chain(attachment_ids.iter().cloned())
            .collect::<HashSet<_>>();
        self.document = std::mem::take(&mut self.document).sanitized(&revision_attachment_ids, now);
        self.accent_seed = sanitize_accent_seed(&self.accent_seed);
        self.attachment_ids = attachment_ids;
        self.attachments = attachments;
        self.captured_at_epoch_millis = sanitize_timestamp(self.captured_at_epoch_millis, now);
        self.updated_at_epoch_millis = if self.updated_at_epoch_millis <= 0 {
            self.captured_at_epoch_millis
        } else {
            sanitize_timestamp(self.updated_at_epoch_millis, now)
        };
        self
    }

    fn matches_note(&self, note: &NoteEntry) -> bool {
        let resolved_document = note.resolved_document();
        self.title == note.title
            && self.content == note.content
            && self.kind == note.kind
            && self.document == resolved_document
            && self.accent_seed == note.accent_seed
            && self.pinned == note.pinned
            && self.folder_id.as_deref() == note.folder_id.as_deref()
            && self.attachment_ids == note.document_image_attachment_ids()
    }
}

impl NoteVersionSnapshot {
    fn sanitized(
        mut self,
        current_attachment_index: &HashMap<String, NoteAttachment>,
        note_id: &str,
        now: i64,
    ) -> Self {
        let attachment_ids = distinct_values(self.attachment_ids.clone());
        let mut attachments = std::mem::take(&mut self.attachments);
        for attachment_id in &attachment_ids {
            if attachments
                .iter()
                .all(|attachment| attachment.id != *attachment_id)
            {
                if let Some(attachment) = current_attachment_index.get(attachment_id) {
                    attachments.push(attachment.clone());
                }
            }
        }
        assign_missing_recovery_ids(
            &mut attachments,
            "note-attachment",
            |attachment| &attachment.id,
            |attachment, id| attachment.id = id,
        );
        attachments = attachments
            .into_iter()
            .map(|attachment| attachment.sanitized(now))
            .collect();
        attachments = distinct_by_latest(
            attachments,
            |attachment| attachment.id.clone(),
            |attachment| attachment.updated_at_epoch_millis,
        );
        let attachment_ids = distinct_values(
            attachment_ids
                .into_iter()
                .chain(attachments.iter().map(|attachment| attachment.id.clone()))
                .collect(),
        );
        let version_attachment_ids = attachments
            .iter()
            .map(|attachment| attachment.id.clone())
            .chain(attachment_ids.iter().cloned())
            .collect::<HashSet<_>>();
        self.note_id = note_id.to_string();
        self.sequence = self.sequence.max(0);
        self.document = std::mem::take(&mut self.document).sanitized(&version_attachment_ids, now);
        self.accent_seed = sanitize_accent_seed(&self.accent_seed);
        self.attachment_ids = attachment_ids;
        self.attachments = attachments;
        self.created_at_epoch_millis = sanitize_timestamp(self.created_at_epoch_millis, now);
        self.updated_at_epoch_millis = if self.updated_at_epoch_millis <= 0 {
            self.created_at_epoch_millis
        } else {
            sanitize_timestamp(self.updated_at_epoch_millis, now)
        };
        self.deleted_at_epoch_millis = self
            .deleted_at_epoch_millis
            .map(|value| sanitize_timestamp(value, now));
        self
    }

    fn matches_note_content(&self, note: &NoteEntry) -> bool {
        let resolved_document = note.resolved_document();
        let content = if note.document.blocks.is_empty() {
            note.content.clone()
        } else {
            resolved_document.storage_content()
        };
        let attachment_ids = distinct_values(
            note.attachments
                .iter()
                .map(|attachment| attachment.id.clone())
                .chain(note.document_image_attachment_ids())
                .collect(),
        );
        self.title == note.title
            && self.content == content
            && self.kind == note.kind
            && self.document == resolved_document
            && self.accent_seed == note.accent_seed
            && self.attachment_ids == attachment_ids
            && self.attachments == note.attachments
    }

    fn apply_to_note(&self, note: &NoteEntry, updated_at_epoch_millis: i64) -> NoteEntry {
        NoteEntry {
            title: self.title.clone(),
            content: self.content.clone(),
            kind: self.kind.clone(),
            document: self.document.clone(),
            accent_seed: self.accent_seed.clone(),
            attachments: self.attachments.clone(),
            updated_at_epoch_millis,
            deleted_at_epoch_millis: None,
            ..note.clone()
        }
    }
}

impl NoteDocument {
    fn sanitized(mut self, valid_attachment_ids: &HashSet<String>, now: i64) -> Self {
        let rich_text_enabled = self.rich_text_enabled;
        let mut blocks = std::mem::take(&mut self.blocks)
            .into_iter()
            .map(|block| {
                let mut sanitized = block.sanitized(valid_attachment_ids, now);
                if rich_text_enabled && matches!(sanitized.block_type, NoteBlockType::Text) {
                    if let Some(safe_html) = sanitize_rich_text_html_for_storage(&sanitized.text) {
                        sanitized.text = safe_html;
                    }
                }
                sanitized
            })
            .collect::<Vec<_>>();
        assign_missing_recovery_ids(
            &mut blocks,
            "note-block",
            |block| &block.id,
            |block, id| block.id = id,
        );
        self.blocks = blocks;
        self
    }

    fn storage_content(&self) -> String {
        build_note_document_text_digest(&self.to_text_input(), true, false, "").plain_text
    }

    fn is_blank(&self) -> bool {
        if self.knowledge.is_some() {
            return false;
        }
        if self.rich_text_enabled && !self.rich_text_plain_text.trim().is_empty() {
            return false;
        }
        self.blocks.is_empty() || self.blocks.iter().all(NoteBlock::is_blank)
    }

    fn to_text_input(&self) -> NoteDocumentTextInput {
        NoteDocumentTextInput {
            rich_text_enabled: self.rich_text_enabled,
            rich_text_plain_text: self.rich_text_plain_text.clone(),
            blocks: self.blocks.iter().map(NoteBlock::to_text_input).collect(),
        }
    }
}

impl NoteBlock {
    fn to_text_input(&self) -> NoteBlockTextInput {
        let type_code = match &self.block_type {
            NoteBlockType::Text => 0,
            NoteBlockType::Image => 1,
            NoteBlockType::Contact => 2,
            NoteBlockType::Call => 3,
        };
        NoteBlockTextInput {
            type_code,
            text: crate::knowledge::block_plaintext(self.knowledge.as_ref(), &self.text),
            caption: self.caption.clone(),
            contact_name: self.contact_name.clone(),
            contact_organization: self.contact_organization.clone(),
            first_contact_phone_number: self
                .contact_phones
                .first()
                .map(|phone| phone.number.clone())
                .unwrap_or_default(),
            contact_phone_search_text: self
                .contact_phones
                .iter()
                .map(|phone| format!("{} {}", phone.label, phone.number))
                .collect::<Vec<_>>()
                .join("\n"),
            call_contact_name: self.call_contact_name.clone(),
            call_phone_number: self.call_phone_number.clone(),
            call_direction_name: format!("{:?}", self.call_direction),
        }
    }

    fn sanitized(mut self, _valid_attachment_ids: &HashSet<String>, now: i64) -> Self {
        self.call_occurred_at_epoch_millis = self
            .call_occurred_at_epoch_millis
            .map(|value| sanitize_timestamp(value, now));
        self.call_duration_millis = self.call_duration_millis.map(|value| value.max(0));
        self
    }

    fn is_blank(&self) -> bool {
        if self.knowledge.is_some() {
            return false;
        }
        match self.block_type {
            NoteBlockType::Text => self.text.trim().is_empty(),
            NoteBlockType::Image => {
                self.attachment_id
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
                    && self.caption.trim().is_empty()
            }
            NoteBlockType::Contact => {
                self.contact_name.trim().is_empty()
                    && self.contact_organization.trim().is_empty()
                    && self.caption.trim().is_empty()
                    && self.contact_phones.iter().all(|phone| {
                        phone.label.trim().is_empty() && phone.number.trim().is_empty()
                    })
            }
            NoteBlockType::Call => {
                self.call_phone_number.trim().is_empty()
                    && self.call_contact_name.trim().is_empty()
                    && self.text.trim().is_empty()
                    && self.caption.trim().is_empty()
                    && self.call_direction == NoteCallDirection::Unknown
                    && self.call_occurred_at_epoch_millis.is_none()
                    && self.call_duration_millis.is_none()
            }
        }
    }
}

struct MicroBreakAdvance {
    phase: MicroBreakPhase,
    cycle_index: i32,
    phase_progress: i64,
    accumulated_millis: i64,
    active_segment_start: i64,
    safe_now: i64,
    latest_transition_at: Option<i64>,
    catch_up_compacted: bool,
    sessions: Vec<TimerSession>,
}

#[derive(Clone, Debug)]
struct FocusSessionAggregate {
    started_at_epoch_millis: i64,
    ended_at_epoch_millis: i64,
    duration_millis: i64,
}

struct FocusSessionAccumulator<'a> {
    slot: &'a TimerSlot,
    safe_now: i64,
    details: Vec<TimerSession>,
    aggregate: Option<FocusSessionAggregate>,
}

impl<'a> FocusSessionAccumulator<'a> {
    fn new(slot: &'a TimerSlot, safe_now: i64) -> Self {
        Self {
            slot,
            safe_now,
            details: Vec::with_capacity(MAX_MICRO_BREAK_DETAIL_SESSIONS),
            aggregate: None,
        }
    }

    fn push_detail(
        &mut self,
        cycle_index: i32,
        started_at_epoch_millis: i64,
        ended_at_epoch_millis: i64,
        duration_millis: i64,
    ) {
        if duration_millis <= 0 {
            return;
        }
        let session = TimerSession {
            id: active_run_focus_session_id(&self.slot.active_run_id, cycle_index),
            slot_id: self.slot.id,
            slot_title: timer_slot_history_title(self.slot),
            category_id: self.slot.category_id.clone(),
            started_at_epoch_millis,
            ended_at_epoch_millis,
            duration_millis,
            updated_at_epoch_millis: self.safe_now,
        };
        if cycle_index == i32::MAX {
            self.fold_session(session);
            return;
        }
        if self.details.len() == MAX_MICRO_BREAK_DETAIL_SESSIONS {
            let oldest = self.details.remove(0);
            self.fold_session(oldest);
        }
        self.details.push(session);
    }

    fn fold_span(
        &mut self,
        started_at_epoch_millis: i64,
        ended_at_epoch_millis: i64,
        duration_millis: i64,
    ) {
        if duration_millis <= 0 {
            return;
        }
        match &mut self.aggregate {
            Some(aggregate) => {
                aggregate.started_at_epoch_millis = aggregate
                    .started_at_epoch_millis
                    .min(started_at_epoch_millis);
                aggregate.ended_at_epoch_millis =
                    aggregate.ended_at_epoch_millis.max(ended_at_epoch_millis);
                aggregate.duration_millis =
                    aggregate.duration_millis.saturating_add(duration_millis);
            }
            None => {
                self.aggregate = Some(FocusSessionAggregate {
                    started_at_epoch_millis,
                    ended_at_epoch_millis,
                    duration_millis,
                });
            }
        }
    }

    fn fold_session(&mut self, session: TimerSession) {
        self.fold_span(
            session.started_at_epoch_millis,
            session.ended_at_epoch_millis,
            session.duration_millis,
        );
    }

    fn fold_all_details(&mut self) {
        for session in std::mem::take(&mut self.details) {
            self.fold_session(session);
        }
    }

    fn finish(self) -> Vec<TimerSession> {
        let mut sessions =
            Vec::with_capacity(self.details.len() + usize::from(self.aggregate.is_some()));
        if let Some(aggregate) = self.aggregate {
            sessions.push(TimerSession {
                id: format!(
                    "{}-focus-aggregate-{}-{}",
                    self.slot.active_run_id,
                    aggregate.started_at_epoch_millis,
                    aggregate.ended_at_epoch_millis
                ),
                slot_id: self.slot.id,
                slot_title: timer_slot_history_title(self.slot),
                category_id: self.slot.category_id.clone(),
                started_at_epoch_millis: aggregate.started_at_epoch_millis,
                ended_at_epoch_millis: aggregate.ended_at_epoch_millis,
                duration_millis: aggregate.duration_millis,
                updated_at_epoch_millis: self.safe_now,
            });
        }
        sessions.extend(self.details);
        sessions
    }
}

fn advance_micro_break_state(
    slot: &TimerSlot,
    running_since: i64,
    now: i64,
    collect_sessions: bool,
) -> MicroBreakAdvance {
    let safe_now = now.max(running_since);
    let mut phase = slot.micro_break_phase;
    let mut cycle_index = slot.micro_break_cycle_index.max(0);
    let mut phase_progress = slot.micro_break_phase_progress_millis.max(0);
    let mut accumulated_millis = sanitize_tracked_duration(slot.accumulated_millis);
    let mut active_segment_start = running_since;
    let mut remaining_elapsed = safe_now.saturating_sub(running_since).max(0);
    let mut latest_transition_at = None;
    let mut catch_up_compacted = false;
    let mut exact_transitions = 0usize;
    let mut sessions = FocusSessionAccumulator::new(slot, safe_now);

    loop {
        let phase_target = micro_break_phase_target_millis(slot.id, &phase, cycle_index);
        phase_progress = phase_progress.clamp(0, phase_target);
        let remaining_in_phase = phase_target.saturating_sub(phase_progress);

        if remaining_in_phase == 0 {
            if phase == MicroBreakPhase::Focus {
                phase = MicroBreakPhase::Break;
            } else {
                phase = MicroBreakPhase::Focus;
                cycle_index = cycle_index.saturating_add(1);
            }
            phase_progress = 0;
            latest_transition_at = Some(active_segment_start);
            exact_transitions = exact_transitions.saturating_add(1);
            continue;
        }

        if remaining_elapsed < remaining_in_phase {
            break;
        }

        // Normal sleep/wake spans retain the historical per-phase semantics.
        // Once the fixed work budget is exhausted, skip complete cycles using
        // one deterministic target. This keeps adversarial i64 spans bounded;
        // the resulting aggregate is explicitly marked in TimerView.
        let should_compact = phase == MicroBreakPhase::Focus
            && phase_progress == 0
            && (exact_transitions >= MAX_EXACT_MICRO_BREAK_PHASE_TRANSITIONS
                || cycle_index == i32::MAX);
        if should_compact {
            let focus_target = compute_micro_break_target_millis(slot.id, cycle_index);
            let full_cycle_millis = focus_target.saturating_add(MICRO_BREAK_REST_MILLIS);
            let full_cycles = remaining_elapsed / full_cycle_millis;
            if full_cycles > 0 {
                catch_up_compacted = true;
                if collect_sessions {
                    sessions.fold_all_details();
                }
                let consumed_millis = full_cycles.saturating_mul(full_cycle_millis);
                let focus_duration = full_cycles.saturating_mul(focus_target);
                let last_focus_offset = full_cycles
                    .saturating_sub(1)
                    .saturating_mul(full_cycle_millis)
                    .saturating_add(focus_target);
                let last_focus_end = active_segment_start.saturating_add(last_focus_offset);
                if collect_sessions {
                    sessions.fold_span(active_segment_start, last_focus_end, focus_duration);
                }
                accumulated_millis = accumulated_millis.saturating_add(focus_duration);
                cycle_index = saturating_add_micro_break_cycles(cycle_index, full_cycles);
                active_segment_start = active_segment_start.saturating_add(consumed_millis);
                remaining_elapsed = remaining_elapsed.saturating_sub(consumed_millis);
                latest_transition_at = Some(active_segment_start);
                continue;
            }
        }

        let transition_at = active_segment_start.saturating_add(remaining_in_phase);
        if phase == MicroBreakPhase::Focus {
            accumulated_millis = accumulated_millis.saturating_add(remaining_in_phase);
            if collect_sessions {
                sessions.push_detail(
                    cycle_index,
                    active_segment_start,
                    transition_at,
                    remaining_in_phase,
                );
            }
            phase = MicroBreakPhase::Break;
        } else {
            phase = MicroBreakPhase::Focus;
            cycle_index = cycle_index.saturating_add(1);
        }

        latest_transition_at = Some(transition_at);
        phase_progress = 0;
        active_segment_start = transition_at;
        remaining_elapsed = remaining_elapsed.saturating_sub(remaining_in_phase);
        exact_transitions = exact_transitions.saturating_add(1);
    }

    MicroBreakAdvance {
        phase,
        cycle_index,
        phase_progress,
        accumulated_millis: sanitize_tracked_duration(accumulated_millis),
        active_segment_start,
        safe_now,
        latest_transition_at,
        catch_up_compacted,
        sessions: sessions.finish(),
    }
}

fn saturating_add_micro_break_cycles(cycle_index: i32, additional_cycles: i64) -> i32 {
    let available = i64::from(i32::MAX.saturating_sub(cycle_index.max(0)));
    let applied = additional_cycles.max(0).min(available);
    cycle_index.max(0).saturating_add(applied as i32)
}

fn resolve_micro_breaks_for_app_data(mut data: AppData, now: i64) -> Option<AppData> {
    let valid_category_ids = data
        .categories
        .iter()
        .map(|category| category.id.clone())
        .collect::<HashSet<_>>();
    let mut generated_sessions = Vec::<TimerSession>::new();
    let mut resolved_slots = Vec::with_capacity(data.slots.len());
    for slot in data.slots {
        let (resolved_slot, sessions) =
            resolve_slot_micro_break(slot.sanitized(&valid_category_ids, now), now)?;
        resolved_slots.push(resolved_slot);
        generated_sessions.extend(sessions);
    }
    generated_sessions.sort_by_key(|session| Reverse(session.ended_at_epoch_millis));
    if generated_sessions.is_empty() {
        data.slots = resolved_slots;
    } else {
        let mut sessions = generated_sessions;
        sessions.extend(data.sessions);
        data.slots = resolved_slots;
        data.sessions = sessions;
    }
    Some(data)
}

fn resolve_slot_micro_break(
    mut slot: TimerSlot,
    now: i64,
) -> Option<(TimerSlot, Vec<TimerSession>)> {
    let Some(running_since) = slot.running_since_epoch_millis else {
        return Some((slot, Vec::new()));
    };
    let original_accumulated_millis = slot.accumulated_millis;
    let original_running_since = slot.running_since_epoch_millis;
    let original_micro_break_phase = slot.micro_break_phase;
    let original_micro_break_cycle_index = slot.micro_break_cycle_index;
    let original_micro_break_phase_progress_millis = slot.micro_break_phase_progress_millis;
    let advanced = advance_micro_break_state(&slot, running_since, now, true);

    slot.accumulated_millis = advanced.accumulated_millis;
    slot.running_since_epoch_millis = Some(advanced.active_segment_start);
    slot.micro_break_phase = advanced.phase;
    slot.micro_break_cycle_index = advanced.cycle_index;
    slot.micro_break_phase_progress_millis = advanced.phase_progress;
    if let Some(transition_at) = advanced.latest_transition_at {
        if slot.accumulated_millis != original_accumulated_millis {
            slot.accumulated_updated_at_epoch_millis =
                next_mutation_revision(slot.accumulated_updated_at_epoch_millis, transition_at)?;
        }
        if slot.running_since_epoch_millis != original_running_since {
            slot.running_updated_at_epoch_millis =
                next_mutation_revision(slot.running_updated_at_epoch_millis, transition_at)?;
        }
        if slot.micro_break_phase != original_micro_break_phase
            || slot.micro_break_cycle_index != original_micro_break_cycle_index
            || slot.micro_break_phase_progress_millis != original_micro_break_phase_progress_millis
        {
            slot.micro_break_updated_at_epoch_millis =
                next_mutation_revision(slot.micro_break_updated_at_epoch_millis, transition_at)?;
        }
        slot.updated_at = slot
            .updated_at
            .max(slot.accumulated_updated_at_epoch_millis)
            .max(slot.running_updated_at_epoch_millis)
            .max(slot.micro_break_updated_at_epoch_millis);
    }
    Some((slot, advanced.sessions))
}

fn pause_slot_micro_break(
    mut slot: TimerSlot,
    now: i64,
) -> Option<(TimerSlot, Option<TimerSession>)> {
    let Some(running_since) = slot.running_since_epoch_millis else {
        return Some((slot, None));
    };
    let elapsed = safe_elapsed_since(running_since, now);
    if slot.micro_break_phase == MicroBreakPhase::Focus {
        let phase_target = micro_break_phase_target_millis(
            slot.id,
            &slot.micro_break_phase,
            slot.micro_break_cycle_index,
        );
        let duration = elapsed.max(0);
        let session = (duration > 0).then(|| TimerSession {
            id: active_run_focus_session_id(&slot.active_run_id, slot.micro_break_cycle_index),
            slot_id: slot.id,
            slot_title: timer_slot_history_title(&slot),
            category_id: slot.category_id.clone(),
            started_at_epoch_millis: running_since,
            ended_at_epoch_millis: now,
            duration_millis: duration,
            updated_at_epoch_millis: now.max(running_since),
        });
        slot.accumulated_millis =
            sanitize_tracked_duration(slot.accumulated_millis.saturating_add(duration));
        slot.running_since_epoch_millis = None;
        slot.active_run_id.clear();
        slot.micro_break_phase_progress_millis = slot
            .micro_break_phase_progress_millis
            .saturating_add(duration)
            .clamp(0, phase_target);
        let accumulated_revision =
            next_mutation_revision(slot.accumulated_updated_at_epoch_millis, now)?;
        let running_revision = next_mutation_revision(slot.running_updated_at_epoch_millis, now)?;
        let micro_break_revision =
            next_mutation_revision(slot.micro_break_updated_at_epoch_millis, now)?;
        slot.updated_at = slot
            .updated_at
            .max(accumulated_revision)
            .max(running_revision)
            .max(micro_break_revision);
        slot.accumulated_updated_at_epoch_millis = accumulated_revision;
        slot.running_updated_at_epoch_millis = running_revision;
        slot.micro_break_updated_at_epoch_millis = micro_break_revision;
        Some((slot, session))
    } else {
        slot.running_since_epoch_millis = None;
        slot.active_run_id.clear();
        slot.micro_break_phase_progress_millis = slot
            .micro_break_phase_progress_millis
            .saturating_add(elapsed)
            .clamp(0, MICRO_BREAK_REST_MILLIS);
        let running_revision = next_mutation_revision(slot.running_updated_at_epoch_millis, now)?;
        let micro_break_revision =
            next_mutation_revision(slot.micro_break_updated_at_epoch_millis, now)?;
        slot.updated_at = slot
            .updated_at
            .max(running_revision)
            .max(micro_break_revision);
        slot.running_updated_at_epoch_millis = running_revision;
        slot.micro_break_updated_at_epoch_millis = micro_break_revision;
        Some((slot, None))
    }
}

fn micro_break_phase_target_millis(slot_id: i32, phase: &MicroBreakPhase, cycle_index: i32) -> i64 {
    match phase {
        MicroBreakPhase::Focus => compute_micro_break_target_millis(slot_id, cycle_index),
        MicroBreakPhase::Break => MICRO_BREAK_REST_MILLIS,
    }
}

fn safe_elapsed_since(started_at_epoch_millis: i64, now: i64) -> i64 {
    if started_at_epoch_millis < 0 || started_at_epoch_millis > now {
        0
    } else {
        now - started_at_epoch_millis
    }
}

fn deterministic_timer_run_id(slot_id: i32, running_since_epoch_millis: i64) -> String {
    let input = format!("gridtimer-run-v1\0{slot_id}\0{running_since_epoch_millis}");
    let mut digest = Sha256::new();
    digest.update(input.as_bytes());
    let hex = format!("{:x}", digest.finalize());
    format!("run-{}", &hex[..32])
}

fn active_run_focus_session_id(active_run_id: &str, cycle_index: i32) -> String {
    format!("{active_run_id}-focus-{}", cycle_index.max(0))
}

fn timer_slot_history_title(slot: &TimerSlot) -> String {
    if slot.title.trim().is_empty() {
        format!("任务 {:02}", slot.id)
    } else {
        slot.title.clone()
    }
}

fn normalize_slot_order(slot_order: &[i32]) -> Vec<i32> {
    let mut seen = HashSet::<i32>::new();
    let mut normalized = Vec::new();
    for slot_id in slot_order {
        if (1..=DEFAULT_SLOT_COUNT).contains(slot_id) && seen.insert(*slot_id) {
            normalized.push(*slot_id);
        }
    }
    for slot_id in 1..=DEFAULT_SLOT_COUNT {
        if seen.insert(slot_id) {
            normalized.push(slot_id);
        }
    }
    normalized
}

fn sanitize_timestamp(value: i64, _now: i64) -> i64 {
    // Persisted event clocks from another device may legitimately be ahead of
    // this machine. Clamping them to local `now` collapses ordering and can
    // make a newer record lose to an older tombstone.
    value.max(0)
}

fn sanitize_tracked_duration(value: i64) -> i64 {
    value.max(0)
}

fn slot_field_revision(value: i64, legacy_revision: i64, now: i64) -> i64 {
    if value == 0 {
        legacy_revision
    } else {
        sanitize_timestamp(value, now)
    }
}

fn compute_micro_break_target_millis(slot_id: i32, cycle_index: i32) -> i64 {
    let mixed = mix_micro_break_seed(slot_id, cycle_index);
    let variant_index = (mixed % MICRO_BREAK_FOCUS_VARIANT_COUNT as u64) as i64;
    MICRO_BREAK_FOCUS_MIN_MILLIS + (variant_index * MICRO_BREAK_FOCUS_STEP_MILLIS)
}

fn mix_micro_break_seed(slot_id: i32, cycle_index: i32) -> u64 {
    let mut value = ((slot_id as i64 as u64) << 32) ^ (cycle_index as i64 as u64);
    value ^= value >> 33;
    value = value.wrapping_mul(0xff51afd7ed558ccd);
    value ^= value >> 33;
    value = value.wrapping_mul(0xc4ceb9fe1a85ec53);
    value ^= value >> 33;
    value & 0x7fff_ffff_ffff_ffff
}

pub(crate) fn upsert_data_tombstone(
    tombstones: &mut Vec<DataTombstone>,
    entity_type: &str,
    entity_id: &str,
    deleted_at_epoch_millis: i64,
) {
    if entity_type.is_empty() || entity_id.is_empty() {
        return;
    }
    if let Some(existing) = tombstones
        .iter_mut()
        .find(|tombstone| tombstone.entity_type == entity_type && tombstone.entity_id == entity_id)
    {
        existing.deleted_at_epoch_millis = existing
            .deleted_at_epoch_millis
            .max(deleted_at_epoch_millis);
    } else {
        tombstones.push(DataTombstone {
            entity_type: entity_type.to_string(),
            entity_id: entity_id.to_string(),
            deleted_at_epoch_millis,
        });
    }
}

pub(crate) fn latest_data_tombstone_deleted_at(
    tombstones: &[DataTombstone],
    entity_type: &str,
    entity_id: &str,
) -> Option<i64> {
    tombstones
        .iter()
        .filter(|tombstone| {
            tombstone.entity_type == entity_type && tombstone.entity_id == entity_id
        })
        .map(|tombstone| tombstone.deleted_at_epoch_millis)
        .max()
}

fn next_entity_mutation_revision(
    tombstones: &[DataTombstone],
    entity_type: &str,
    entity_id: &str,
    existing_record_revision: i64,
    now: i64,
) -> Option<i64> {
    let previous_revision = latest_data_tombstone_deleted_at(tombstones, entity_type, entity_id)
        .unwrap_or(0)
        .max(existing_record_revision.max(0));
    Some(previous_revision.checked_add(1)?.max(now.max(0)))
}

fn next_mutation_revision(existing_revision: i64, now: i64) -> Option<i64> {
    Some(existing_revision.max(0).checked_add(1)?.max(now.max(0)))
}

fn next_deletion_revision(
    tombstones: &[DataTombstone],
    entity_type: &str,
    entity_id: &str,
    existing_record_revision: i64,
    now: i64,
) -> Option<i64> {
    let previous_revision = latest_data_tombstone_deleted_at(tombstones, entity_type, entity_id)
        .unwrap_or(0)
        .max(existing_record_revision.max(0));
    Some(previous_revision.checked_add(1)?.max(now.max(0)))
}

pub(crate) fn normalize_data_tombstones(
    tombstones: Vec<DataTombstone>,
    now: i64,
) -> Vec<DataTombstone> {
    let mut normalized = tombstones
        .into_iter()
        .filter_map(|mut tombstone| {
            if tombstone.entity_type.is_empty() || tombstone.entity_id.is_empty() {
                return None;
            }
            tombstone.deleted_at_epoch_millis =
                sanitize_timestamp(tombstone.deleted_at_epoch_millis, now);
            Some(tombstone)
        })
        .collect::<Vec<_>>();
    normalized = distinct_by_latest(
        normalized,
        |tombstone| (tombstone.entity_type.clone(), tombstone.entity_id.clone()),
        |tombstone| tombstone.deleted_at_epoch_millis,
    );
    normalized.sort_by(|left, right| {
        Reverse(left.deleted_at_epoch_millis)
            .cmp(&Reverse(right.deleted_at_epoch_millis))
            .then_with(|| left.entity_type.cmp(&right.entity_type))
            .then_with(|| left.entity_id.cmp(&right.entity_id))
    });
    // A tombstone is causal state, not disposable history. Without an
    // acknowledgement from every replica that could still reconnect, dropping
    // even the oldest tombstone lets that replica resurrect a deleted entity.
    // The account snapshot byte quota remains the fail-closed storage bound.
    normalized
}

fn record_is_newer_than_tombstone(
    tombstone_index: &HashMap<String, HashMap<String, i64>>,
    entity_type: &str,
    entity_id: &str,
    record_updated_at_epoch_millis: i64,
) -> bool {
    match tombstone_index
        .get(entity_type)
        .and_then(|entities| entities.get(entity_id))
    {
        Some(deleted_at) => record_updated_at_epoch_millis > *deleted_at,
        None => true,
    }
}

fn sanitize_attachment_file_name(value: &str, fallback_id: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        format!("{fallback_id}.jpg")
    } else {
        sanitized
    }
}

fn sanitize_accent_seed(value: &str) -> String {
    match value {
        "red" | "blue" | "green" | "amber" | "teal" => value.to_string(),
        _ => default_amber(),
    }
}

fn accent_seed_for_category_index(index: usize) -> String {
    match index % 5 {
        0 => "red".to_string(),
        1 => "blue".to_string(),
        2 => "green".to_string(),
        3 => "amber".to_string(),
        _ => "teal".to_string(),
    }
}

fn restore_target_slot_id_for_task(slots: &[TimerSlot], original_slot_id: i32) -> Option<i32> {
    slots
        .iter()
        .find(|slot| slot.id == original_slot_id && slot.is_blank_slate())
        .map(|slot| slot.id)
        .or_else(|| {
            slots
                .iter()
                .find(|slot| slot.is_blank_slate())
                .map(|slot| slot.id)
        })
}

fn revision_snapshot_id(
    note_id: &str,
    captured_at_epoch_millis: i64,
    updated_at_epoch_millis: &i64,
) -> String {
    let mut hasher = DefaultHasher::new();
    note_id.hash(&mut hasher);
    captured_at_epoch_millis.hash(&mut hasher);
    updated_at_epoch_millis.hash(&mut hasher);
    format!(
        "revision-{}-{:016x}",
        captured_at_epoch_millis.max(0),
        hasher.finish()
    )
}

fn note_version_id(note_id: &str, sequence: i64) -> String {
    format!("{}:version:{}", note_id, sequence.max(1))
}

fn requested_note_version_id(_note_id: &str, request_id: &str) -> String {
    // The caller generates one UUID-like value per tap and uses it both as the
    // product version id and the idempotency key. Returning that exact value
    // keeps the native result and the UI fallback on one stable contract.
    request_id.trim().to_string()
}

fn append_explicit_note_version(
    data: &AppData,
    mut note: NoteEntry,
    source_version_id: &str,
    expected_latest_version_id: &str,
    request_id: &str,
    now: i64,
) -> Option<NoteEntry> {
    let requested_version_id = requested_note_version_id(&note.id, request_id);
    if note
        .versions
        .iter()
        .any(|version| version.id == requested_version_id)
    {
        return Some(note);
    }
    if note.latest_version_id != expected_latest_version_id {
        return None;
    }

    let source_id = if source_version_id.trim().is_empty() {
        note.latest_version_id.as_str()
    } else {
        source_version_id.trim()
    };
    let source = note
        .versions
        .iter()
        .find(|version| version.id == source_id && version.deleted_at_epoch_millis.is_none())
        .cloned()?;
    let next_sequence = note
        .versions
        .iter()
        .map(|version| version.sequence.max(0))
        .max()
        .unwrap_or(0)
        .checked_add(1)?
        .max(1);
    let mutation_revision = note.updated_at_epoch_millis;
    let mut next = source;
    for attachment in &mut next.attachments {
        let suppressing_revision = [
            TOMBSTONE_ENTITY_NOTE_ATTACHMENT,
            TOMBSTONE_ENTITY_NOTE_MEDIA,
        ]
        .into_iter()
        .filter_map(|entity_type| {
            latest_data_tombstone_deleted_at(&data.tombstones, entity_type, &attachment.id)
        })
        .max()
        .unwrap_or(0);
        if attachment.updated_at_epoch_millis <= suppressing_revision {
            attachment.updated_at_epoch_millis =
                suppressing_revision.checked_add(1)?.max(now.max(0));
        }
    }
    next.id = requested_version_id;
    next.note_id = note.id.clone();
    next.sequence = next_sequence;
    next.created_at_epoch_millis = now.max(0);
    next.updated_at_epoch_millis = mutation_revision;
    next.is_latest = true;
    next.deleted_at_epoch_millis = None;

    note = next.apply_to_note(&note, mutation_revision);
    for version in &mut note.versions {
        version.is_latest = false;
    }
    note.latest_version_id = next.id.clone();
    note.versions.push(next);
    Some(note)
}

fn normalize_note_for_save(
    data: &AppData,
    mut note: NoteEntry,
    existing: Option<&NoteEntry>,
    timestamp: i64,
) -> Option<NoteEntry> {
    note.protection_state_revision = validate_note_protection_state_transition(existing, &note)?;
    if !knowledge_save_allowed(data, existing, &note) {
        return None;
    }
    let existing_revision = existing
        .map(note_revision_epoch_millis)
        .unwrap_or(0)
        .max(note_revision_epoch_millis(&note));
    let mutation_revision = next_entity_mutation_revision(
        &data.tombstones,
        TOMBSTONE_ENTITY_NOTE,
        &note.id,
        existing_revision,
        timestamp,
    )?;
    let is_encrypted = note.encryption.is_some();
    let crossed_encryption_boundary = existing
        .map(|previous| previous.encryption.is_some() != is_encrypted)
        .unwrap_or(false);
    if !is_encrypted && !crossed_encryption_boundary {
        if let Some(previous) = existing {
            // A regular save may update only the current version's content.
            // Never trust a stale editor copy to rewrite the saved lineage.
            note.versions = previous.versions.clone();
            note.latest_version_id = previous.latest_version_id.clone();
        }
    }
    let merged_attachments = if is_encrypted || crossed_encryption_boundary {
        note.attachments.clone()
    } else {
        merge_note_attachments(
            existing
                .map(|note| note.attachments.clone())
                .unwrap_or_default(),
            note.attachments.clone(),
        )
    };
    if note.revisions.is_empty() && !is_encrypted && !crossed_encryption_boundary {
        note.revisions = existing
            .map(|note| note.revisions.clone())
            .unwrap_or_default();
    }
    note.attachments = merged_attachments;
    let document_was_explicit =
        !note.document.blocks.is_empty() || note.document.knowledge.is_some();
    let resolved_document = note.resolved_document();
    let created_at = existing
        .map(|note| note.created_at_epoch_millis)
        .unwrap_or_else(|| {
            if note.created_at_epoch_millis > 0 {
                note.created_at_epoch_millis
            } else {
                mutation_revision
            }
        });
    let deleted_at = note
        .deleted_at_epoch_millis
        .or_else(|| existing.and_then(|note| note.deleted_at_epoch_millis));
    if is_encrypted {
        // Treat the envelope as one opaque value. Its decrypted children must
        // never be merged into, or snapshotted beside, the protected record.
        note.title.clear();
        note.content.clear();
        note.document = NoteDocument::default();
        note.attachments.clear();
        note.revisions.clear();
        note.versions.clear();
        note.latest_version_id.clear();
        note.created_at_epoch_millis = created_at;
        note.updated_at_epoch_millis = mutation_revision;
        note.deleted_at_epoch_millis = deleted_at;
        return Some(note);
    }
    if document_was_explicit {
        note.content = resolved_document.storage_content();
    }
    note.document = resolved_document;
    note.created_at_epoch_millis = created_at;
    note.updated_at_epoch_millis = mutation_revision;
    note.deleted_at_epoch_millis = deleted_at;
    note.revisions = build_note_revisions(existing, &note, timestamp, mutation_revision);
    Some(note)
}

fn merge_note_attachments(
    existing: Vec<NoteAttachment>,
    incoming: Vec<NoteAttachment>,
) -> Vec<NoteAttachment> {
    if existing.is_empty() {
        return incoming;
    }
    if incoming.is_empty() {
        return existing;
    }
    let mut merged = incoming;
    let incoming_ids = merged
        .iter()
        .map(|attachment| attachment.id.clone())
        .collect::<HashSet<_>>();
    merged.extend(
        existing
            .into_iter()
            .filter(|attachment| !incoming_ids.contains(&attachment.id)),
    );
    merged
}

fn build_note_revisions(
    previous: Option<&NoteEntry>,
    next: &NoteEntry,
    timestamp: i64,
    updated_at_epoch_millis: i64,
) -> Vec<NoteRevisionSnapshot> {
    let should_capture = previous
        .map(|previous| should_capture_revision(previous, next, timestamp))
        .unwrap_or(false);
    let snapshot = previous
        .filter(|_| should_capture)
        .map(|previous| previous.snapshot_for_history(timestamp.max(0), updated_at_epoch_millis));
    let mut base = next.revisions.clone();
    base = distinct_by_latest(
        base,
        |revision| revision.id.clone(),
        |revision| revision.updated_at_epoch_millis,
    );
    base.sort_by_key(|revision| Reverse(revision.captured_at_epoch_millis));
    if let Some(snapshot) = snapshot {
        std::iter::once(snapshot.clone())
            .chain(base.into_iter().filter(|revision| {
                revision.id != snapshot.id
                    && previous
                        .map(|previous| {
                            !revision.label.is_empty() || !revision.matches_note(previous)
                        })
                        .unwrap_or(true)
            }))
            .collect()
    } else {
        base
    }
}

fn should_capture_revision(previous: &NoteEntry, next: &NoteEntry, timestamp: i64) -> bool {
    if previous.encryption.is_some() || next.encryption.is_some() {
        return false;
    }
    let previous_document = previous.resolved_document();
    let next_document = next.resolved_document();
    let attachment_ids_changed =
        previous.document_image_attachment_ids() != next.document_image_attachment_ids();
    let structure_changed = previous.title.trim() != next.title.trim()
        || previous_document != next_document
        || previous.accent_seed != next.accent_seed
        || previous.pinned != next.pinned
        || previous.folder_id != next.folder_id
        || attachment_ids_changed;
    let metadata_changed = previous.accent_seed != next.accent_seed
        || previous.pinned != next.pinned
        || previous.folder_id != next.folder_id
        || attachment_ids_changed;
    if !structure_changed {
        return false;
    }
    let last_captured_at = previous
        .revisions
        .iter()
        .map(|revision| revision.captured_at_epoch_millis)
        .max();
    match last_captured_at {
        None => true,
        Some(value) if timestamp.saturating_sub(value) >= 45_000 => true,
        Some(_) => metadata_changed,
    }
}

fn normalize_newlines(value: &str) -> String {
    value.replace("\r\n", "\n")
}

fn legacy_note_document(content: &str, attachments: &[NoteAttachment]) -> NoteDocument {
    let safe_content = normalize_newlines(content).trim().to_string();
    let mut blocks = Vec::new();
    if !safe_content.is_empty() {
        blocks.push(NoteBlock {
            id: "legacy-text".to_string(),
            block_type: NoteBlockType::Text,
            text: safe_content,
            ..NoteBlock::default()
        });
    }
    for attachment in attachments {
        blocks.push(NoteBlock {
            id: format!("legacy-image-{}", attachment.id),
            block_type: NoteBlockType::Image,
            attachment_id: Some(attachment.id.clone()),
            caption: attachment.display_name.clone(),
            ..NoteBlock::default()
        });
    }
    let normalized_content = normalize_newlines(content);
    NoteDocument {
        markdown_enabled: normalized_content.lines().any(is_markdown_note_line),
        blocks,
        ..NoteDocument::default()
    }
}

fn is_markdown_note_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with("# ")
        || trimmed.starts_with("> ")
        || trimmed.starts_with("- [ ] ")
        || trimmed.starts_with("- ")
        || looks_like_ordered_markdown_line(trimmed)
        || looks_like_center_markdown_line(trimmed)
        || trimmed.contains("**")
        || trimmed.contains("__")
}

fn looks_like_ordered_markdown_line(value: &str) -> bool {
    let mut chars = value.chars().peekable();
    let mut digit_count = 0usize;
    while chars.peek().is_some_and(|ch| ch.is_ascii_digit()) {
        digit_count += 1;
        chars.next();
    }
    digit_count > 0
        && chars.next() == Some('.')
        && chars.next().is_some_and(|ch| ch.is_whitespace())
}

fn looks_like_center_markdown_line(value: &str) -> bool {
    let plain_bold = value.replace("**", "").replace("__", "");
    plain_bold.starts_with('[') && plain_bold.ends_with(']') && plain_bold.len() > 2
}

fn remove_rich_text_attachment_reference_for_app_data(html: &str, attachment_id: &str) -> String {
    if html.trim().is_empty() || attachment_id.trim().is_empty() {
        return html.to_string();
    }
    let without_attachment = remove_matching_image_tags_for_app_data(
        &remove_matching_figure_blocks_for_app_data(html, attachment_id),
        attachment_id,
    );
    // Global deletion markers visit every note. Unrelated text, including its
    // leading spaces and intentional blank paragraphs, must remain unchanged.
    if without_attachment == html {
        return without_attachment;
    }
    collapse_repeated_blank_paragraphs_for_app_data(&without_attachment)
        .trim()
        .to_string()
}

fn remove_matching_figure_blocks_for_app_data(html: &str, attachment_id: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut output = String::with_capacity(html.len());
    let mut cursor = 0usize;
    while let Some(relative_start) = lower[cursor..].find("<figure") {
        let figure_start = cursor + relative_start;
        let Some(open_end) = html[figure_start..]
            .find('>')
            .map(|offset| figure_start + offset + 1)
        else {
            break;
        };
        let opening_tag = &html[figure_start..open_end];
        let matches_attachment =
            first_quoted_attribute_value_for_app_data(opening_tag, "data-note-image")
                .map(|value| value.trim() == attachment_id)
                .unwrap_or(false);
        if matches_attachment {
            let close_search_start = open_end;
            if let Some(relative_close) = lower[close_search_start..].find("</figure>") {
                output.push_str(&html[cursor..figure_start]);
                let close_end = close_search_start + relative_close + "</figure>".len();
                cursor = skip_ascii_whitespace_for_app_data(html, close_end);
                continue;
            }
        }
        output.push_str(&html[cursor..open_end]);
        cursor = open_end;
    }
    output.push_str(&html[cursor..]);
    output
}

fn remove_matching_image_tags_for_app_data(html: &str, attachment_id: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut output = String::with_capacity(html.len());
    let mut cursor = 0usize;
    while let Some(relative_start) = lower[cursor..].find("<img") {
        let image_start = cursor + relative_start;
        let Some(tag_end) = html[image_start..]
            .find('>')
            .map(|offset| image_start + offset + 1)
        else {
            break;
        };
        let image_tag = &html[image_start..tag_end];
        let matches_attachment = first_quoted_attribute_value_for_app_data(image_tag, "src")
            .and_then(|value| value.strip_prefix("note-image://").map(str::to_string))
            .map(|id| id.trim() == attachment_id)
            .unwrap_or(false);
        if matches_attachment {
            output.push_str(&html[cursor..image_start]);
            cursor = skip_ascii_whitespace_for_app_data(html, tag_end);
        } else {
            output.push_str(&html[cursor..tag_end]);
            cursor = tag_end;
        }
    }
    output.push_str(&html[cursor..]);
    output
}

fn first_quoted_attribute_value_for_app_data(tag: &str, attr_name: &str) -> Option<String> {
    quoted_attribute_values_for_app_data(tag, attr_name)
        .into_iter()
        .next()
}

fn quoted_attribute_values_for_app_data(html: &str, attr_name: &str) -> Vec<String> {
    if attr_name.is_empty() {
        return Vec::new();
    }
    let lower = html.to_ascii_lowercase();
    let attr = attr_name.to_ascii_lowercase();
    let mut values = Vec::new();
    let mut search_start = 0usize;
    while let Some(relative_index) = lower.get(search_start..).and_then(|tail| tail.find(&attr)) {
        let attr_start = search_start + relative_index;
        if !is_attribute_name_boundary_for_app_data(html, attr_start, attr.len()) {
            search_start = attr_start + attr.len();
            continue;
        }
        let mut cursor = skip_ascii_whitespace_for_app_data(html, attr_start + attr.len());
        if html.as_bytes().get(cursor) != Some(&b'=') {
            search_start = cursor;
            continue;
        }
        cursor += 1;
        cursor = skip_ascii_whitespace_for_app_data(html, cursor);
        let quote = match html.as_bytes().get(cursor) {
            Some(b'\'') => b'\'',
            Some(b'"') => b'"',
            _ => {
                search_start = cursor;
                continue;
            }
        };
        cursor += 1;
        let value_start = cursor;
        while cursor < html.len() && html.as_bytes()[cursor] != quote {
            cursor += 1;
        }
        if cursor == html.len() {
            // A truncated quoted value is not an attachment identity.
            break;
        }
        values.push(html[value_start..cursor].to_string());
        search_start = cursor + 1;
    }
    values
}

fn collapse_repeated_blank_paragraphs_for_app_data(html: &str) -> String {
    let mut output = String::with_capacity(html.len());
    let mut cursor = 0usize;
    while cursor < html.len() {
        if let Some(first_end) = parse_blank_paragraph_for_app_data(html, cursor) {
            let mut end = first_end;
            let mut count = 1usize;
            while let Some(next_end) = parse_blank_paragraph_for_app_data(html, end) {
                end = next_end;
                count += 1;
            }
            if count > 1 {
                output.push_str("<p><br></p>");
            } else {
                output.push_str(&html[cursor..first_end]);
            }
            cursor = end;
        } else {
            let ch = html[cursor..].chars().next().unwrap_or_default();
            output.push(ch);
            cursor += ch.len_utf8();
        }
    }
    output
}

fn parse_blank_paragraph_for_app_data(html: &str, start: usize) -> Option<usize> {
    let mut cursor = skip_ascii_whitespace_for_app_data(html, start);
    cursor = consume_ascii_case_insensitive_for_app_data(html, cursor, "<p>")?;
    cursor = skip_ascii_whitespace_for_app_data(html, cursor);
    if let Some(after_break) = consume_blank_break_for_app_data(html, cursor) {
        cursor = skip_ascii_whitespace_for_app_data(html, after_break);
    }
    cursor = consume_ascii_case_insensitive_for_app_data(html, cursor, "</p>")?;
    Some(skip_ascii_whitespace_for_app_data(html, cursor))
}

fn consume_blank_break_for_app_data(html: &str, start: usize) -> Option<usize> {
    let mut cursor = consume_ascii_case_insensitive_for_app_data(html, start, "<br")?;
    cursor = skip_ascii_whitespace_for_app_data(html, cursor);
    if html.as_bytes().get(cursor) == Some(&b'/') {
        cursor += 1;
        cursor = skip_ascii_whitespace_for_app_data(html, cursor);
    }
    if html.as_bytes().get(cursor) == Some(&b'>') {
        Some(cursor + 1)
    } else {
        None
    }
}

fn consume_ascii_case_insensitive_for_app_data(
    html: &str,
    start: usize,
    token: &str,
) -> Option<usize> {
    let end = start.checked_add(token.len())?;
    if html
        .get(start..end)
        .is_some_and(|value| value.eq_ignore_ascii_case(token))
    {
        Some(end)
    } else {
        None
    }
}

fn skip_ascii_whitespace_for_app_data(value: &str, mut index: usize) -> usize {
    while index < value.len() && value.as_bytes()[index].is_ascii_whitespace() {
        index += 1;
    }
    index
}

fn is_attribute_name_boundary_for_app_data(value: &str, start: usize, len: usize) -> bool {
    let before = if start == 0 {
        true
    } else {
        !is_attribute_name_char_for_app_data(value.as_bytes()[start - 1])
    };
    let after_index = start + len;
    let after = if after_index >= value.len() {
        true
    } else {
        !is_attribute_name_char_for_app_data(value.as_bytes()[after_index])
    };
    before && after
}

fn is_attribute_name_char_for_app_data(value: u8) -> bool {
    value.is_ascii_alphanumeric() || value == b'-' || value == b'_' || value == b':'
}

fn merge_duplicate_notes(notes: Vec<NoteEntry>) -> Option<Vec<NoteEntry>> {
    let mut groups = HashMap::<String, Vec<NoteEntry>>::new();
    let mut ordered_ids = Vec::<String>::new();
    for note in notes {
        if !groups.contains_key(&note.id) {
            ordered_ids.push(note.id.clone());
        }
        groups.entry(note.id.clone()).or_default().push(note);
    }
    ordered_ids
        .into_iter()
        .map(|id| merge_duplicate_note_group(groups.remove(&id)?))
        .collect()
}

fn merge_duplicate_note_group(notes: Vec<NoteEntry>) -> Option<NoteEntry> {
    let protection_revisions = notes
        .iter()
        .map(effective_note_protection_state_revision)
        .collect::<Option<Vec<_>>>()?;
    let highest_protection_revision = protection_revisions.into_iter().max()?;
    let highest_generation = notes
        .into_iter()
        .filter(|note| {
            effective_note_protection_state_revision(note) == Some(highest_protection_revision)
        })
        .collect::<Vec<_>>();

    let encrypted = highest_generation
        .iter()
        .filter(|note| note.encryption.is_some())
        .cloned()
        .collect::<Vec<_>>();
    if let Some(first_encrypted) = encrypted.first() {
        let first_envelope = first_encrypted.encryption.as_ref()?;
        if encrypted.iter().skip(1).any(|note| {
            note.encryption
                .as_ref()
                .is_none_or(|envelope| !same_note_encryption_generation(first_envelope, envelope))
        }) {
            // A key fork at the winning protection generation is not a
            // last-writer-wins conflict: choosing either branch can make the
            // other branch permanently undecryptable. Reject the complete
            // duplicate group so repository recovery can use a known-good copy.
            return None;
        }
        // At the same generation, an authenticated ciphertext always outranks
        // plaintext. Multiple seals under the exact same key generation are
        // ordinary content races and use a stable LWW tie-breaker.
        return encrypted.into_iter().max_by_key(note_deterministic_lww_key);
    }

    let mut plaintext = highest_generation.into_iter();
    let first = plaintext.next()?;
    Some(plaintext.fold(first, merge_duplicate_note_pair))
}

fn note_deterministic_lww_key(note: &NoteEntry) -> (i64, i64, Vec<u8>) {
    (
        note_revision_epoch_millis(note),
        note.created_at_epoch_millis,
        serde_json::to_vec(note).unwrap_or_default(),
    )
}

fn note_revision_epoch_millis(note: &NoteEntry) -> i64 {
    note.updated_at_epoch_millis
}

fn effective_note_protection_state_revision(note: &NoteEntry) -> Option<i64> {
    if note.protection_state_revision < 0 {
        return None;
    }
    let Some(encryption) = note.encryption.as_ref() else {
        return Some(note.protection_state_revision);
    };
    if encryption.protection_revision <= 0 {
        return None;
    }
    let state_revision = if note.protection_state_revision == 0 {
        // Legacy protected notes predate the top-level transition counter.
        encryption.protection_revision
    } else {
        note.protection_state_revision
    };
    (state_revision == encryption.protection_revision).then_some(state_revision)
}

fn same_note_encryption_generation(
    left: &NoteEncryptionEnvelope,
    right: &NoteEncryptionEnvelope,
) -> bool {
    left.format_version == right.format_version
        && left.key_id == right.key_id
        && left.protection_revision == right.protection_revision
        && left.cipher_suite == right.cipher_suite
        && left.kdf == right.kdf
        && left.memory_kib == right.memory_kib
        && left.iterations == right.iterations
        && left.parallelism == right.parallelism
        && left.salt_base64 == right.salt_base64
        && left.key_nonce_base64 == right.key_nonce_base64
        && left.wrapped_key_base64 == right.wrapped_key_base64
}

fn validate_note_protection_state_transition(
    existing: Option<&NoteEntry>,
    incoming: &NoteEntry,
) -> Option<i64> {
    let incoming_revision = effective_note_protection_state_revision(incoming)?;
    let Some(existing) = existing else {
        return Some(incoming_revision);
    };
    let existing_revision = effective_note_protection_state_revision(existing)?;
    if incoming_revision < existing_revision {
        return None;
    }

    let is_next_revision = || {
        existing_revision
            .checked_add(1)
            .is_some_and(|next| incoming_revision == next)
    };

    match (existing.encryption.as_ref(), incoming.encryption.as_ref()) {
        (Some(existing_encryption), Some(incoming_encryption)) => {
            if incoming_revision == existing_revision {
                if !same_note_encryption_generation(existing_encryption, incoming_encryption) {
                    return None;
                }
            } else if !is_next_revision()
                || existing_encryption.key_id == incoming_encryption.key_id
            {
                // A password change advances exactly one generation and rotates
                // the data key. Skips and integer-only edits are never valid.
                return None;
            }
        }
        (Some(_), None) | (None, Some(_)) => {
            // Enabling or disabling protection advances exactly one generation.
            if !is_next_revision() {
                return None;
            }
        }
        (None, None) => {
            // Ordinary plaintext edits cannot manufacture protection history.
            if incoming_revision != existing_revision {
                return None;
            }
        }
    }
    Some(incoming_revision)
}

fn note_protection_revision(note: &NoteEntry) -> i64 {
    effective_note_protection_state_revision(note).unwrap_or(-1)
}

fn note_folder_revision_epoch_millis(folder: &NoteFolder) -> i64 {
    if folder.updated_at_epoch_millis > 0 {
        folder.updated_at_epoch_millis
    } else {
        folder.created_at_epoch_millis
    }
}

fn session_revision_epoch_millis(session: &TimerSession) -> i64 {
    session.updated_at_epoch_millis
}

fn note_attachment_revision_index(note: &NoteEntry) -> HashMap<String, i64> {
    let mut revisions = HashMap::<String, i64>::new();
    for attachment in note
        .attachments
        .iter()
        .chain(
            note.revisions
                .iter()
                .flat_map(|revision| revision.attachments.iter()),
        )
        .chain(
            note.versions
                .iter()
                .flat_map(|version| version.attachments.iter()),
        )
    {
        revisions
            .entry(attachment.id.clone())
            .and_modify(|existing| *existing = (*existing).max(attachment.updated_at_epoch_millis))
            .or_insert(attachment.updated_at_epoch_millis);
    }
    for attachment_id in note
        .revisions
        .iter()
        .flat_map(|revision| revision.attachment_ids.iter())
        .chain(
            note.versions
                .iter()
                .flat_map(|version| version.attachment_ids.iter()),
        )
    {
        revisions.entry(attachment_id.clone()).or_insert(0);
    }
    revisions
}

fn notes_reference_attachment(notes: &[NoteEntry], attachment_id: &str) -> bool {
    notes.iter().any(|note| {
        note.attachments
            .iter()
            .any(|attachment| attachment.id == attachment_id)
            || note.revisions.iter().any(|revision| {
                revision.attachment_ids.iter().any(|id| id == attachment_id)
                    || revision
                        .attachments
                        .iter()
                        .any(|attachment| attachment.id == attachment_id)
            })
            || note.versions.iter().any(|version| {
                version.attachment_ids.iter().any(|id| id == attachment_id)
                    || version
                        .attachments
                        .iter()
                        .any(|attachment| attachment.id == attachment_id)
            })
    })
}

fn apply_current_attachment_tombstones(
    note: &mut NoteEntry,
    tombstone_index: &HashMap<String, HashMap<String, i64>>,
    now: i64,
) {
    let mut deleted_attachments = HashMap::<String, i64>::new();
    for entity_type in [
        TOMBSTONE_ENTITY_NOTE_ATTACHMENT,
        TOMBSTONE_ENTITY_NOTE_MEDIA,
    ] {
        if let Some(tombstones) = tombstone_index.get(entity_type) {
            for (attachment_id, deleted_at) in tombstones {
                deleted_attachments
                    .entry(attachment_id.clone())
                    .and_modify(|existing| *existing = (*existing).max(*deleted_at))
                    .or_insert(*deleted_at);
            }
        }
    }
    if deleted_attachments.is_empty() {
        return;
    }
    let suppressed = deleted_attachments
        .iter()
        .filter_map(|(attachment_id, deleted_at)| {
            let active_revision = note
                .attachments
                .iter()
                .filter(|attachment| attachment.id == *attachment_id)
                .map(|attachment| attachment.updated_at_epoch_millis)
                .max();
            (active_revision.unwrap_or(i64::MIN) <= *deleted_at).then_some(attachment_id.clone())
        })
        .collect::<HashSet<_>>();
    if suppressed.is_empty() {
        return;
    }

    note.attachments
        .retain(|attachment| !suppressed.contains(&attachment.id));
    let mut document = note.resolved_document();
    let rich_text_enabled = document.rich_text_enabled;
    document.blocks = document
        .blocks
        .into_iter()
        .filter_map(|mut block| {
            if matches!(block.block_type, NoteBlockType::Image)
                && block
                    .attachment_id
                    .as_ref()
                    .is_some_and(|id| suppressed.contains(id))
            {
                return None;
            }
            if matches!(block.block_type, NoteBlockType::Text) && rich_text_enabled {
                for attachment_id in &suppressed {
                    block.text = remove_rich_text_attachment_reference_for_app_data(
                        &block.text,
                        attachment_id,
                    );
                }
            }
            Some(block)
        })
        .collect();
    let valid_attachment_ids = note.attachment_ids();
    note.document = document.sanitized(&valid_attachment_ids, now);
    for attachment_id in &suppressed {
        note.content =
            remove_rich_text_attachment_reference_for_app_data(&note.content, attachment_id);
    }
}

fn merge_duplicate_note_pair(left: NoteEntry, right: NoteEntry) -> NoteEntry {
    if left.encryption.is_some() || right.encryption.is_some() {
        let left_key = (
            note_protection_revision(&left),
            note_revision_epoch_millis(&left),
            left.created_at_epoch_millis,
            serde_json::to_vec(&left).unwrap_or_default(),
        );
        let right_key = (
            note_protection_revision(&right),
            note_revision_epoch_millis(&right),
            right.created_at_epoch_millis,
            serde_json::to_vec(&right).unwrap_or_default(),
        );
        return if right_key > left_key { right } else { left };
    }
    let left_key = (
        note_protection_revision(&left),
        note_revision_epoch_millis(&left),
        left.created_at_epoch_millis,
        serde_json::to_vec(&left).unwrap_or_default(),
    );
    let right_key = (
        note_protection_revision(&right),
        note_revision_epoch_millis(&right),
        right.created_at_epoch_millis,
        serde_json::to_vec(&right).unwrap_or_default(),
    );
    let (mut winner, loser) = if right_key > left_key {
        (right, left)
    } else {
        (left, right)
    };

    let loser_snapshot = (!note_current_content_equal(&winner, &loser)).then(|| {
        loser.snapshot_for_history(loser.updated_at_epoch_millis, loser.updated_at_epoch_millis)
    });
    winner.attachments.extend(loser.attachments);
    winner.attachments = distinct_by_latest(
        winner.attachments,
        |attachment| attachment.id.clone(),
        |attachment| attachment.updated_at_epoch_millis,
    );
    winner.revisions.extend(loser.revisions);
    if let Some(snapshot) = loser_snapshot {
        winner.revisions.push(snapshot);
    }
    winner.revisions = distinct_by_latest(
        winner.revisions,
        |revision| revision.id.clone(),
        |revision| revision.updated_at_epoch_millis,
    );
    winner.versions.extend(loser.versions);
    winner.versions = distinct_note_versions(winner.versions);
    if let Some(latest) = winner
        .versions
        .iter()
        .filter(|version| version.deleted_at_epoch_millis.is_none())
        .max_by(|left, right| {
            (
                left.sequence,
                left.created_at_epoch_millis,
                left.id.as_str(),
            )
                .cmp(&(
                    right.sequence,
                    right.created_at_epoch_millis,
                    right.id.as_str(),
                ))
        })
        .cloned()
    {
        let updated_at = winner
            .updated_at_epoch_millis
            .max(latest.updated_at_epoch_millis);
        winner = latest.apply_to_note(&winner, updated_at);
        winner.latest_version_id = latest.id;
    }
    winner
}

fn note_current_content_equal(left: &NoteEntry, right: &NoteEntry) -> bool {
    left.title == right.title
        && left.content == right.content
        && left.kind == right.kind
        && left.document == right.document
        && left.accent_seed == right.accent_seed
        && left.pinned == right.pinned
        && left.folder_id == right.folder_id
        && left.encryption == right.encryption
        && left.protection_state_revision == right.protection_state_revision
        && left.deleted_at_epoch_millis == right.deleted_at_epoch_millis
}

fn distinct_by_latest<T, K>(
    values: Vec<T>,
    mut key: impl FnMut(&T) -> K,
    mut updated_at: impl FnMut(&T) -> i64,
) -> Vec<T>
where
    K: Eq + std::hash::Hash,
{
    let mut indexes = HashMap::<K, usize>::new();
    let mut distinct = Vec::<T>::new();
    for value in values {
        let value_key = key(&value);
        if let Some(index) = indexes.get(&value_key).copied() {
            if updated_at(&value) > updated_at(&distinct[index]) {
                distinct[index] = value;
            }
        } else {
            indexes.insert(value_key, distinct.len());
            distinct.push(value);
        }
    }
    distinct
}

fn distinct_note_versions(values: Vec<NoteVersionSnapshot>) -> Vec<NoteVersionSnapshot> {
    let mut indexes = HashMap::<String, usize>::new();
    let mut distinct = Vec::<NoteVersionSnapshot>::new();
    for version in values {
        if let Some(index) = indexes.get(&version.id).copied() {
            let current = &distinct[index];
            let version_key = (
                version.updated_at_epoch_millis,
                version.created_at_epoch_millis,
                version.sequence,
                serde_json::to_vec(&version).unwrap_or_default(),
            );
            let current_key = (
                current.updated_at_epoch_millis,
                current.created_at_epoch_millis,
                current.sequence,
                serde_json::to_vec(current).unwrap_or_default(),
            );
            if version_key > current_key {
                distinct[index] = version;
            }
        } else {
            indexes.insert(version.id.clone(), distinct.len());
            distinct.push(version);
        }
    }
    distinct
}

fn distinct_values<T>(values: Vec<T>) -> Vec<T>
where
    T: Eq + std::hash::Hash + Clone,
{
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn stable_recovery_id<T: Serialize>(entity_type: &str, value: &T) -> String {
    let canonical = serde_json::to_vec(value).unwrap_or_default();
    let mut digest = Sha256::new();
    digest.update(b"gridtimer-recovery-id-v1\0");
    digest.update((entity_type.len() as u64).to_be_bytes());
    digest.update(entity_type.as_bytes());
    digest.update((canonical.len() as u64).to_be_bytes());
    digest.update(canonical);
    let digest = digest.finalize();
    format!("recovered-{entity_type}-v1-{digest:x}")
}

fn assign_missing_recovery_ids<T: Serialize>(
    values: &mut [T],
    entity_type: &str,
    id: impl Fn(&T) -> &str,
    mut set_id: impl FnMut(&mut T, String),
) {
    let mut occurrences = HashMap::<String, usize>::new();
    for value in values {
        if !id(value).is_empty() {
            continue;
        }
        let base = stable_recovery_id(entity_type, value);
        let occurrence = occurrences.entry(base.clone()).or_default();
        *occurrence += 1;
        set_id(value, format!("{base}-occurrence-{occurrence}"));
    }
}

fn default_red() -> String {
    "red".to_string()
}

fn default_amber() -> String {
    "amber".to_string()
}

fn default_image_mime() -> String {
    "image/jpeg".to_string()
}
