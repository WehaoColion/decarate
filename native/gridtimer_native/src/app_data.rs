use crate::finance_profile::FinanceProfile;
use crate::note_documents::{
    build_note_document_text_digest, NoteBlockTextInput, NoteDocumentTextInput,
};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{hash_map::DefaultHasher, HashSet};
use std::hash::{Hash, Hasher};

const APP_DATA_SCHEMA_VERSION: i32 = 10;
const DEFAULT_SLOT_COUNT: i32 = 14;
const MAX_TRACKED_DURATION_MILLIS: i64 = 10 * 365 * 24 * 60 * 60 * 1_000;
const MAX_NOTE_TITLE_LENGTH: usize = 64;
const MAX_NOTE_CONTENT_LENGTH: usize = 20_000;
const MAX_NOTE_COUNT: usize = 320;
const MAX_NOTE_FOLDER_NAME_LENGTH: usize = 18;
const MAX_NOTE_FOLDER_COUNT: usize = 24;
const MAX_NOTE_ATTACHMENT_COUNT: usize = 12;
const MAX_NOTE_ATTACHMENT_NAME_LENGTH: usize = 48;
const MAX_NOTE_FILE_NAME_LENGTH: usize = 64;
const MAX_NOTE_BLOCK_COUNT: usize = 96;
const MAX_NOTE_REVISION_COUNT: usize = 48;
const MAX_TEXT_BLOCK_LENGTH: usize = 60_000;
const MAX_CONTACT_NAME_LENGTH: usize = 48;
const MAX_CONTACT_ORG_LENGTH: usize = 48;
const MAX_CONTACT_PHONE_LENGTH: usize = 32;
const MAX_CONTACT_LABEL_LENGTH: usize = 24;
const MAX_BLOCK_CAPTION_LENGTH: usize = 160;
const MAX_CALL_NAME_LENGTH: usize = 48;
const MAX_CALL_NUMBER_LENGTH: usize = 32;
const MAX_NOTE_REVISION_TITLE_LENGTH: usize = 64;
const MAX_NOTE_REVISION_CONTENT_LENGTH: usize = 20_000;
const MAX_RICH_TEXT_PLAIN_LENGTH: usize = 20_000;
const MICRO_BREAK_FOCUS_MIN_MILLIS: i64 = 3 * 60 * 1_000;
const MICRO_BREAK_FOCUS_MAX_MILLIS: i64 = 5 * 60 * 1_000;
const MICRO_BREAK_FOCUS_STEP_MILLIS: i64 = 15_000;
const MICRO_BREAK_FOCUS_VARIANT_COUNT: i64 =
    ((MICRO_BREAK_FOCUS_MAX_MILLIS - MICRO_BREAK_FOCUS_MIN_MILLIS) / MICRO_BREAK_FOCUS_STEP_MILLIS)
        + 1;
const MICRO_BREAK_REST_MILLIS: i64 = 15_000;

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

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
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
    micro_break_phase: MicroBreakPhase,
    #[serde(default)]
    micro_break_cycle_index: i32,
    #[serde(default)]
    micro_break_phase_progress_millis: i64,
    #[serde(default)]
    updated_at: i64,
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
    created_at_epoch_millis: i64,
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
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    content: String,
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
    captured_at_epoch_millis: i64,
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
    finance_profile: FinanceProfile,
    #[serde(default)]
    theme_mode: ThemeMode,
}

pub fn default_app_data_json(now: i64) -> String {
    let categories = [
        ("category-work", "工作", "red"),
        ("category-study", "学习", "blue"),
        ("category-sport", "运动", "green"),
        ("category-life", "生活", "amber"),
    ]
    .into_iter()
    .map(|(id, name, accent_seed)| Category {
        id: id.to_string(),
        name: name.to_string(),
        accent_seed: accent_seed.to_string(),
    })
    .collect::<Vec<_>>();
    let slots = (1..=DEFAULT_SLOT_COUNT)
        .map(|slot_id| TimerSlot {
            id: slot_id,
            updated_at: now,
            ..TimerSlot::default()
        })
        .collect::<Vec<_>>();
    let data = AppData {
        schema_version: APP_DATA_SCHEMA_VERSION,
        categories,
        slots,
        slot_order: (1..=DEFAULT_SLOT_COUNT).collect(),
        sessions: Vec::new(),
        archived_tasks: Vec::new(),
        note_folders: Vec::new(),
        notes: Vec::new(),
        note_preferences: NotePreferences::default(),
        finance_profile: FinanceProfile::default(),
        theme_mode: ThemeMode::System,
    };
    serde_json::to_string(&data).unwrap_or_else(|_| "{}".to_string())
}

pub fn sanitize_app_data_json(raw: &str, now: i64) -> Option<String> {
    let data = serde_json::from_str::<AppData>(raw).ok()?;
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn delete_session_app_data_json(raw: &str, session_id: &str, now: i64) -> Option<String> {
    let mut data = serde_json::from_str::<AppData>(raw).ok()?;
    data.sessions.retain(|session| session.id != session_id);
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn delete_archived_task_app_data_json(
    raw: &str,
    archived_task_id: &str,
    now: i64,
) -> Option<String> {
    let mut data = serde_json::from_str::<AppData>(raw).ok()?;
    data.archived_tasks
        .retain(|archived_task| archived_task.id != archived_task_id);
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn upsert_note_app_data_json(raw: &str, note_json: &str, now: i64) -> Option<String> {
    let mut data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
    let note = serde_json::from_str::<NoteEntry>(note_json).ok()?;
    if note.id.trim().is_empty() {
        return None;
    }
    let existing = data
        .notes
        .iter()
        .find(|existing| existing.id == note.id)
        .cloned();
    let normalized = normalize_note_for_save(&data, note, existing.as_ref(), now);
    data.notes.retain(|note| note.id != normalized.id);
    data.notes.insert(0, normalized);
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn history_deletion_summary_values(
    raw: &str,
    session_id: &str,
    archived_task_id: &str,
    now: i64,
) -> Option<[i64; 6]> {
    let data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
    let sessions_before = data.sessions.len() as i64;
    let archived_before = data.archived_tasks.len() as i64;
    let sessions_after = data
        .sessions
        .iter()
        .filter(|session| session_id.trim().is_empty() || session.id != session_id)
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
    let mut data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
    for slot in &mut data.slots {
        if slot.id == slot_id {
            slot.title = truncate_chars(title.trim_start(), 24);
            slot.updated_at = now;
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
    let mut data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
    for slot in &mut data.slots {
        if slot.id == slot_id {
            slot.note = truncate_chars(note.trim_start(), 60);
            slot.updated_at = now;
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
    let mut data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
    let next_category_id = if category_id.trim().is_empty() {
        None
    } else {
        Some(category_id.to_string())
    };
    for slot in &mut data.slots {
        if slot.id == slot_id {
            slot.category_id = next_category_id.clone();
            slot.updated_at = now;
            break;
        }
    }
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn set_slot_order_app_data_json(raw: &str, slot_order: &[i32], now: i64) -> Option<String> {
    let mut data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
    data.slot_order = normalize_slot_order(slot_order);
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
    let safe_name = truncate_chars(&compact_whitespace(name), 12);
    if safe_name.trim().is_empty() {
        return None;
    }
    let mut data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
    let existing_category = data
        .categories
        .iter()
        .find(|category| category.name.eq_ignore_ascii_case(&safe_name))
        .cloned();
    let selected_category = existing_category.unwrap_or_else(|| Category {
        id: if category_id.trim().is_empty() {
            format!("category-{now}-{}", data.categories.len().saturating_add(1))
        } else {
            category_id.to_string()
        },
        name: safe_name,
        accent_seed: accent_seed_for_category_index(data.categories.len()),
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
                slot.category_id = Some(selected_category.id.clone());
                slot.updated_at = now;
                break;
            }
        }
    }

    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn start_slot_app_data_json(raw: &str, slot_id: i32, now: i64) -> Option<String> {
    let data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
    let mut data = resolve_micro_breaks_for_app_data(data, now);
    let valid_category_ids = data
        .categories
        .iter()
        .map(|category| category.id.clone())
        .collect::<HashSet<_>>();
    for slot in &mut data.slots {
        if slot.id == slot_id && slot.running_since_epoch_millis.is_none() {
            *slot = slot.clone().sanitized(&valid_category_ids, now);
            slot.running_since_epoch_millis = Some(now);
            slot.updated_at = now;
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
    let data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
    let mut data = resolve_micro_breaks_for_app_data(data, now);
    let mut pause_sessions = Vec::<TimerSession>::new();
    let valid_category_ids = data
        .categories
        .iter()
        .map(|category| category.id.clone())
        .collect::<HashSet<_>>();
    for slot in &mut data.slots {
        if target_slot_ids.contains(&slot.id) {
            let (next_slot, session) =
                pause_slot_micro_break(slot.clone().sanitized(&valid_category_ids, now), now);
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
    let data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
    let mut data = resolve_micro_breaks_for_app_data(data, now);
    let valid_category_ids = data
        .categories
        .iter()
        .map(|category| category.id.clone())
        .collect::<HashSet<_>>();
    let mut session_to_add = None;
    for slot in &mut data.slots {
        if slot.id == slot_id {
            let (paused_slot, session) =
                pause_slot_micro_break(slot.clone().sanitized(&valid_category_ids, now), now);
            let mut reset_slot = paused_slot.cleared_micro_break_tracking(now);
            reset_slot.accumulated_millis = 0;
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
    let safe_name = truncate_chars(&compact_whitespace(name), MAX_NOTE_FOLDER_NAME_LENGTH);
    if safe_name.trim().is_empty() {
        return None;
    }
    let mut data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
    if let Some(existing) = data
        .note_folders
        .iter()
        .find(|folder| folder.name.eq_ignore_ascii_case(&safe_name))
    {
        data.note_preferences.selected_folder_id = Some(existing.id.clone());
    } else {
        let id = if folder_id.trim().is_empty() {
            format!("folder-{now}-{}", data.note_folders.len().saturating_add(1))
        } else {
            folder_id.to_string()
        };
        data.note_folders.insert(
            0,
            NoteFolder {
                id: id.clone(),
                name: safe_name,
                created_at_epoch_millis: now,
                updated_at_epoch_millis: now,
            },
        );
        data.note_preferences.selected_folder_id = Some(id);
    }
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn rename_note_folder_app_data_json(
    raw: &str,
    folder_id: &str,
    name: &str,
    now: i64,
) -> Option<String> {
    let safe_name = truncate_chars(&compact_whitespace(name), MAX_NOTE_FOLDER_NAME_LENGTH);
    if safe_name.trim().is_empty() {
        return None;
    }
    let mut data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
    for folder in &mut data.note_folders {
        if folder.id == folder_id {
            folder.name = safe_name;
            folder.updated_at_epoch_millis = now;
            break;
        }
    }
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn delete_note_folder_app_data_json(raw: &str, folder_id: &str, now: i64) -> Option<String> {
    let mut data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
    if !data
        .note_folders
        .iter()
        .any(|folder| folder.id == folder_id)
    {
        return serde_json::to_string(&data).ok();
    }
    data.note_folders.retain(|folder| folder.id != folder_id);
    for note in &mut data.notes {
        if note.folder_id.as_deref() == Some(folder_id) {
            note.folder_id = None;
            note.updated_at_epoch_millis = now;
        }
    }
    if data.note_preferences.selected_folder_id.as_deref() == Some(folder_id) {
        data.note_preferences.selected_folder_id = None;
    }
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn set_selected_note_folder_app_data_json(
    raw: &str,
    folder_id: &str,
    now: i64,
) -> Option<String> {
    let mut data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
    data.note_preferences.selected_folder_id = if folder_id.trim().is_empty() {
        None
    } else {
        data.note_folders
            .iter()
            .any(|folder| folder.id == folder_id)
            .then(|| folder_id.to_string())
    };
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn set_note_sort_mode_app_data_json(
    raw: &str,
    sort_mode_code: i32,
    now: i64,
) -> Option<String> {
    let mut data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
    data.note_preferences.sort_mode = match sort_mode_code {
        0 => NoteSortMode::UpdatedDesc,
        1 => NoteSortMode::CreatedDesc,
        2 => NoteSortMode::CreatedAsc,
        3 => NoteSortMode::TitleAsc,
        _ => return None,
    };
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn move_note_to_folder_app_data_json(
    raw: &str,
    note_id: &str,
    folder_id: &str,
    now: i64,
) -> Option<String> {
    let mut data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
    let safe_folder_id = if folder_id.trim().is_empty() {
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
            note.updated_at_epoch_millis = now;
            break;
        }
    }
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn restore_note_app_data_json(raw: &str, note_id: &str, now: i64) -> Option<String> {
    let mut data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
    for note in &mut data.notes {
        if note.id == note_id {
            note.deleted_at_epoch_millis = None;
            note.updated_at_epoch_millis = now;
            break;
        }
    }
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn delete_note_app_data_json(raw: &str, note_id: &str, now: i64) -> Option<String> {
    let mut data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
    for note in &mut data.notes {
        if note.id == note_id {
            note.pinned = false;
            note.deleted_at_epoch_millis = Some(now);
            note.updated_at_epoch_millis = now;
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
    let mut data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
    for note in &mut data.notes {
        if note.id == note_id {
            note.pinned = pinned && note.deleted_at_epoch_millis.is_none();
            note.updated_at_epoch_millis = now;
            break;
        }
    }
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn capture_note_revision_app_data_json(raw: &str, note_id: &str, now: i64) -> Option<String> {
    let mut data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
    for note in &mut data.notes {
        if note.id == note_id {
            let previous = note.clone();
            let snapshot = previous.snapshot_for_history(now);
            let mut revisions =
                Vec::with_capacity(MAX_NOTE_REVISION_COUNT.min(note.revisions.len() + 1));
            revisions.push(snapshot);
            revisions.extend(
                note.revisions
                    .iter()
                    .filter(|revision| !revision.matches_note(&previous))
                    .take(MAX_NOTE_REVISION_COUNT.saturating_sub(1))
                    .cloned(),
            );
            note.revisions = revisions;
            note.updated_at_epoch_millis = now;
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
    let mut data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
    let Some(note_index) = data.notes.iter().position(|note| note.id == note_id) else {
        return serde_json::to_string(&data).ok();
    };
    let target = data.notes[note_index].clone();
    let Some(revision) = target
        .revisions
        .iter()
        .find(|revision| revision.id == revision_id)
        .cloned()
    else {
        return serde_json::to_string(&data).ok();
    };
    let valid_attachment_ids = target.attachment_ids();
    let restored_document = revision
        .document
        .clone()
        .sanitized(&valid_attachment_ids, now);
    let snapshot = target.snapshot_for_history(now);
    let mut restored = target.clone();
    restored.title = revision.title;
    restored.content = revision.content;
    restored.document = restored_document;
    restored.accent_seed = revision.accent_seed;
    restored.pinned = revision.pinned;
    restored.folder_id = revision.folder_id;
    restored.deleted_at_epoch_millis = None;
    restored.updated_at_epoch_millis = now;
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
    let mut data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
    data.notes.retain(|note| note.id != note_id);
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn empty_note_trash_app_data_json(raw: &str, now: i64) -> Option<String> {
    let mut data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
    data.notes
        .retain(|note| note.deleted_at_epoch_millis.is_none());
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn set_theme_mode_app_data_json(raw: &str, theme_mode_code: i32, now: i64) -> Option<String> {
    let mut data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
    data.theme_mode = match theme_mode_code {
        0 => ThemeMode::System,
        1 => ThemeMode::Light,
        2 => ThemeMode::Dark,
        _ => return None,
    };
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
    let mut data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
    data.finance_profile = serde_json::from_str::<FinanceProfile>(profile_json)
        .ok()?
        .sanitized();
    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
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
    let mut data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
    let Some(slot_index) = data.slots.iter().position(|slot| slot.id == slot_id) else {
        return serde_json::to_string(&data).ok();
    };
    let slot = data.slots[slot_index].clone();
    if slot.running_since_epoch_millis.is_some() || slot.is_blank_slate() {
        return serde_json::to_string(&data).ok();
    }

    data.archived_tasks.insert(
        0,
        ArchivedTask {
            id: archived_task_id.to_string(),
            original_slot_id: slot.id,
            title: slot.title,
            category_id: slot.category_id,
            note: slot.note,
            accumulated_millis: slot.accumulated_millis,
            archived_at_epoch_millis: now,
        },
    );
    data.slots[slot_index] = data.slots[slot_index]
        .clone()
        .cleared_micro_break_tracking(now)
        .as_blank_task(now);

    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn restore_archived_task_app_data_json(
    raw: &str,
    archived_task_id: &str,
    now: i64,
) -> Option<String> {
    let mut data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
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
                .cleared_micro_break_tracking(now)
                .with_restored_archived_task(&archived_task, now);
            break;
        }
    }
    data.archived_tasks.remove(task_index);

    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn import_note_image_attachment_app_data_json(
    raw: &str,
    note_id: &str,
    attachment_json: &str,
    now: i64,
) -> Option<String> {
    let mut data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
    let mut attachment = serde_json::from_str::<NoteAttachment>(attachment_json).ok()?;
    attachment = attachment.sanitized(now);
    if attachment.id.trim().is_empty() {
        return None;
    }
    let Some(note) = data.notes.iter_mut().find(|note| note.id == note_id) else {
        return serde_json::to_string(&data).ok();
    };
    if note.deleted_at_epoch_millis.is_some() || note.attachments.len() >= MAX_NOTE_ATTACHMENT_COUNT
    {
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
    note.updated_at_epoch_millis = now;

    data.sanitized(now)
        .and_then(|sanitized| serde_json::to_string(&sanitized).ok())
}

pub fn delete_note_attachment_app_data_json(
    raw: &str,
    note_id: &str,
    attachment_id: &str,
    now: i64,
) -> Option<String> {
    if attachment_id.trim().is_empty() {
        return None;
    }
    let mut data = serde_json::from_str::<AppData>(raw).ok()?.sanitized(now)?;
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

    note.attachments
        .retain(|attachment| attachment.id != attachment_id);
    let mut document = note.resolved_document();
    let document_rich_text_enabled = document.rich_text_enabled;
    document.blocks = document
        .blocks
        .into_iter()
        .filter_map(|mut block| match &block.block_type {
            NoteBlockType::Image if block.attachment_id.as_deref() == Some(attachment_id) => None,
            NoteBlockType::Text if document_rich_text_enabled => {
                block.text =
                    remove_rich_text_attachment_reference_for_app_data(&block.text, attachment_id);
                Some(block)
            }
            _ => Some(block),
        })
        .collect();
    note.document = document.sanitized(&note.attachment_ids(), now);
    note.content = note.document.storage_content();
    note.updated_at_epoch_millis = now;

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
    fn sanitized(self, now: i64) -> Option<Self> {
        if self.categories.is_empty() {
            return None;
        }

        let mut categories = self
            .categories
            .into_iter()
            .map(|category| Category {
                name: truncate_chars(&normalized_category_name(&category.name), 12),
                ..category
            })
            .collect::<Vec<_>>();
        categories = distinct_by(categories, |category| category.id.clone());
        let valid_category_ids = categories
            .iter()
            .map(|category| category.id.clone())
            .collect::<HashSet<_>>();

        let mut note_folders = self
            .note_folders
            .into_iter()
            .map(|folder| {
                let created_at = sanitize_timestamp(folder.created_at_epoch_millis, now);
                let updated_at =
                    sanitize_timestamp(folder.updated_at_epoch_millis, now).max(created_at);
                NoteFolder {
                    name: truncate_chars(
                        trimmed_or(&folder.name, "默认文件夹"),
                        MAX_NOTE_FOLDER_NAME_LENGTH,
                    ),
                    created_at_epoch_millis: created_at,
                    updated_at_epoch_millis: updated_at,
                    ..folder
                }
            })
            .collect::<Vec<_>>();
        note_folders = distinct_by(note_folders, |folder| folder.id.clone());
        note_folders.sort_by_key(|folder| Reverse(folder.updated_at_epoch_millis));
        note_folders.truncate(MAX_NOTE_FOLDER_COUNT);
        let valid_folder_ids = note_folders
            .iter()
            .map(|folder| folder.id.clone())
            .collect::<HashSet<_>>();

        let slots = (1..=DEFAULT_SLOT_COUNT)
            .map(|slot_id| {
                self.slots
                    .iter()
                    .find(|slot| slot.id == slot_id)
                    .cloned()
                    .map(|slot| slot.sanitized(&valid_category_ids, now))
                    .unwrap_or_else(|| TimerSlot {
                        id: slot_id,
                        updated_at: now,
                        ..TimerSlot::default()
                    })
            })
            .collect();

        let mut sessions = self
            .sessions
            .into_iter()
            .filter_map(|session| session.sanitized(&valid_category_ids, now))
            .collect::<Vec<_>>();
        sessions.sort_by_key(|session| Reverse(session.ended_at_epoch_millis));
        sessions.truncate(500);

        let mut archived_tasks = self
            .archived_tasks
            .into_iter()
            .filter_map(|task| task.sanitized(&valid_category_ids, now))
            .collect::<Vec<_>>();
        archived_tasks.sort_by_key(|task| Reverse(task.archived_at_epoch_millis));
        archived_tasks.truncate(200);

        let mut notes = self
            .notes
            .into_iter()
            .map(|note| note.sanitized(&valid_folder_ids, now))
            .collect::<Vec<_>>();
        notes = distinct_by(notes, |note| note.id.clone());
        notes.sort_by(|left, right| {
            (
                left.deleted_at_epoch_millis.is_some(),
                !left.pinned,
                Reverse(left.updated_at_epoch_millis),
                Reverse(left.created_at_epoch_millis),
            )
                .cmp(&(
                    right.deleted_at_epoch_millis.is_some(),
                    !right.pinned,
                    Reverse(right.updated_at_epoch_millis),
                    Reverse(right.created_at_epoch_millis),
                ))
        });
        notes.truncate(MAX_NOTE_COUNT);

        let mut note_preferences = self.note_preferences;
        note_preferences.selected_folder_id = note_preferences
            .selected_folder_id
            .take()
            .filter(|folder_id| valid_folder_ids.contains(folder_id));

        Some(Self {
            schema_version: APP_DATA_SCHEMA_VERSION,
            categories,
            slots,
            slot_order: normalize_slot_order(&self.slot_order),
            sessions,
            archived_tasks,
            note_folders,
            notes,
            note_preferences,
            finance_profile: self.finance_profile.sanitized(),
            theme_mode: self.theme_mode,
        })
    }
}

impl TimerSlot {
    fn sanitized(mut self, valid_category_ids: &HashSet<String>, now: i64) -> Self {
        let cycle_index = self.micro_break_cycle_index.max(0);
        let phase_target = match self.micro_break_phase {
            MicroBreakPhase::Focus => compute_micro_break_target_millis(self.id, cycle_index),
            MicroBreakPhase::Break => MICRO_BREAK_REST_MILLIS,
        };
        self.title = truncate_chars(self.title.trim(), 24);
        self.category_id = self
            .category_id
            .take()
            .filter(|category_id| valid_category_ids.contains(category_id));
        self.note = truncate_chars(self.note.trim(), 60);
        self.accumulated_millis = sanitize_tracked_duration(self.accumulated_millis);
        self.running_since_epoch_millis = self.running_since_epoch_millis.and_then(|value| {
            if (0..=now.max(0)).contains(&value) {
                Some(value)
            } else {
                None
            }
        });
        self.micro_break_cycle_index = cycle_index;
        self.micro_break_phase_progress_millis =
            sanitize_tracked_duration(self.micro_break_phase_progress_millis)
                .clamp(0, phase_target);
        self.updated_at = sanitize_timestamp(self.updated_at, now);
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

    fn cleared_micro_break_tracking(mut self, updated_at: i64) -> Self {
        self.running_since_epoch_millis = None;
        self.micro_break_phase = MicroBreakPhase::Focus;
        self.micro_break_cycle_index = 0;
        self.micro_break_phase_progress_millis = 0;
        self.updated_at = updated_at;
        self
    }

    fn as_blank_task(mut self, updated_at: i64) -> Self {
        self.title.clear();
        self.category_id = None;
        self.note.clear();
        self.accumulated_millis = 0;
        self.updated_at = updated_at;
        self
    }

    fn with_restored_archived_task(
        mut self,
        archived_task: &ArchivedTask,
        updated_at: i64,
    ) -> Self {
        self.title = archived_task.title.clone();
        self.category_id = archived_task.category_id.clone();
        self.note = archived_task.note.clone();
        self.accumulated_millis = archived_task.accumulated_millis;
        self.updated_at = updated_at;
        self
    }
}

impl TimerSession {
    fn sanitized(mut self, valid_category_ids: &HashSet<String>, now: i64) -> Option<Self> {
        if !(1..=DEFAULT_SLOT_COUNT).contains(&self.slot_id) {
            return None;
        }
        let started_at = sanitize_timestamp(self.started_at_epoch_millis, now);
        let ended_at = sanitize_timestamp(self.ended_at_epoch_millis, now).max(started_at);
        self.slot_title = truncate_chars(self.slot_title.trim(), 24);
        self.category_id = self
            .category_id
            .take()
            .filter(|category_id| valid_category_ids.contains(category_id));
        self.started_at_epoch_millis = started_at;
        self.ended_at_epoch_millis = ended_at;
        self.duration_millis =
            sanitize_tracked_duration(self.duration_millis).min(ended_at - started_at);
        Some(self)
    }
}

impl ArchivedTask {
    fn sanitized(mut self, valid_category_ids: &HashSet<String>, now: i64) -> Option<Self> {
        if !(1..=DEFAULT_SLOT_COUNT).contains(&self.original_slot_id) {
            return None;
        }
        self.title = truncate_chars(self.title.trim(), 24);
        self.category_id = self
            .category_id
            .take()
            .filter(|category_id| valid_category_ids.contains(category_id));
        self.note = truncate_chars(self.note.trim(), 60);
        self.accumulated_millis = sanitize_tracked_duration(self.accumulated_millis);
        self.archived_at_epoch_millis = sanitize_timestamp(self.archived_at_epoch_millis, now);
        Some(self)
    }
}

impl NoteEntry {
    fn sanitized(mut self, valid_folder_ids: &HashSet<String>, now: i64) -> Self {
        let created_at = sanitize_timestamp(self.created_at_epoch_millis, now);
        let updated_at = sanitize_timestamp(self.updated_at_epoch_millis, now).max(created_at);
        let deleted_at = self
            .deleted_at_epoch_millis
            .map(|value| sanitize_timestamp(value, now).max(updated_at));
        let mut attachments = std::mem::take(&mut self.attachments)
            .into_iter()
            .map(|attachment| attachment.sanitized(now))
            .collect::<Vec<_>>();
        attachments = distinct_by(attachments, |attachment| attachment.id.clone());
        attachments.truncate(MAX_NOTE_ATTACHMENT_COUNT);
        let valid_attachment_ids = attachments
            .iter()
            .map(|attachment| attachment.id.clone())
            .collect::<HashSet<_>>();
        let mut revisions = std::mem::take(&mut self.revisions)
            .into_iter()
            .map(|revision| revision.sanitized(&valid_attachment_ids, now))
            .collect::<Vec<_>>();
        revisions = distinct_by(revisions, |revision| revision.id.clone());
        revisions.sort_by_key(|revision| Reverse(revision.captured_at_epoch_millis));
        revisions.truncate(MAX_NOTE_REVISION_COUNT);

        self.title = truncate_chars(self.title.trim(), MAX_NOTE_TITLE_LENGTH);
        self.content = truncate_chars(
            normalize_newlines(&self.content).trim(),
            MAX_NOTE_CONTENT_LENGTH,
        );
        self.document = std::mem::take(&mut self.document).sanitized(&valid_attachment_ids, now);
        self.accent_seed = sanitize_accent_seed(&self.accent_seed);
        self.folder_id = self
            .folder_id
            .take()
            .filter(|folder_id| valid_folder_ids.contains(folder_id));
        self.attachments = attachments;
        self.revisions = revisions;
        self.created_at_epoch_millis = created_at;
        self.updated_at_epoch_millis = updated_at;
        self.deleted_at_epoch_millis = deleted_at;
        self
    }

    fn attachment_ids(&self) -> HashSet<String> {
        self.attachments
            .iter()
            .map(|attachment| attachment.id.clone())
            .collect()
    }

    fn resolved_document(&self) -> NoteDocument {
        if self.document.blocks.is_empty() {
            legacy_note_document(&self.content, &self.attachments)
        } else {
            self.document.clone()
        }
    }

    fn snapshot_for_history(&self, captured_at_epoch_millis: i64) -> NoteRevisionSnapshot {
        let resolved_document = self.resolved_document();
        NoteRevisionSnapshot {
            id: revision_snapshot_id(
                &self.id,
                captured_at_epoch_millis,
                &self.updated_at_epoch_millis,
            ),
            title: truncate_chars(self.title.trim(), MAX_NOTE_REVISION_TITLE_LENGTH),
            content: truncate_chars(
                &resolved_document.storage_content(),
                MAX_NOTE_REVISION_CONTENT_LENGTH,
            ),
            document: resolved_document,
            accent_seed: self.accent_seed.clone(),
            pinned: self.pinned,
            folder_id: self.folder_id.clone(),
            attachment_ids: self.document_image_attachment_ids(),
            captured_at_epoch_millis,
        }
    }

    fn document_image_attachment_ids(&self) -> Vec<String> {
        let valid_attachment_ids = self.attachment_ids();
        let mut ids = self
            .resolved_document()
            .blocks
            .into_iter()
            .filter(|block| matches!(block.block_type, NoteBlockType::Image))
            .filter_map(|block| block.attachment_id)
            .filter(|id| valid_attachment_ids.contains(id))
            .collect::<Vec<_>>();
        ids = distinct_values(ids);
        ids.truncate(MAX_NOTE_ATTACHMENT_COUNT);
        ids
    }
}

impl NoteAttachment {
    fn sanitized(mut self, now: i64) -> Self {
        let safe_display_name =
            truncate_chars(self.display_name.trim(), MAX_NOTE_ATTACHMENT_NAME_LENGTH);
        let safe_file_name = truncate_chars(
            &sanitize_attachment_file_name(self.file_name.trim(), &self.id),
            MAX_NOTE_FILE_NAME_LENGTH,
        );
        self.file_name = safe_file_name.clone();
        self.display_name = if safe_display_name.is_empty() {
            safe_file_name
        } else {
            safe_display_name
        };
        if self.mime_type.is_empty() {
            self.mime_type = default_image_mime();
        }
        self.width = self.width.max(0);
        self.height = self.height.max(0);
        self.size_bytes = self.size_bytes.max(0);
        self.created_at_epoch_millis = sanitize_timestamp(self.created_at_epoch_millis, now);
        self
    }
}

impl NoteRevisionSnapshot {
    fn sanitized(mut self, valid_attachment_ids: &HashSet<String>, now: i64) -> Self {
        let mut attachment_ids = self
            .attachment_ids
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| valid_attachment_ids.contains(value))
            .collect::<Vec<_>>();
        attachment_ids = distinct_values(attachment_ids);
        attachment_ids.truncate(MAX_NOTE_ATTACHMENT_COUNT);
        self.title = truncate_chars(self.title.trim(), MAX_NOTE_REVISION_TITLE_LENGTH);
        self.content = truncate_chars(
            normalize_newlines(&self.content).trim(),
            MAX_NOTE_REVISION_CONTENT_LENGTH,
        );
        self.document = std::mem::take(&mut self.document).sanitized(valid_attachment_ids, now);
        self.accent_seed = sanitize_accent_seed(&self.accent_seed);
        self.attachment_ids = attachment_ids;
        self.captured_at_epoch_millis = sanitize_timestamp(self.captured_at_epoch_millis, now);
        self
    }

    fn matches_note(&self, note: &NoteEntry) -> bool {
        let resolved_document = note.resolved_document();
        self.title == note.title.trim()
            && self.content == resolved_document.storage_content()
            && self.document == resolved_document
            && self.accent_seed == note.accent_seed
            && self.pinned == note.pinned
            && self.folder_id.as_deref() == note.folder_id.as_deref()
            && self.attachment_ids == note.document_image_attachment_ids()
    }
}

impl NoteDocument {
    fn sanitized(mut self, valid_attachment_ids: &HashSet<String>, now: i64) -> Self {
        let mut blocks = std::mem::take(&mut self.blocks)
            .into_iter()
            .filter_map(|block| block.sanitized(valid_attachment_ids, now))
            .collect::<Vec<_>>();
        blocks = distinct_by(blocks, |block| block.id.clone());
        blocks.truncate(MAX_NOTE_BLOCK_COUNT);
        self.markdown_enabled = self.markdown_enabled && !self.rich_text_enabled;
        self.rich_text_plain_text = if self.rich_text_enabled {
            truncate_chars(
                normalize_newlines(&self.rich_text_plain_text).trim(),
                MAX_RICH_TEXT_PLAIN_LENGTH,
            )
        } else {
            String::new()
        };
        self.blocks = blocks;
        self
    }

    fn storage_content(&self) -> String {
        build_note_document_text_digest(&self.to_text_input(), true, false, "").plain_text
    }

    fn is_blank(&self) -> bool {
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
            text: self.text.clone(),
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

    fn sanitized(mut self, valid_attachment_ids: &HashSet<String>, now: i64) -> Option<Self> {
        let mut phones = self
            .contact_phones
            .iter()
            .map(|phone| NoteContactPhone {
                label: truncate_chars(phone.label.trim(), MAX_CONTACT_LABEL_LENGTH),
                number: truncate_chars(phone.number.trim(), MAX_CONTACT_PHONE_LENGTH),
            })
            .filter(|phone| !phone.label.is_empty() || !phone.number.is_empty())
            .collect::<Vec<_>>();
        phones = distinct_by(phones, |phone| (phone.label.clone(), phone.number.clone()));
        phones.truncate(4);
        self.text = truncate_chars(
            normalize_newlines(&self.text).trim_end(),
            MAX_TEXT_BLOCK_LENGTH,
        );
        self.attachment_id = self
            .attachment_id
            .take()
            .filter(|id| valid_attachment_ids.contains(id));
        self.caption = truncate_chars(self.caption.trim(), MAX_BLOCK_CAPTION_LENGTH);
        self.contact_name = truncate_chars(self.contact_name.trim(), MAX_CONTACT_NAME_LENGTH);
        self.contact_organization =
            truncate_chars(self.contact_organization.trim(), MAX_CONTACT_ORG_LENGTH);
        self.contact_phones = phones;
        self.call_phone_number =
            truncate_chars(self.call_phone_number.trim(), MAX_CALL_NUMBER_LENGTH);
        self.call_contact_name =
            truncate_chars(self.call_contact_name.trim(), MAX_CALL_NAME_LENGTH);
        self.call_occurred_at_epoch_millis = self
            .call_occurred_at_epoch_millis
            .map(|value| sanitize_timestamp(value, now));
        self.call_duration_millis = self.call_duration_millis.map(|value| value.max(0));
        match self.block_type {
            NoteBlockType::Text => (!self.text.is_empty()).then_some(self),
            NoteBlockType::Image => self
                .attachment_id
                .as_ref()
                .is_some_and(|value| !value.is_empty())
                .then_some(self),
            NoteBlockType::Contact => {
                (!self.contact_name.is_empty() || !self.contact_phones.is_empty()).then_some(self)
            }
            NoteBlockType::Call => (!self.call_phone_number.is_empty()
                || !self.call_contact_name.is_empty()
                || !self.text.is_empty())
            .then_some(self),
        }
    }

    fn is_blank(&self) -> bool {
        match self.block_type {
            NoteBlockType::Text => self.text.trim().is_empty(),
            NoteBlockType::Image => self
                .attachment_id
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty(),
            NoteBlockType::Contact => {
                self.contact_name.trim().is_empty()
                    && self.contact_organization.trim().is_empty()
                    && self.contact_phones.iter().all(|phone| {
                        phone.label.trim().is_empty() && phone.number.trim().is_empty()
                    })
            }
            NoteBlockType::Call => {
                self.call_phone_number.trim().is_empty()
                    && self.call_contact_name.trim().is_empty()
                    && self.text.trim().is_empty()
            }
        }
    }
}

fn resolve_micro_breaks_for_app_data(mut data: AppData, now: i64) -> AppData {
    let valid_category_ids = data
        .categories
        .iter()
        .map(|category| category.id.clone())
        .collect::<HashSet<_>>();
    let mut generated_sessions = Vec::<TimerSession>::new();
    let mut resolved_slots = Vec::with_capacity(data.slots.len());
    for slot in data.slots {
        let (resolved_slot, sessions) =
            resolve_slot_micro_break(slot.sanitized(&valid_category_ids, now), now);
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
    data
}

fn resolve_slot_micro_break(mut slot: TimerSlot, now: i64) -> (TimerSlot, Vec<TimerSession>) {
    let Some(running_since) = slot.running_since_epoch_millis else {
        return (slot, Vec::new());
    };
    let safe_now = now.max(running_since);
    let mut phase = slot.micro_break_phase.clone();
    let mut cycle_index = slot.micro_break_cycle_index.max(0);
    let mut phase_progress = slot.micro_break_phase_progress_millis.max(0);
    let mut accumulated_millis = sanitize_tracked_duration(slot.accumulated_millis);
    let mut active_segment_start = running_since;
    let mut remaining_elapsed = (safe_now - running_since).max(0);
    let mut latest_transition_at = None;
    let mut sessions = Vec::<TimerSession>::new();

    loop {
        let phase_target = micro_break_phase_target_millis(slot.id, &phase, cycle_index);
        phase_progress = phase_progress.clamp(0, phase_target);
        let remaining_in_phase = (phase_target - phase_progress).max(0);

        if remaining_in_phase == 0 {
            if phase == MicroBreakPhase::Focus {
                phase = MicroBreakPhase::Break;
                phase_progress = 0;
                latest_transition_at = Some(active_segment_start);
            } else {
                phase = MicroBreakPhase::Focus;
                phase_progress = 0;
                cycle_index = cycle_index.saturating_add(1);
                latest_transition_at = Some(active_segment_start);
            }
            continue;
        }

        if remaining_elapsed < remaining_in_phase {
            break;
        }

        let transition_at = active_segment_start.saturating_add(remaining_in_phase);
        if phase == MicroBreakPhase::Focus {
            accumulated_millis =
                sanitize_tracked_duration(accumulated_millis.saturating_add(remaining_in_phase));
            if remaining_in_phase > 0 {
                sessions.push(TimerSession {
                    id: generated_timer_session_id(slot.id, active_segment_start, transition_at),
                    slot_id: slot.id,
                    slot_title: timer_slot_history_title(&slot),
                    category_id: slot.category_id.clone(),
                    started_at_epoch_millis: active_segment_start,
                    ended_at_epoch_millis: transition_at,
                    duration_millis: remaining_in_phase,
                });
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
    }

    slot.accumulated_millis = accumulated_millis;
    slot.running_since_epoch_millis = Some(active_segment_start);
    slot.micro_break_phase = phase;
    slot.micro_break_cycle_index = cycle_index;
    slot.micro_break_phase_progress_millis = phase_progress;
    if let Some(transition_at) = latest_transition_at {
        slot.updated_at = transition_at.max(slot.updated_at);
    }
    (slot, sessions)
}

fn pause_slot_micro_break(mut slot: TimerSlot, now: i64) -> (TimerSlot, Option<TimerSession>) {
    let Some(running_since) = slot.running_since_epoch_millis else {
        return (slot, None);
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
            id: generated_timer_session_id(slot.id, running_since, now),
            slot_id: slot.id,
            slot_title: timer_slot_history_title(&slot),
            category_id: slot.category_id.clone(),
            started_at_epoch_millis: running_since,
            ended_at_epoch_millis: now,
            duration_millis: duration,
        });
        slot.accumulated_millis =
            sanitize_tracked_duration(slot.accumulated_millis.saturating_add(duration));
        slot.running_since_epoch_millis = None;
        slot.micro_break_phase_progress_millis = slot
            .micro_break_phase_progress_millis
            .saturating_add(duration)
            .clamp(0, phase_target);
        slot.updated_at = now;
        (slot, session)
    } else {
        slot.running_since_epoch_millis = None;
        slot.micro_break_phase_progress_millis = slot
            .micro_break_phase_progress_millis
            .saturating_add(elapsed)
            .clamp(0, MICRO_BREAK_REST_MILLIS);
        slot.updated_at = now;
        (slot, None)
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

fn generated_timer_session_id(slot_id: i32, started_at: i64, ended_at: i64) -> String {
    format!("session-{slot_id}-{started_at}-{ended_at}")
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

fn sanitize_timestamp(value: i64, now: i64) -> i64 {
    value.clamp(0, now.max(0))
}

fn sanitize_tracked_duration(value: i64) -> i64 {
    value.clamp(0, MAX_TRACKED_DURATION_MILLIS)
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

fn normalized_category_name(value: &str) -> String {
    let trimmed = value.trim();
    match trimmed {
        "宸ヤ綔" => "工作".to_string(),
        "瀛︿範" => "学习".to_string(),
        "杩愬姩" => "运动".to_string(),
        "鐢熸椿" => "生活".to_string(),
        "" => "未分类".to_string(),
        _ => trimmed.to_string(),
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

fn compact_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
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

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
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

fn normalize_note_for_save(
    data: &AppData,
    mut note: NoteEntry,
    existing: Option<&NoteEntry>,
    timestamp: i64,
) -> NoteEntry {
    let merged_attachments = merge_note_attachments(
        existing
            .map(|note| note.attachments.clone())
            .unwrap_or_default(),
        note.attachments.clone(),
    );
    if note.revisions.is_empty() {
        note.revisions = existing
            .map(|note| note.revisions.clone())
            .unwrap_or_default();
    }
    note.attachments = merged_attachments;
    let resolved_document = note.resolved_document();
    let safe_folder_id = note.folder_id.take().filter(|folder_id| {
        data.note_folders
            .iter()
            .any(|folder| folder.id == *folder_id)
    });
    let created_at = existing
        .map(|note| note.created_at_epoch_millis)
        .unwrap_or_else(|| {
            if note.created_at_epoch_millis > 0 {
                note.created_at_epoch_millis
            } else {
                timestamp
            }
        });
    let deleted_at = note
        .deleted_at_epoch_millis
        .or_else(|| existing.and_then(|note| note.deleted_at_epoch_millis));
    note.title = note.title.trim().to_string();
    note.content = resolved_document.storage_content();
    note.document = resolved_document;
    note.folder_id = safe_folder_id;
    note.created_at_epoch_millis = created_at;
    note.updated_at_epoch_millis = timestamp;
    note.deleted_at_epoch_millis = deleted_at;
    note.revisions = build_note_revisions(existing, &note, timestamp);
    note
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
) -> Vec<NoteRevisionSnapshot> {
    let should_capture = previous
        .map(|previous| should_capture_revision(previous, next, timestamp))
        .unwrap_or(false);
    let snapshot = previous.filter(|_| should_capture).map(|previous| {
        previous.snapshot_for_history(if previous.updated_at_epoch_millis > 0 {
            previous.updated_at_epoch_millis
        } else {
            timestamp
        })
    });
    let mut base = next.revisions.clone();
    base = distinct_by(base, |revision| revision.id.clone());
    base.sort_by_key(|revision| Reverse(revision.captured_at_epoch_millis));
    if let Some(snapshot) = snapshot {
        std::iter::once(snapshot.clone())
            .chain(base.into_iter().filter(|revision| {
                revision.id != snapshot.id
                    && previous
                        .map(|previous| !revision.matches_note(previous))
                        .unwrap_or(true)
            }))
            .take(MAX_NOTE_REVISION_COUNT)
            .collect()
    } else {
        base.into_iter().take(MAX_NOTE_REVISION_COUNT).collect()
    }
}

fn should_capture_revision(previous: &NoteEntry, next: &NoteEntry, timestamp: i64) -> bool {
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
    collapse_repeated_blank_paragraphs_for_app_data(&remove_matching_image_tags_for_app_data(
        &remove_matching_figure_blocks_for_app_data(html, attachment_id),
        attachment_id,
    ))
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
    let lower = html.to_ascii_lowercase();
    let attr = attr_name.to_ascii_lowercase();
    let mut values = Vec::new();
    let mut search_start = 0usize;
    while let Some(relative_index) = lower[search_start..].find(&attr) {
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
        if cursor <= html.len() {
            values.push(html[value_start..cursor].to_string());
        }
        search_start = cursor.saturating_add(1);
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
    if end <= html.len() && html[start..end].eq_ignore_ascii_case(token) {
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

fn trimmed_or<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback
    } else {
        trimmed
    }
}

fn distinct_by<T, K>(values: Vec<T>, mut key: impl FnMut(&T) -> K) -> Vec<T>
where
    K: Eq + std::hash::Hash,
{
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(key(value)))
        .collect()
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

fn default_red() -> String {
    "red".to_string()
}

fn default_amber() -> String {
    "amber".to_string()
}

fn default_image_mime() -> String {
    "image/jpeg".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sanitizes_app_data_without_changing_default_category_generation() {
        let raw = json!({
            "schemaVersion": 1,
            "categories": [],
            "slots": []
        });
        assert_eq!(None, sanitize_app_data_json(&raw.to_string(), 1_000));
    }

    #[test]
    fn repairs_core_app_data_collections() {
        let raw = json!({
            "schemaVersion": 1,
            "categories": [
                {"id": "work", "name": "  宸ヤ綔  ", "accentSeed": "red"},
                {"id": "work", "name": "duplicate", "accentSeed": "blue"}
            ],
            "slots": [
                {
                    "id": 1,
                    "title": "  deep focus title that is longer than twenty four chars  ",
                    "categoryId": "missing",
                    "note": " memo ",
                    "accumulatedMillis": -1,
                    "runningSinceEpochMillis": 2_000,
                    "microBreakPhase": "FOCUS",
                    "microBreakCycleIndex": -2,
                    "microBreakPhaseProgressMillis": 999_999_999,
                    "updatedAt": 9_999
                }
            ],
            "slotOrder": [2, 2, 99, 1],
            "sessions": [
                {"id": "bad", "slotId": 99, "endedAtEpochMillis": 5},
                {"id": "ok", "slotId": 1, "slotTitle": "  finished task  ", "startedAtEpochMillis": 20, "endedAtEpochMillis": 10, "durationMillis": 100}
            ],
            "archivedTasks": [
                {"id": "arch", "originalSlotId": 1, "title": " done ", "accumulatedMillis": 50, "archivedAtEpochMillis": 2_000}
            ],
            "noteFolders": [
                {"id": "folder", "name": "  ", "createdAtEpochMillis": 50, "updatedAtEpochMillis": 10_000}
            ],
            "notes": [
                {
                    "id": "note",
                    "title": "  title  ",
                    "content": "\r\nbody\r\n",
                    "accentSeed": "bad",
                    "folderId": "folder",
                    "createdAtEpochMillis": 90,
                    "updatedAtEpochMillis": 80,
                    "attachments": [
                        {"id": "image", "fileName": "a b.jpg", "displayName": "", "mimeType": "", "width": -1, "height": -2, "sizeBytes": -3, "createdAtEpochMillis": 9_999}
                    ],
                    "document": {
                        "markdownEnabled": true,
                        "richTextEnabled": true,
                        "richTextPlainText": " plain ",
                        "blocks": [
                            {"id": "b1", "type": "IMAGE", "attachmentId": "missing"},
                            {"id": "b2", "type": "IMAGE", "attachmentId": "image", "caption": " cap "}
                        ]
                    },
                    "revisions": [
                        {"id": "rev", "title": " old ", "content": "\r\nold\r\n", "accentSeed": "teal", "attachmentIds": [" image ", "missing"], "capturedAtEpochMillis": 9_999}
                    ]
                }
            ],
            "financeProfile": {"activeIncomeMonthly": -1},
            "themeMode": "SYSTEM"
        });
        let sanitized_json = sanitize_app_data_json(&raw.to_string(), 1_000).expect("sanitized");
        let sanitized: serde_json::Value = serde_json::from_str(&sanitized_json).unwrap();

        assert_eq!(
            APP_DATA_SCHEMA_VERSION as i64,
            sanitized["schemaVersion"].as_i64().unwrap()
        );
        assert_eq!("工作", sanitized["categories"][0]["name"]);
        assert_eq!(14, sanitized["slots"].as_array().unwrap().len());
        assert_eq!(
            vec![2, 1, 3],
            sanitized["slotOrder"].as_array().unwrap()[0..3]
                .iter()
                .map(|value| value.as_i64().unwrap())
                .collect::<Vec<_>>()
        );
        assert_eq!("finished task", sanitized["sessions"][0]["slotTitle"]);
        assert_eq!(0, sanitized["sessions"][0]["durationMillis"]);
        assert_eq!("默认文件夹", sanitized["noteFolders"][0]["name"]);
        assert_eq!("title", sanitized["notes"][0]["title"]);
        assert_eq!("body", sanitized["notes"][0]["content"]);
        assert_eq!("amber", sanitized["notes"][0]["accentSeed"]);
        assert_eq!(
            "a_b.jpg",
            sanitized["notes"][0]["attachments"][0]["fileName"]
        );
        assert_eq!(false, sanitized["notes"][0]["document"]["markdownEnabled"]);
        assert_eq!(
            1,
            sanitized["notes"][0]["document"]["blocks"]
                .as_array()
                .unwrap()
                .len()
        );
        assert_eq!(
            "image",
            sanitized["notes"][0]["revisions"][0]["attachmentIds"][0]
        );
        assert_eq!(0, sanitized["financeProfile"]["activeIncomeMonthly"]);
    }

    #[test]
    fn delete_history_items_returns_sanitized_app_data_json() {
        let raw = json!({
            "schemaVersion": 9,
            "categories": [{"id": "work", "name": "Work", "accentSeed": "red"}],
            "slots": [{"id": 1, "updatedAt": 1}],
            "slotOrder": [1],
            "sessions": [
                {"id": "remove-session", "slotId": 1, "slotTitle": "old", "startedAtEpochMillis": 1, "endedAtEpochMillis": 5, "durationMillis": 4},
                {"id": "keep-session", "slotId": 1, "slotTitle": "new", "startedAtEpochMillis": 2, "endedAtEpochMillis": 6, "durationMillis": 4}
            ],
            "archivedTasks": [
                {"id": "remove-archive", "originalSlotId": 1, "title": "old", "archivedAtEpochMillis": 7},
                {"id": "keep-archive", "originalSlotId": 1, "title": "new", "archivedAtEpochMillis": 8}
            ],
            "themeMode": "SYSTEM"
        });

        let without_session = delete_session_app_data_json(&raw.to_string(), "remove-session", 10)
            .expect("session delete");
        let without_archive =
            delete_archived_task_app_data_json(&without_session, "remove-archive", 10)
                .expect("archive delete");
        let updated: serde_json::Value = serde_json::from_str(&without_archive).unwrap();

        assert_eq!(
            APP_DATA_SCHEMA_VERSION as i64,
            updated["schemaVersion"].as_i64().unwrap()
        );
        assert_eq!(14, updated["slots"].as_array().unwrap().len());
        assert_eq!(1, updated["sessions"].as_array().unwrap().len());
        assert_eq!("keep-session", updated["sessions"][0]["id"]);
        assert_eq!(1, updated["archivedTasks"].as_array().unwrap().len());
        assert_eq!("keep-archive", updated["archivedTasks"][0]["id"]);
    }

    #[test]
    fn history_deletion_summary_reports_before_after_counts() {
        let raw = json!({
            "schemaVersion": 9,
            "categories": [{"id": "work", "name": "Work", "accentSeed": "red"}],
            "slots": [{"id": 1, "updatedAt": 1}],
            "slotOrder": [1],
            "sessions": [
                {"id": "remove-session", "slotId": 1, "slotTitle": "old", "startedAtEpochMillis": 1, "endedAtEpochMillis": 5, "durationMillis": 4},
                {"id": "keep-session", "slotId": 1, "slotTitle": "new", "startedAtEpochMillis": 2, "endedAtEpochMillis": 6, "durationMillis": 4}
            ],
            "archivedTasks": [
                {"id": "remove-archive", "originalSlotId": 1, "title": "old", "archivedAtEpochMillis": 7},
                {"id": "keep-archive", "originalSlotId": 1, "title": "new", "archivedAtEpochMillis": 8}
            ],
            "themeMode": "SYSTEM"
        });

        assert_eq!(
            Some([2, 1, 1, 2, 1, 1]),
            history_deletion_summary_values(
                &raw.to_string(),
                "remove-session",
                "remove-archive",
                10
            )
        );
    }

    #[test]
    fn app_data_slot_and_finance_mutations_match_repository_updates() {
        let raw = json!({
            "schemaVersion": 9,
            "categories": [{"id": "work", "name": "Work", "accentSeed": "red"}],
            "slots": [{"id": 1, "updatedAt": 1}, {"id": 2, "updatedAt": 1}],
            "slotOrder": [1, 2],
            "financeProfile": {"activeIncomeMonthly": 0},
            "themeMode": "SYSTEM"
        });

        let titled_json =
            update_slot_title_app_data_json(&raw.to_string(), 1, "  Focus", 100).expect("title");
        let noted_json =
            update_slot_note_app_data_json(&titled_json, 1, "  Draft note", 120).expect("note");
        let categorized_json =
            set_slot_category_app_data_json(&noted_json, 1, "work", 140).expect("category");
        let ordered_json =
            set_slot_order_app_data_json(&categorized_json, &[2, 1, 2, 99], 160).expect("order");
        let financed_json = update_finance_profile_app_data_json(
            &ordered_json,
            r#"{"activeIncomeMonthly":5000,"assetIncomeMonthly":-1}"#,
            180,
        )
        .expect("finance");
        let updated: serde_json::Value = serde_json::from_str(&financed_json).unwrap();

        assert_eq!("Focus", updated["slots"][0]["title"]);
        assert_eq!("Draft note", updated["slots"][0]["note"]);
        assert_eq!("work", updated["slots"][0]["categoryId"]);
        assert_eq!(140, updated["slots"][0]["updatedAt"]);
        assert_eq!(2, updated["slotOrder"][0]);
        assert_eq!(1, updated["slotOrder"][1]);
        assert_eq!(5_000, updated["financeProfile"]["activeIncomeMonthly"]);
        assert_eq!(0, updated["financeProfile"]["assetIncomeMonthly"]);
    }

    #[test]
    fn repository_app_data_mutations_cover_category_timer_and_folder_state() {
        let raw = json!({
            "schemaVersion": 9,
            "categories": [{"id": "work", "name": "Work", "accentSeed": "red"}],
            "slots": [{"id": 1, "updatedAt": 1}, {"id": 2, "updatedAt": 1}],
            "slotOrder": [1, 2],
            "themeMode": "SYSTEM"
        });

        let categorized_json =
            add_category_and_assign_app_data_json(&raw.to_string(), 1, "deep", " Deep  Work ", 100)
                .expect("category");
        let started_json = start_slot_app_data_json(&categorized_json, 1, 1_000).expect("start");
        let paused_json = pause_slots_app_data_json(&started_json, &[1], 1_500).expect("pause");
        let reset_json = reset_slot_app_data_json(&paused_json, 1, 2_000).expect("reset");
        let folder_json =
            create_note_folder_app_data_json(&reset_json, "folder-1", " Logs ", 2_100)
                .expect("folder");
        let note_raw: serde_json::Value = serde_json::from_str(&folder_json).unwrap();
        let note_seed = json!({
            "schemaVersion": 9,
            "categories": note_raw["categories"],
            "slots": note_raw["slots"],
            "slotOrder": note_raw["slotOrder"],
            "sessions": note_raw["sessions"],
            "noteFolders": note_raw["noteFolders"],
            "notePreferences": note_raw["notePreferences"],
            "notes": [
                {"id": "note-1", "title": "Draft", "folderId": "folder-1", "pinned": true, "createdAtEpochMillis": 1, "updatedAtEpochMillis": 1}
            ],
            "themeMode": "SYSTEM"
        });
        let renamed_json =
            rename_note_folder_app_data_json(&note_seed.to_string(), "folder-1", "Archive", 2_200)
                .expect("rename");
        let sorted_json = set_note_sort_mode_app_data_json(&renamed_json, 3, 2_300).expect("sort");
        let moved_json =
            move_note_to_folder_app_data_json(&sorted_json, "note-1", "", 2_400).expect("move");
        let deleted_json =
            delete_note_app_data_json(&moved_json, "note-1", 2_500).expect("delete note");
        let restored_json =
            restore_note_app_data_json(&deleted_json, "note-1", 2_600).expect("restore note");
        let without_folder_json =
            delete_note_folder_app_data_json(&restored_json, "folder-1", 2_700)
                .expect("delete folder");
        let themed_json =
            set_theme_mode_app_data_json(&without_folder_json, 2, 2_800).expect("theme");
        let updated: serde_json::Value = serde_json::from_str(&themed_json).unwrap();

        assert_eq!("deep", updated["slots"][0]["categoryId"]);
        assert_eq!(0, updated["slots"][0]["accumulatedMillis"]);
        assert_eq!(
            serde_json::Value::Null,
            updated["slots"][0]["runningSinceEpochMillis"]
        );
        assert_eq!("session-1-1000-1500", updated["sessions"][0]["id"]);
        assert_eq!(500, updated["sessions"][0]["durationMillis"]);
        assert_eq!(0, updated["noteFolders"].as_array().unwrap().len());
        assert_eq!(
            serde_json::Value::Null,
            updated["notePreferences"]["selectedFolderId"]
        );
        assert_eq!("TITLE_ASC", updated["notePreferences"]["sortMode"]);
        assert_eq!(serde_json::Value::Null, updated["notes"][0]["folderId"]);
        assert_eq!(
            serde_json::Value::Null,
            updated["notes"][0]["deletedAtEpochMillis"]
        );
        assert_eq!("DARK", updated["themeMode"]);
    }

    #[test]
    fn repository_note_trash_mutations_remove_only_requested_notes() {
        let raw = json!({
            "schemaVersion": 9,
            "categories": [{"id": "work", "name": "Work", "accentSeed": "red"}],
            "slots": [{"id": 1, "updatedAt": 1}, {"id": 2, "updatedAt": 1}],
            "notes": [
                {"id": "active", "title": "Active", "createdAtEpochMillis": 1, "updatedAtEpochMillis": 1},
                {"id": "trash-1", "title": "Trash 1", "deletedAtEpochMillis": 5, "createdAtEpochMillis": 2, "updatedAtEpochMillis": 5},
                {"id": "trash-2", "title": "Trash 2", "deletedAtEpochMillis": 6, "createdAtEpochMillis": 3, "updatedAtEpochMillis": 6}
            ],
            "themeMode": "SYSTEM"
        });

        let permanently_deleted =
            delete_note_permanently_app_data_json(&raw.to_string(), "trash-1", 100)
                .expect("permanent");
        let updated: serde_json::Value = serde_json::from_str(&permanently_deleted).unwrap();
        assert_eq!(vec!["active", "trash-2"], note_ids_from_value(&updated));

        let emptied = empty_note_trash_app_data_json(&raw.to_string(), 120).expect("empty");
        let updated: serde_json::Value = serde_json::from_str(&emptied).unwrap();
        assert_eq!(vec!["active"], note_ids_from_value(&updated));
    }

    #[test]
    fn repository_note_revision_mutations_match_editor_paths() {
        let raw = json!({
            "schemaVersion": 9,
            "categories": [{"id": "work", "name": "Work", "accentSeed": "red"}],
            "slots": [{"id": 1, "updatedAt": 1}],
            "notes": [
                {
                    "id": "note-1",
                    "title": " Current ",
                    "content": "body",
                    "document": {
                        "blocks": [{"id": "text-1", "type": "TEXT", "text": "body"}]
                    },
                    "accentSeed": "red",
                    "pinned": false,
                    "createdAtEpochMillis": 1,
                    "updatedAtEpochMillis": 2,
                    "revisions": [
                        {
                            "id": "restore-1",
                            "title": "Old",
                            "content": "old body",
                            "document": {
                                "blocks": [{"id": "old-text", "type": "TEXT", "text": "old body"}]
                            },
                            "accentSeed": "blue",
                            "pinned": true,
                            "capturedAtEpochMillis": 3
                        }
                    ]
                }
            ],
            "themeMode": "SYSTEM"
        });

        let pinned_json =
            set_note_pinned_app_data_json(&raw.to_string(), "note-1", true, 100).expect("pin");
        let pinned: serde_json::Value = serde_json::from_str(&pinned_json).unwrap();
        assert_eq!(true, pinned["notes"][0]["pinned"]);
        assert_eq!(100, pinned["notes"][0]["updatedAtEpochMillis"]);

        let captured_json =
            capture_note_revision_app_data_json(&pinned_json, "note-1", 120).expect("capture");
        let captured: serde_json::Value = serde_json::from_str(&captured_json).unwrap();
        assert_eq!(
            2,
            captured["notes"][0]["revisions"].as_array().unwrap().len()
        );
        assert_eq!("Current", captured["notes"][0]["revisions"][0]["title"]);
        assert_eq!("body", captured["notes"][0]["revisions"][0]["content"]);

        let restored_json =
            restore_note_revision_app_data_json(&captured_json, "note-1", "restore-1", 140)
                .expect("restore");
        let restored: serde_json::Value = serde_json::from_str(&restored_json).unwrap();
        assert_eq!("Old", restored["notes"][0]["title"]);
        assert_eq!("old body", restored["notes"][0]["content"]);
        assert_eq!("blue", restored["notes"][0]["accentSeed"]);
        assert_eq!(true, restored["notes"][0]["pinned"]);
        assert_eq!(140, restored["notes"][0]["updatedAtEpochMillis"]);
        assert!(restored["notes"][0]["deletedAtEpochMillis"].is_null());
        assert!(restored["notes"][0]["revisions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|revision| revision["id"].as_str() != Some("restore-1")));
    }

    #[test]
    fn upsert_note_merges_attachments_folder_and_revision_plan() {
        let raw = json!({
            "schemaVersion": 9,
            "categories": [{"id": "work", "name": "Work", "accentSeed": "red"}],
            "slots": [{"id": 1, "updatedAt": 1}],
            "noteFolders": [{"id": "folder-1", "name": "Inbox", "createdAtEpochMillis": 1, "updatedAtEpochMillis": 1}],
            "notes": [{
                "id": "note-1",
                "title": "Old",
                "content": "old",
                "document": {"blocks": [{"id": "old-block", "type": "TEXT", "text": "old"}]},
                "attachments": [{"id": "old-image", "fileName": "old.jpg", "displayName": "Old", "createdAtEpochMillis": 1}],
                "createdAtEpochMillis": 10,
                "updatedAtEpochMillis": 20
            }],
            "themeMode": "SYSTEM"
        });
        let incoming = json!({
            "id": "note-1",
            "title": " New ",
            "content": "ignored",
            "document": {"blocks": [{"id": "new-block", "type": "TEXT", "text": "new body"}]},
            "folderId": "missing-folder",
            "attachments": [{"id": "new-image", "fileName": "new.jpg", "displayName": "New", "createdAtEpochMillis": 30}]
        });

        let updated_json =
            upsert_note_app_data_json(&raw.to_string(), &incoming.to_string(), 50).expect("upsert");
        let updated: serde_json::Value = serde_json::from_str(&updated_json).unwrap();
        let note = &updated["notes"][0];

        assert_eq!("note-1", note["id"]);
        assert_eq!("New", note["title"]);
        assert_eq!("new body", note["content"]);
        assert_eq!(10, note["createdAtEpochMillis"]);
        assert_eq!(50, note["updatedAtEpochMillis"]);
        assert!(note["folderId"].is_null());
        let attachment_ids = note["attachments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|attachment| attachment["id"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(vec!["new-image", "old-image"], attachment_ids);
        assert_eq!(1, note["revisions"].as_array().unwrap().len());
        assert_eq!("Old", note["revisions"][0]["title"]);
        assert_eq!("old", note["revisions"][0]["content"]);
    }

    #[test]
    fn note_folder_counts_ignore_trashed_and_unfiled_notes() {
        let raw = json!({
            "schemaVersion": 9,
            "categories": [{"id": "work", "name": "Work", "accentSeed": "red"}],
            "slots": [{"id": 1, "updatedAt": 1}],
            "notes": [
                {"id": "a", "folderId": "inbox", "createdAtEpochMillis": 1, "updatedAtEpochMillis": 1},
                {"id": "b", "folderId": "inbox", "createdAtEpochMillis": 2, "updatedAtEpochMillis": 2},
                {"id": "c", "folderId": "archive", "createdAtEpochMillis": 3, "updatedAtEpochMillis": 3},
                {"id": "d", "folderId": "archive", "deletedAtEpochMillis": 4, "createdAtEpochMillis": 4, "updatedAtEpochMillis": 4},
                {"id": "e", "createdAtEpochMillis": 5, "updatedAtEpochMillis": 5}
            ],
            "themeMode": "SYSTEM"
        });

        let pairs = note_folder_count_pairs(&raw.to_string()).expect("counts");

        assert_eq!(
            vec![("inbox".to_owned(), 2), ("archive".to_owned(), 1)],
            pairs
        );
    }

    #[test]
    fn note_visibility_indices_split_active_and_trashed_notes() {
        let raw = json!({
            "schemaVersion": 9,
            "categories": [{"id": "work", "name": "Work", "accentSeed": "red"}],
            "slots": [{"id": 1, "updatedAt": 1}],
            "notes": [
                {"id": "active-1", "createdAtEpochMillis": 1, "updatedAtEpochMillis": 1},
                {"id": "trash-1", "deletedAtEpochMillis": 2, "createdAtEpochMillis": 2, "updatedAtEpochMillis": 2},
                {"id": "active-2", "createdAtEpochMillis": 3, "updatedAtEpochMillis": 3}
            ],
            "themeMode": "SYSTEM"
        });

        assert_eq!(
            Some(vec![0, 2]),
            note_visibility_indices(&raw.to_string(), false)
        );
        assert_eq!(
            Some(vec![1]),
            note_visibility_indices(&raw.to_string(), true)
        );
    }

    #[test]
    fn note_blank_draft_json_matches_title_document_and_attachment_rules() {
        let blank = json!({
            "title": " ",
            "content": " ",
            "document": {"blocks": []},
            "attachments": []
        });
        let text = json!({
            "title": " ",
            "document": {"blocks": [{"id": "text", "type": "TEXT", "text": "body"}]},
            "attachments": []
        });
        let image = json!({
            "title": " ",
            "document": {"blocks": []},
            "attachments": [{"id": "image", "fileName": "a.jpg"}]
        });

        assert_eq!(Some(true), is_note_blank_draft_json(&blank.to_string(), 10));
        assert_eq!(Some(false), is_note_blank_draft_json(&text.to_string(), 10));
        assert_eq!(
            Some(false),
            is_note_blank_draft_json(&image.to_string(), 10)
        );
    }

    #[test]
    fn archive_and_restore_slot_app_data_json_round_trips_task_state() {
        let raw = json!({
            "schemaVersion": 9,
            "categories": [{"id": "work", "name": "Work", "accentSeed": "red"}],
            "slots": [
                {
                    "id": 1,
                    "title": "Deep Work",
                    "categoryId": "work",
                    "note": "draft",
                    "accumulatedMillis": 90_000,
                    "microBreakPhase": "FOCUS",
                    "updatedAt": 5
                },
                {"id": 2, "updatedAt": 5}
            ],
            "slotOrder": [1, 2],
            "themeMode": "SYSTEM"
        });

        let archived_json =
            archive_slot_app_data_json(&raw.to_string(), 1, "archive-1", 100).expect("archive");
        let archived: serde_json::Value = serde_json::from_str(&archived_json).unwrap();
        assert_eq!("archive-1", archived["archivedTasks"][0]["id"]);
        assert_eq!("Deep Work", archived["archivedTasks"][0]["title"]);
        assert_eq!("", archived["slots"][0]["title"]);
        assert_eq!(0, archived["slots"][0]["accumulatedMillis"]);

        let restored_json =
            restore_archived_task_app_data_json(&archived_json, "archive-1", 120).expect("restore");
        let restored: serde_json::Value = serde_json::from_str(&restored_json).unwrap();
        assert_eq!(0, restored["archivedTasks"].as_array().unwrap().len());
        assert_eq!("Deep Work", restored["slots"][0]["title"]);
        assert_eq!("work", restored["slots"][0]["categoryId"]);
        assert_eq!("draft", restored["slots"][0]["note"]);
        assert_eq!(90_000, restored["slots"][0]["accumulatedMillis"]);
        assert_eq!(120, restored["slots"][0]["updatedAt"]);
    }

    #[test]
    fn restore_uses_first_blank_slot_when_original_slot_is_occupied() {
        let raw = json!({
            "schemaVersion": 9,
            "categories": [{"id": "work", "name": "Work", "accentSeed": "red"}],
            "slots": [
                {"id": 1, "title": "Busy", "updatedAt": 5},
                {"id": 2, "updatedAt": 5}
            ],
            "archivedTasks": [
                {"id": "archive-1", "originalSlotId": 1, "title": "Restored", "accumulatedMillis": 10, "archivedAtEpochMillis": 20}
            ],
            "themeMode": "SYSTEM"
        });

        let restored_json = restore_archived_task_app_data_json(&raw.to_string(), "archive-1", 120)
            .expect("restore");
        let restored: serde_json::Value = serde_json::from_str(&restored_json).unwrap();

        assert_eq!("Busy", restored["slots"][0]["title"]);
        assert_eq!("Restored", restored["slots"][1]["title"]);
        assert_eq!(0, restored["archivedTasks"].as_array().unwrap().len());
    }

    #[test]
    fn note_image_import_policy_and_cleanup_match_repository_branches() {
        assert_eq!(0, note_image_import_policy(true, false, 0, 12));
        assert_eq!(1, note_image_import_policy(false, false, 0, 12));
        assert_eq!(1, note_image_import_policy(true, true, 0, 12));
        assert_eq!(2, note_image_import_policy(true, false, 12, 12));
        assert!(should_delete_unattached_imported_note_image(true, false));
        assert!(!should_delete_unattached_imported_note_image(true, true));
        assert!(!should_delete_unattached_imported_note_image(false, false));
    }

    #[test]
    fn repository_note_attachment_import_adds_document_image_block() {
        let raw = json!({
            "schemaVersion": 9,
            "categories": [{"id": "work", "name": "Work", "accentSeed": "red"}],
            "slots": [{"id": 1, "updatedAt": 1}],
            "notes": [
                {
                    "id": "note-1",
                    "title": "Daily",
                    "content": "body",
                    "createdAtEpochMillis": 1,
                    "updatedAtEpochMillis": 1
                }
            ],
            "themeMode": "SYSTEM"
        });
        let attachment = json!({
            "id": "image-1",
            "fileName": "new photo.jpg",
            "displayName": "Receipt",
            "mimeType": "image/jpeg",
            "width": 640,
            "height": 480,
            "sizeBytes": 2048,
            "createdAtEpochMillis": 90
        });

        let updated_json = import_note_image_attachment_app_data_json(
            &raw.to_string(),
            "note-1",
            &attachment.to_string(),
            100,
        )
        .expect("import");
        let updated: serde_json::Value = serde_json::from_str(&updated_json).unwrap();

        assert_eq!("image-1", updated["notes"][0]["attachments"][0]["id"]);
        assert_eq!(
            "new_photo.jpg",
            updated["notes"][0]["attachments"][0]["fileName"]
        );
        assert_eq!(
            "IMAGE",
            updated["notes"][0]["document"]["blocks"][1]["type"]
        );
        assert_eq!(
            "image-1",
            updated["notes"][0]["document"]["blocks"][1]["attachmentId"]
        );
        assert_eq!(100, updated["notes"][0]["updatedAtEpochMillis"]);
        assert!(updated["notes"][0]["content"]
            .as_str()
            .unwrap()
            .contains("Receipt"));
    }

    #[test]
    fn repository_note_attachment_delete_removes_image_and_rich_text_references() {
        let raw = json!({
            "schemaVersion": 9,
            "categories": [{"id": "work", "name": "Work", "accentSeed": "red"}],
            "slots": [{"id": 1, "updatedAt": 1}],
            "notes": [
                {
                    "id": "note-1",
                    "title": "Daily",
                    "createdAtEpochMillis": 1,
                    "updatedAtEpochMillis": 1,
                    "attachments": [
                        {"id": "image-1", "fileName": "one.jpg", "displayName": "One"},
                        {"id": "image-2", "fileName": "two.jpg", "displayName": "Two"}
                    ],
                    "document": {
                        "richTextEnabled": true,
                        "richTextPlainText": "body",
                        "blocks": [
                            {
                                "id": "text",
                                "type": "TEXT",
                                "text": "<p>body</p><figure data-note-image=\"image-1\"><img src=\"note-image://image-1\"></figure><p><br></p><p><br></p>"
                            },
                            {"id": "image-one", "type": "IMAGE", "attachmentId": "image-1", "caption": "One"},
                            {"id": "image-two", "type": "IMAGE", "attachmentId": "image-2", "caption": "Two"}
                        ]
                    }
                }
            ],
            "themeMode": "SYSTEM"
        });

        let updated_json =
            delete_note_attachment_app_data_json(&raw.to_string(), "note-1", "image-1", 200)
                .expect("delete");
        let updated: serde_json::Value = serde_json::from_str(&updated_json).unwrap();

        assert_eq!(
            vec!["image-2"],
            updated["notes"][0]["attachments"]
                .as_array()
                .unwrap()
                .iter()
                .map(|attachment| attachment["id"].as_str().unwrap())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            2,
            updated["notes"][0]["document"]["blocks"]
                .as_array()
                .unwrap()
                .len()
        );
        assert_eq!(
            "image-2",
            updated["notes"][0]["document"]["blocks"][1]["attachmentId"]
        );
        assert!(!updated["notes"][0]["document"]["blocks"][0]["text"]
            .as_str()
            .unwrap()
            .contains("image-1"));
        assert_eq!(200, updated["notes"][0]["updatedAtEpochMillis"]);
    }

    #[test]
    fn repository_persist_plan_preserves_backup_repair_branches() {
        assert_eq!(
            PERSIST_PLAN_COPY_STATE_TO_BACKUP | PERSIST_PLAN_ENSURE_BACKUP_AFTER_STATE,
            repository_persist_plan_flags(true, true)
        );
        assert_eq!(
            PERSIST_PLAN_COPY_STATE_TO_BACKUP | PERSIST_PLAN_ENSURE_BACKUP_AFTER_STATE,
            repository_persist_plan_flags(true, false)
        );
        assert_eq!(
            PERSIST_PLAN_WRITE_BACKUP_FROM_ENCODED | PERSIST_PLAN_ENSURE_BACKUP_AFTER_STATE,
            repository_persist_plan_flags(false, false)
        );
        assert_eq!(
            PERSIST_PLAN_ENSURE_BACKUP_AFTER_STATE,
            repository_persist_plan_flags(false, true)
        );
    }

    fn note_ids_from_value(value: &serde_json::Value) -> Vec<&str> {
        value["notes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|note| note["id"].as_str().unwrap())
            .collect()
    }
}
