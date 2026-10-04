// v0.0.4 - Keep current AppData separate from compact retained attachment identities.
// v0.0.3 - Reconcile ciphertext-bound files while preserving both attachment deletion kinds.
// v0.0.2 - Synchronize private metadata and let reference updates precede deferred deletions.
// v0.0.1 - Keep unresolved attachment deletion conflicts separate from confirmed remote deletion.
// v2.22.42 - Accept verified remote media during document upload preflight.
// v2.22.34 - Defer only account-attested legacy media gaps during data sync.
//! Desktop reconciliation for account-scoped note image attachments.
//!
//! AppData remains the source of truth for attachment references and media
//! tombstones. The server manifest is only used to transfer matching bytes.
//! A hash mismatch is deliberately reported as a conflict: this module never
//! chooses one copy, overwrites the local blob, or physically prunes old blobs.

#![cfg(not(target_os = "android"))]

use crate::desktop_note_media::{
    DesktopNoteMediaError, DesktopNoteMediaStore, NoteMediaManifestEntry,
};
use crate::sync_core::{
    desktop_media_delete, desktop_media_download, desktop_media_manifest, desktop_media_upload,
    LegacyMediaReference, MediaManifestItem, SyncClientResult,
};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
#[path = "desktop_private_media_sync.rs"]
mod private_media;

const NOTE_ATTACHMENT_TOMBSTONE: &str = "noteAttachment";
const NOTE_MEDIA_TOMBSTONE: &str = "noteMedia";

/// Controls which side may supply note-media mutations during reconciliation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DesktopNoteMediaSyncDirection {
    /// Upload local blobs and tombstones, download missing blobs, and apply
    /// remote tombstones. Hash disagreements remain conflicts.
    Bidirectional,
    /// Treat the account workspace as the only source of incoming media.
    /// Local blobs and tombstones are never sent.
    DownloadOnly,
    /// Send local blobs and tombstones without downloading or applying remote
    /// tombstones. Remote content or tombstones that disagree are conflicts.
    UploadOnly,
}

impl DesktopNoteMediaSyncDirection {
    fn sends_local_mutations(self) -> bool {
        !matches!(self, Self::DownloadOnly)
    }

    fn applies_remote_deletions(self) -> bool {
        !matches!(self, Self::UploadOnly)
    }

    fn receives_remote_content(self) -> bool {
        !matches!(self, Self::UploadOnly)
    }
}

/// Structured outcome of one complete desktop note-media reconciliation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DesktopNoteMediaSyncSummary {
    pub uploaded: usize,
    pub downloaded: usize,
    pub conflicts: usize,
    pub failures: usize,
    pub legacy_missing: usize,
    /// A retained reference postpones DELETE until the new AppData is committed.
    /// This must not block that commit or pretend the remote deletion happened.
    pub deferred_reference_deletions: usize,
    pub private_media_pending: usize,
    pub recovery_sources_pending: usize,
    pub verified_private_media: Vec<crate::private_media_protocol::VerifiedPrivateMediaReply>,
    pub restore_required: bool,
    pub server_generation_rollback: bool,
    pub quota_rejected: bool,
    pub current_generation: i64,
    pub resolved_sha256_by_attachment_id: BTreeMap<String, String>,
    pub remote_deleted_at_by_attachment_id: BTreeMap<String, i64>,
    /// Safe diagnostic messages intended for the local Windows status panel.
    /// Authentication secrets and media bytes are never included.
    pub failure_messages: Vec<String>,
}

impl DesktopNoteMediaSyncSummary {
    pub fn message_suffix(&self) -> String {
        if self.restore_required {
            return "；服务器数据代际已变化，正在重新恢复完整快照".to_string();
        }
        if self.server_generation_rollback {
            return "；服务器数据代际异常回退，附件未改动".to_string();
        }
        if self.quota_rejected {
            return "；服务器历史空间已满，附件未写入，本机文件保持完整".to_string();
        }
        let mut message = if self.uploaded == 0
            && self.downloaded == 0
            && self.conflicts == 0
            && self.failures == 0
        {
            String::new()
        } else {
            format!(
                "；附件 上传{} 下载{} 冲突{} 失败{}",
                self.uploaded, self.downloaded, self.conflicts, self.failures
            )
        };
        if self.legacy_missing > 0 {
            message.push_str(&format!("；{} 个历史附件待恢复", self.legacy_missing));
        }
        if self.deferred_reference_deletions > 0 {
            message.push_str(&format!(
                "；{} 个附件等待引用更新后删除",
                self.deferred_reference_deletions
            ));
        }
        if self.private_media_pending > 0 {
            message.push_str(&format!(
                "；{} 条加密笔记的附件引用待确认",
                self.private_media_pending
            ));
        }
        if self.recovery_sources_pending > 0 {
            message.push_str(&format!(
                "；{} 份恢复副本待核验",
                self.recovery_sources_pending
            ));
        }
        message
    }

    fn record_failure(&mut self, message: impl Into<String>) {
        self.failures += 1;
        let message = message.into();
        if !message.trim().is_empty() && self.failure_messages.len() < 16 {
            self.failure_messages.push(message);
        }
    }
}

/// Reconciles all current and historical note-image references for one bound
/// account workspace.
///
/// The identity tuple is forwarded unchanged to every hardened sync-core media
/// operation. Local `noteMedia` tombstones are sent before any upload so a
/// stale blob cannot be resurrected. The function never calls
/// `DesktopNoteMediaStore::delete_blob`.
#[allow(clippy::too_many_arguments)]
pub fn sync_desktop_note_media(
    store: &DesktopNoteMediaStore,
    app_data_json: &str,
    server_url: &str,
    token: &str,
    user_id: &str,
    server_instance_id: &str,
    account_namespace: &str,
    acknowledged_generation: i64,
    restore_receipt: &str,
) -> DesktopNoteMediaSyncSummary {
    if crate::runtime::cancellation::check_current().is_err() {
        let mut summary = DesktopNoteMediaSyncSummary::default();
        summary.record_failure("附件同步已取消");
        return summary;
    }
    sync_desktop_note_media_with_direction(
        store,
        app_data_json,
        server_url,
        token,
        user_id,
        server_instance_id,
        account_namespace,
        acknowledged_generation,
        restore_receipt,
        DesktopNoteMediaSyncDirection::Bidirectional,
    )
}

/// Reconciles note media while limiting mutations to the requested direction.
///
/// `DownloadOnly` never sends local blobs or tombstones. `UploadOnly` never
/// downloads blobs or writes remote tombstones into the returned summary.
/// `Bidirectional` preserves [`sync_desktop_note_media`] semantics.
#[allow(clippy::too_many_arguments)]
pub fn sync_desktop_note_media_with_direction(
    store: &DesktopNoteMediaStore,
    app_data_json: &str,
    server_url: &str,
    token: &str,
    user_id: &str,
    server_instance_id: &str,
    account_namespace: &str,
    acknowledged_generation: i64,
    restore_receipt: &str,
    direction: DesktopNoteMediaSyncDirection,
) -> DesktopNoteMediaSyncSummary {
    sync_desktop_note_media_with_private_references(
        store,
        app_data_json,
        server_url,
        token,
        user_id,
        server_instance_id,
        account_namespace,
        acknowledged_generation,
        restore_receipt,
        direction,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn sync_desktop_note_media_with_private_references(
    store: &DesktopNoteMediaStore,
    app_data_json: &str,
    server_url: &str,
    token: &str,
    user_id: &str,
    server_instance_id: &str,
    account_namespace: &str,
    acknowledged_generation: i64,
    restore_receipt: &str,
    direction: DesktopNoteMediaSyncDirection,
    policy: Option<&crate::desktop_state_store::DesktopPrivacyPolicy>,
) -> DesktopNoteMediaSyncSummary {
    sync_desktop_note_media_with_reference_index(
        store,
        app_data_json,
        server_url,
        token,
        user_id,
        server_instance_id,
        account_namespace,
        acknowledged_generation,
        restore_receipt,
        direction,
        policy,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn sync_desktop_note_media_with_reference_index(
    store: &DesktopNoteMediaStore,
    app_data_json: &str,
    server_url: &str,
    token: &str,
    user_id: &str,
    server_instance_id: &str,
    account_namespace: &str,
    acknowledged_generation: i64,
    restore_receipt: &str,
    direction: DesktopNoteMediaSyncDirection,
    policy: Option<&crate::desktop_state_store::DesktopPrivacyPolicy>,
    references: Option<&crate::desktop_private_media_index::DesktopPrivateMediaReferences>,
) -> DesktopNoteMediaSyncSummary {
    if crate::runtime::cancellation::check_current().is_err() {
        let mut summary = DesktopNoteMediaSyncSummary::default();
        summary.record_failure("附件同步已取消");
        return summary;
    }
    let owned_references;
    let references = if policy.is_some() {
        let references = match references {
            Some(value) => value,
            None => {
                owned_references = match crate::desktop_private_media_index::DesktopPrivateMediaReferences::from_snapshot(app_data_json) {
                    Ok(value) => value,
                    Err(error) => { let mut result=DesktopNoteMediaSyncSummary::default(); result.record_failure(error); return result; }
                };
                &owned_references
            }
        };
        if !references.matches_snapshot(app_data_json) {
            let mut result = DesktopNoteMediaSyncSummary::default();
            result.record_failure("附件引用对应的数据已变化，请重新同步");
            return result;
        }
        Some(references)
    } else {
        None
    };
    let app_data = match serde_json::from_str::<MediaAppData>(app_data_json) {
        Ok(value) => value,
        Err(error) => {
            let mut summary = DesktopNoteMediaSyncSummary::default();
            summary.record_failure(format!("无法解析附件索引：{error}"));
            return summary;
        }
    };

    let manifest_result = desktop_media_manifest(
        server_url,
        user_id,
        token,
        server_instance_id,
        account_namespace,
        acknowledged_generation,
        restore_receipt,
    );
    let mut summary = DesktopNoteMediaSyncSummary::default();
    if media_sync_cancelled(&mut summary) || absorb_terminal_result(&manifest_result, &mut summary)
    {
        return summary;
    }
    if !manifest_result.ok {
        summary.record_failure(operation_message(&manifest_result, "无法读取附件清单"));
        return summary;
    }
    let effective_policy = if let Some(policy) = policy {
        let Some(effective) = private_media::exchange(
            policy,
            references.expect("private policy has a reference index"),
            &manifest_result,
            server_url,
            token,
            user_id,
            server_instance_id,
            account_namespace,
            acknowledged_generation,
            restore_receipt,
            direction,
            &mut summary,
        ) else {
            return summary;
        };
        Some(effective)
    } else {
        None
    };

    let server_by_id = manifest_result
        .media_items
        .iter()
        .filter(|item| !item.attachment_id.trim().is_empty())
        .cloned()
        .map(|item| (item.attachment_id.clone(), item))
        .collect::<BTreeMap<_, _>>();
    let mut plan = build_reconcile_plan(&app_data, &server_by_id);
    if let Some(policy) = effective_policy.as_ref() {
        private_media::include_content_plan(
            &mut plan,
            references.expect("private policy has a reference index"),
            policy,
            &server_by_id,
            direction,
            &mut summary,
        );
    }
    let legacy_by_id = manifest_result
        .legacy_media_references
        .iter()
        .map(|item| (item.attachment_id.as_str(), item))
        .collect::<BTreeMap<_, _>>();

    // Deletions must cross the restore barrier before any possible upload.
    for deletion in plan
        .media_deletions
        .iter()
        .filter(|_| direction.sends_local_mutations())
    {
        if media_sync_cancelled(&mut summary) {
            return summary;
        }
        let server_item = server_by_id.get(&deletion.attachment_id);
        if let Some(item) = server_item {
            if item.deleted_at_epoch_millis >= deletion.deleted_at_epoch_millis {
                if direction.applies_remote_deletions() {
                    summary.remote_deleted_at_by_attachment_id.insert(
                        deletion.attachment_id.clone(),
                        item.deleted_at_epoch_millis
                            .max(deletion.deleted_at_epoch_millis),
                    );
                }
                continue;
            }
            if item.deleted_at_epoch_millis <= 0
                && item.updated_at_epoch_millis > deletion.deleted_at_epoch_millis
            {
                summary.conflicts += 1;
                continue;
            }
        }

        let result = desktop_media_delete(
            server_url,
            user_id,
            token,
            server_instance_id,
            account_namespace,
            acknowledged_generation,
            restore_receipt,
            &deletion.attachment_id,
            deletion.deleted_at_epoch_millis,
        );
        if absorb_terminal_result(&result, &mut summary) {
            return summary;
        }
        if result.ok {
            continue;
        }
        if matches!(
            result.mode.as_str(),
            "media_referenced" | "media_references_unresolved"
        ) {
            summary.deferred_reference_deletions += 1;
            continue;
        }
        if is_conflict_result(&result) {
            summary.conflicts += 1;
            if let Some(item) = result.media_item.as_ref() {
                if direction.applies_remote_deletions() && item.deleted_at_epoch_millis > 0 {
                    summary
                        .remote_deleted_at_by_attachment_id
                        .insert(deletion.attachment_id.clone(), item.deleted_at_epoch_millis);
                }
            }
        } else {
            summary.record_failure(operation_message(&result, "无法同步附件删除记录"));
        }
    }

    // A damaged unrelated local blob must never prevent durable deletion
    // tombstones from crossing the restore barrier above.
    let mut local_by_id = BTreeMap::new();
    let mut unreadable_local_ids = BTreeSet::new();
    for attachment in &plan.attachments {
        if media_sync_cancelled(&mut summary) {
            return summary;
        }
        match store.manifest_entry_for(&attachment.id) {
            Ok(Some(entry)) => {
                local_by_id.insert(attachment.id.clone(), entry);
            }
            Ok(None) => {}
            Err(DesktopNoteMediaError::PendingImportCorrupt(_))
                if direction.receives_remote_content()
                    && server_by_id.contains_key(&attachment.id) =>
            {
                // A crash can publish a verified final blob immediately before
                // its metadata sidecar. Treat that recoverable half-commit as
                // locally absent only when the authenticated server manifest
                // can drive a fresh download. `write_download_blob` rechecks
                // the existing bytes before it recreates the sidecar.
            }
            Err(error) => {
                unreadable_local_ids.insert(attachment.id.clone());
                record_store_read_error(&mut summary, error);
            }
        }
    }

    for attachment in &plan.attachments {
        if media_sync_cancelled(&mut summary) {
            return summary;
        }
        if unreadable_local_ids.contains(&attachment.id) {
            continue;
        }
        let local = local_by_id.get(&attachment.id);
        let server = server_by_id.get(&attachment.id);
        let action = decide_attachment_action(
            direction,
            attachment,
            local,
            server,
            plan.media_deletion_by_attachment_id
                .get(&attachment.id)
                .copied(),
            plan.explicitly_restored_attachment_ids
                .contains(&attachment.id),
        );

        match action {
            AttachmentAction::Skip => {}
            AttachmentAction::MissingEverywhere => {
                if legacy_gap_matches(
                    attachment,
                    legacy_by_id.get(attachment.id.as_str()).copied(),
                ) {
                    summary.legacy_missing += 1;
                } else {
                    record_missing_attachment(&mut summary);
                }
            }
            AttachmentAction::RemoteDeleted(deleted_at) => {
                summary
                    .remote_deleted_at_by_attachment_id
                    .insert(attachment.id.clone(), deleted_at);
            }
            AttachmentAction::Conflict => summary.conflicts += 1,
            AttachmentAction::Resolved(sha256) => {
                summary
                    .resolved_sha256_by_attachment_id
                    .insert(attachment.id.clone(), sha256);
            }
            AttachmentAction::Upload { restore_deleted } => {
                let Some(local) = local else {
                    summary.record_failure("本机附件索引在同步期间发生变化");
                    continue;
                };
                let content = match store.read_blob(&attachment.id, &local.sha256, local.size_bytes)
                {
                    Ok(content) => content,
                    Err(error) => {
                        record_store_read_error(&mut summary, error);
                        continue;
                    }
                };
                let mime_type = if attachment.mime_type.trim().is_empty() {
                    local.mime_type.as_str()
                } else {
                    attachment.mime_type.trim()
                };
                let result = desktop_media_upload(
                    server_url,
                    user_id,
                    token,
                    server_instance_id,
                    account_namespace,
                    acknowledged_generation,
                    restore_receipt,
                    &attachment.id,
                    &local.sha256,
                    mime_type,
                    content.len() as i64,
                    attachment.revision(),
                    &content,
                    restore_deleted,
                );
                if absorb_terminal_result(&result, &mut summary) {
                    return summary;
                }
                if result.ok {
                    summary.uploaded += 1;
                    summary
                        .resolved_sha256_by_attachment_id
                        .insert(attachment.id.clone(), local.sha256.to_ascii_lowercase());
                } else if is_conflict_result(&result) {
                    summary.conflicts += 1;
                    if let Some(item) = result.media_item.as_ref() {
                        if direction.applies_remote_deletions() && item.deleted_at_epoch_millis > 0
                        {
                            summary
                                .remote_deleted_at_by_attachment_id
                                .insert(attachment.id.clone(), item.deleted_at_epoch_millis);
                        }
                    }
                } else {
                    summary.record_failure(operation_message(&result, "无法上传附件"));
                }
            }
            AttachmentAction::Download => {
                let Some(manifest_item) = server else {
                    summary.record_failure("服务器附件索引在同步期间发生变化");
                    continue;
                };
                let result = desktop_media_download(
                    server_url,
                    user_id,
                    token,
                    server_instance_id,
                    account_namespace,
                    acknowledged_generation,
                    restore_receipt,
                    &attachment.id,
                );
                if absorb_terminal_result(&result, &mut summary) {
                    return summary;
                }
                if !result.ok {
                    if is_conflict_result(&result) {
                        summary.conflicts += 1;
                    } else {
                        summary.record_failure(operation_message(&result, "无法下载附件"));
                    }
                    continue;
                }
                let Some(downloaded_item) = result.media_item.as_ref() else {
                    summary.record_failure("服务器未返回附件元数据");
                    continue;
                };
                if downloaded_item.attachment_id != manifest_item.attachment_id
                    || !downloaded_item
                        .sha256
                        .eq_ignore_ascii_case(&manifest_item.sha256)
                    || downloaded_item.size_bytes != manifest_item.size_bytes
                    || downloaded_item.deleted_at_epoch_millis > 0
                {
                    summary.conflicts += 1;
                    continue;
                }
                let content = match BASE64_STANDARD.decode(result.media_content_base64.trim()) {
                    Ok(content) => content,
                    Err(_) => {
                        summary.record_failure("服务器附件内容编码无效");
                        continue;
                    }
                };
                if media_sync_cancelled(&mut summary) {
                    return summary;
                }
                match store.write_download_blob(
                    &attachment.id,
                    &downloaded_item.sha256,
                    downloaded_item.size_bytes,
                    &downloaded_item.mime_type,
                    downloaded_item.updated_at_epoch_millis,
                    &content,
                ) {
                    Ok(_) => {
                        summary.downloaded += 1;
                        summary.resolved_sha256_by_attachment_id.insert(
                            attachment.id.clone(),
                            downloaded_item.sha256.to_ascii_lowercase(),
                        );
                    }
                    Err(DesktopNoteMediaError::ExistingBlobConflict(_)) => {
                        summary.conflicts += 1;
                    }
                    Err(error) => {
                        summary.record_failure(format!("无法保存下载附件：{error}"));
                    }
                }
            }
        }
    }

    summary
}

fn media_sync_cancelled(summary: &mut DesktopNoteMediaSyncSummary) -> bool {
    if crate::runtime::cancellation::check_current().is_err() {
        summary.record_failure("附件同步已取消");
        true
    } else {
        false
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MediaAppData {
    #[serde(default)]
    notes: Vec<MediaNote>,
    #[serde(default)]
    tombstones: Vec<MediaTombstone>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MediaNote {
    #[serde(default)]
    attachments: Vec<AttachmentReference>,
    #[serde(default)]
    revisions: Vec<HistoricalAttachments>,
    #[serde(default)]
    versions: Vec<HistoricalAttachments>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoricalAttachments {
    #[serde(default)]
    attachments: Vec<AttachmentReference>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AttachmentReference {
    #[serde(default)]
    id: String,
    #[serde(default)]
    mime_type: String,
    #[serde(default)]
    size_bytes: i64,
    #[serde(default)]
    sha256: String,
    #[serde(default)]
    created_at_epoch_millis: i64,
    #[serde(default)]
    updated_at_epoch_millis: i64,
}

impl AttachmentReference {
    fn revision(&self) -> i64 {
        if self.updated_at_epoch_millis > 0 {
            self.updated_at_epoch_millis
        } else {
            self.created_at_epoch_millis.max(0)
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MediaTombstone {
    #[serde(default)]
    entity_type: String,
    #[serde(default)]
    entity_id: String,
    #[serde(default)]
    deleted_at_epoch_millis: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MediaDeletion {
    attachment_id: String,
    deleted_at_epoch_millis: i64,
}

#[derive(Clone, Debug, Default)]
struct ReconcilePlan {
    attachments: Vec<AttachmentReference>,
    media_deletions: Vec<MediaDeletion>,
    media_deletion_by_attachment_id: BTreeMap<String, i64>,
    attachment_deletion_by_id: BTreeMap<String, i64>,
    explicitly_restored_attachment_ids: BTreeSet<String>,
}

fn build_reconcile_plan(
    app_data: &MediaAppData,
    server_by_id: &BTreeMap<String, MediaManifestItem>,
) -> ReconcilePlan {
    let current_attachments = app_data
        .notes
        .iter()
        .flat_map(|note| note.attachments.iter())
        .filter(|attachment| !attachment.id.trim().is_empty())
        .cloned()
        .collect::<Vec<_>>();

    let mut note_attachment_deletions = BTreeMap::<String, i64>::new();
    let mut media_deletions = BTreeMap::<String, i64>::new();
    for tombstone in &app_data.tombstones {
        if tombstone.entity_id.trim().is_empty() || tombstone.deleted_at_epoch_millis <= 0 {
            continue;
        }
        let target = match tombstone.entity_type.as_str() {
            NOTE_ATTACHMENT_TOMBSTONE => &mut note_attachment_deletions,
            NOTE_MEDIA_TOMBSTONE => &mut media_deletions,
            _ => continue,
        };
        target
            .entry(tombstone.entity_id.clone())
            .and_modify(|revision| *revision = (*revision).max(tombstone.deleted_at_epoch_millis))
            .or_insert(tombstone.deleted_at_epoch_millis);
    }

    let explicitly_restored_attachment_ids = current_attachments
        .iter()
        .filter(|attachment| {
            let attachment_deleted = note_attachment_deletions.get(&attachment.id).copied();
            let media_deleted = media_deletions.get(&attachment.id).copied();
            let server_deleted = server_by_id
                .get(&attachment.id)
                .map(|item| item.deleted_at_epoch_millis)
                .filter(|revision| *revision > 0);
            if attachment_deleted.is_none() && media_deleted.is_none() && server_deleted.is_none() {
                return false;
            }
            let deletion_floor = attachment_deleted
                .unwrap_or(-1)
                .max(media_deleted.unwrap_or(-1))
                .max(server_deleted.unwrap_or(-1));
            attachment.revision() > deletion_floor
        })
        .map(|attachment| attachment.id.clone())
        .collect::<BTreeSet<_>>();

    let mut attachment_by_id = BTreeMap::<String, AttachmentReference>::new();
    let all_attachments = current_attachments
        .into_iter()
        .chain(app_data.notes.iter().flat_map(|note| {
            note.revisions
                .iter()
                .chain(note.versions.iter())
                .flat_map(|snapshot| snapshot.attachments.iter().cloned())
        }));
    for attachment in all_attachments {
        if attachment.id.trim().is_empty() {
            continue;
        }
        match attachment_by_id.get(&attachment.id) {
            Some(existing) if existing.revision() >= attachment.revision() => {}
            _ => {
                attachment_by_id.insert(attachment.id.clone(), attachment);
            }
        }
    }

    let attachment_revision_by_id = attachment_by_id
        .iter()
        .map(|(id, attachment)| (id.clone(), attachment.revision()))
        .collect::<BTreeMap<_, _>>();
    let media_deletion_list = media_deletions
        .iter()
        .filter(|(id, deleted_at)| {
            let attachment_revision = attachment_revision_by_id
                .get(*id)
                .copied()
                .unwrap_or(i64::MIN);
            **deleted_at >= attachment_revision
        })
        .map(|(attachment_id, deleted_at_epoch_millis)| MediaDeletion {
            attachment_id: attachment_id.clone(),
            deleted_at_epoch_millis: *deleted_at_epoch_millis,
        })
        .collect::<Vec<_>>();

    ReconcilePlan {
        attachments: attachment_by_id.into_values().collect(),
        media_deletions: media_deletion_list,
        media_deletion_by_attachment_id: media_deletions,
        attachment_deletion_by_id: note_attachment_deletions,
        explicitly_restored_attachment_ids,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum AttachmentAction {
    Skip,
    MissingEverywhere,
    RemoteDeleted(i64),
    Upload { restore_deleted: bool },
    Download,
    Resolved(String),
    Conflict,
}

fn decide_attachment_action(
    direction: DesktopNoteMediaSyncDirection,
    attachment: &AttachmentReference,
    local: Option<&NoteMediaManifestEntry>,
    server: Option<&MediaManifestItem>,
    local_deleted_at: Option<i64>,
    explicitly_restored: bool,
) -> AttachmentAction {
    let attachment_revision = attachment.revision();
    if local_deleted_at.is_some_and(|deleted_at| deleted_at >= attachment_revision) {
        return AttachmentAction::Skip;
    }
    if let Some(server) = server {
        if server.deleted_at_epoch_millis > 0 {
            match direction {
                DesktopNoteMediaSyncDirection::DownloadOnly => {
                    return AttachmentAction::RemoteDeleted(server.deleted_at_epoch_millis);
                }
                DesktopNoteMediaSyncDirection::UploadOnly
                    if !explicitly_restored
                        || server.deleted_at_epoch_millis >= attachment_revision =>
                {
                    return AttachmentAction::Conflict;
                }
                // The Windows entry point always runs UploadOnly before it
                // publishes AppData. A current reference newer than every
                // deletion floor must reach the authenticated restore upload
                // here, or the subsequent bidirectional pass is unreachable.
                DesktopNoteMediaSyncDirection::UploadOnly => {}
                DesktopNoteMediaSyncDirection::Bidirectional
                    if server.deleted_at_epoch_millis >= attachment_revision =>
                {
                    return AttachmentAction::RemoteDeleted(server.deleted_at_epoch_millis);
                }
                DesktopNoteMediaSyncDirection::Bidirectional => {}
            }
        }
    }
    let active_server = server.filter(|item| item.deleted_at_epoch_millis <= 0);

    match (local, active_server) {
        (Some(local), server) => {
            let expected_hash = attachment.sha256.trim();
            if !expected_hash.is_empty() && !expected_hash.eq_ignore_ascii_case(&local.sha256) {
                return AttachmentAction::Conflict;
            }
            match server {
                None => match direction {
                    DesktopNoteMediaSyncDirection::DownloadOnly => AttachmentAction::Skip,
                    DesktopNoteMediaSyncDirection::Bidirectional
                    | DesktopNoteMediaSyncDirection::UploadOnly => AttachmentAction::Upload {
                        restore_deleted: explicitly_restored,
                    },
                },
                Some(server) if !server.sha256.eq_ignore_ascii_case(&local.sha256) => {
                    AttachmentAction::Conflict
                }
                Some(_) => AttachmentAction::Resolved(local.sha256.to_ascii_lowercase()),
            }
        }
        (None, Some(server)) => {
            let expected_hash = attachment.sha256.trim();
            let expected_mime = attachment.mime_type.trim();
            if (!expected_hash.is_empty() && !expected_hash.eq_ignore_ascii_case(&server.sha256))
                || (attachment.size_bytes > 0 && attachment.size_bytes != server.size_bytes)
                || (!expected_mime.is_empty() && expected_mime != server.mime_type.trim())
            {
                AttachmentAction::Conflict
            } else {
                match direction {
                    DesktopNoteMediaSyncDirection::Bidirectional
                    | DesktopNoteMediaSyncDirection::DownloadOnly => AttachmentAction::Download,
                    // The authenticated account already has these bytes. A missing
                    // local cache is not an upload conflict; the post-merge media
                    // pass downloads it. No blob or tombstone is changed here.
                    DesktopNoteMediaSyncDirection::UploadOnly => {
                        AttachmentAction::Resolved(server.sha256.to_ascii_lowercase())
                    }
                }
            }
        }
        (None, None) => AttachmentAction::MissingEverywhere,
    }
}

fn absorb_terminal_result(
    result: &SyncClientResult,
    summary: &mut DesktopNoteMediaSyncSummary,
) -> bool {
    if result.mode == "server_generation_rollback" {
        summary.verified_private_media.clear();
        summary.resolved_sha256_by_attachment_id.clear();
        summary.remote_deleted_at_by_attachment_id.clear();
        summary.server_generation_rollback = true;
        summary.current_generation = result.current_generation.max(0);
        return true;
    }
    if result.restore_required {
        summary.verified_private_media.clear();
        summary.resolved_sha256_by_attachment_id.clear();
        summary.remote_deleted_at_by_attachment_id.clear();
        summary.restore_required = true;
        summary.current_generation = result.current_generation.max(0);
        return true;
    }
    if is_quota_result(result) {
        summary.verified_private_media.clear();
        summary.resolved_sha256_by_attachment_id.clear();
        summary.remote_deleted_at_by_attachment_id.clear();
        summary.quota_rejected = true;
        return true;
    }
    false
}

fn is_quota_result(result: &SyncClientResult) -> bool {
    result.mode.contains("quota") || result.message.to_ascii_lowercase().contains("quota")
}

fn is_conflict_result(result: &SyncClientResult) -> bool {
    matches!(
        result.mode.as_str(),
        "media_conflict"
            | "media_deleted"
            | "media_upload_newer"
            | "media_referenced"
            | "media_references_unresolved"
    ) || {
        let message = result.message.to_ascii_lowercase();
        message.contains("different content")
            || message.contains("newer media")
            || message.contains("newer revision")
    }
}

fn operation_message(result: &SyncClientResult, fallback: &str) -> String {
    if result.message.trim().is_empty() {
        fallback.to_string()
    } else {
        format!("{fallback}：{}", result.message.trim())
    }
}

fn record_store_read_error(
    summary: &mut DesktopNoteMediaSyncSummary,
    error: DesktopNoteMediaError,
) {
    match error {
        DesktopNoteMediaError::Sha256Mismatch { .. }
        | DesktopNoteMediaError::SizeMismatch { .. }
        | DesktopNoteMediaError::ExistingBlobConflict(_) => summary.conflicts += 1,
        other => summary.record_failure(format!("无法读取本机附件：{other}")),
    }
}

fn record_missing_attachment(summary: &mut DesktopNoteMediaSyncSummary) {
    summary.record_failure("数据引用的附件在本机和服务器都不存在，已阻止发布悬空引用");
}

fn legacy_gap_matches(
    reference: &AttachmentReference,
    attested: Option<&LegacyMediaReference>,
) -> bool {
    // The manifest has already passed token/account/server identity validation.
    // Missing new references and changed content metadata still block preflight.
    reference.sha256.is_empty()
        && attested.is_some_and(|item| {
            item.attachment_id == reference.id
                && item.mime_type == reference.mime_type
                && item.size_bytes == reference.size_bytes
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    include!("desktop_note_media_restore_tests.rs");

    fn hash(character: char) -> String {
        std::iter::repeat(character).take(64).collect()
    }

    fn attachment(id: &str, sha256: &str, created: i64, updated: i64) -> AttachmentReference {
        AttachmentReference {
            id: id.to_string(),
            mime_type: "image/png".to_string(),
            size_bytes: 12,
            sha256: sha256.to_string(),
            created_at_epoch_millis: created,
            updated_at_epoch_millis: updated,
        }
    }

    fn local(id: &str, sha256: &str, revision: i64) -> NoteMediaManifestEntry {
        NoteMediaManifestEntry {
            attachment_id: id.to_string(),
            sha256: sha256.to_string(),
            mime_type: "image/png".to_string(),
            size_bytes: 12,
            updated_at_epoch_millis: revision,
        }
    }

    fn server(id: &str, sha256: &str, updated: i64, deleted: i64) -> MediaManifestItem {
        MediaManifestItem {
            attachment_id: id.to_string(),
            sha256: sha256.to_string(),
            mime_type: "image/png".to_string(),
            size_bytes: 12,
            updated_at_epoch_millis: updated,
            deleted_at_epoch_millis: deleted,
        }
    }

    #[test]
    fn current_revision_and_named_version_references_use_latest_attachment_revision() {
        let a = hash('a');
        let b = hash('b');
        let c = hash('c');
        let raw = format!(
            r#"{{"notes":[{{"attachments":[{{"id":"shared","sha256":"{a}","updatedAtEpochMillis":10}}],"revisions":[{{"attachments":[{{"id":"revision-only","sha256":"{b}","updatedAtEpochMillis":20}},{{"id":"shared","sha256":"{b}","updatedAtEpochMillis":30}}]}}],"versions":[{{"attachments":[{{"id":"version-only","sha256":"{c}","updatedAtEpochMillis":40}}]}}]}}]}}"#
        );
        let app_data: MediaAppData = serde_json::from_str(&raw).unwrap();
        let plan = build_reconcile_plan(&app_data, &BTreeMap::new());
        assert_eq!(3, plan.attachments.len());
        let shared = plan
            .attachments
            .iter()
            .find(|item| item.id == "shared")
            .unwrap();
        assert_eq!(30, shared.revision());
        assert_eq!(b, shared.sha256);
        assert!(plan
            .attachments
            .iter()
            .any(|item| item.id == "revision-only"));
        assert!(plan
            .attachments
            .iter()
            .any(|item| item.id == "version-only"));
    }

    #[test]
    fn newest_media_tombstone_is_planned_before_attachments_and_suppresses_stale_reference() {
        let app_data = MediaAppData {
            notes: vec![MediaNote {
                attachments: vec![attachment("gone", &hash('a'), 0, 50)],
                ..MediaNote::default()
            }],
            tombstones: vec![
                MediaTombstone {
                    entity_type: NOTE_MEDIA_TOMBSTONE.to_string(),
                    entity_id: "gone".to_string(),
                    deleted_at_epoch_millis: 60,
                },
                MediaTombstone {
                    entity_type: NOTE_MEDIA_TOMBSTONE.to_string(),
                    entity_id: "gone".to_string(),
                    deleted_at_epoch_millis: 70,
                },
            ],
        };
        let plan = build_reconcile_plan(&app_data, &BTreeMap::new());
        assert_eq!(
            vec![MediaDeletion {
                attachment_id: "gone".to_string(),
                deleted_at_epoch_millis: 70,
            }],
            plan.media_deletions
        );
        assert_eq!(
            AttachmentAction::Skip,
            decide_attachment_action(
                DesktopNoteMediaSyncDirection::Bidirectional,
                &plan.attachments[0],
                Some(&local("gone", &hash('a'), 50)),
                None,
                Some(70),
                false,
            )
        );
    }

    #[test]
    fn only_a_current_attachment_newer_than_every_delete_floor_is_an_explicit_restore() {
        let sha = hash('a');
        let app_data = MediaAppData {
            notes: vec![MediaNote {
                attachments: vec![attachment("restored", &sha, 0, 101)],
                revisions: vec![HistoricalAttachments {
                    attachments: vec![attachment("history", &sha, 0, 101)],
                }],
                ..MediaNote::default()
            }],
            tombstones: vec![
                MediaTombstone {
                    entity_type: NOTE_ATTACHMENT_TOMBSTONE.to_string(),
                    entity_id: "restored".to_string(),
                    deleted_at_epoch_millis: 90,
                },
                MediaTombstone {
                    entity_type: NOTE_MEDIA_TOMBSTONE.to_string(),
                    entity_id: "restored".to_string(),
                    deleted_at_epoch_millis: 100,
                },
                MediaTombstone {
                    entity_type: NOTE_MEDIA_TOMBSTONE.to_string(),
                    entity_id: "history".to_string(),
                    deleted_at_epoch_millis: 100,
                },
            ],
        };
        let server = BTreeMap::from([("restored".to_string(), server("restored", "", 0, 99))]);
        let plan = build_reconcile_plan(&app_data, &server);
        assert!(plan.explicitly_restored_attachment_ids.contains("restored"));
        assert!(!plan.explicitly_restored_attachment_ids.contains("history"));
        let restored = plan
            .attachments
            .iter()
            .find(|item| item.id == "restored")
            .unwrap();
        assert_eq!(
            AttachmentAction::Upload {
                restore_deleted: true
            },
            decide_attachment_action(
                DesktopNoteMediaSyncDirection::Bidirectional,
                restored,
                Some(&local("restored", &sha, 101)),
                server.get("restored"),
                Some(100),
                true,
            )
        );
    }

    #[test]
    fn hash_conflict_never_selects_local_or_server_copy() {
        let expected = hash('a');
        let local_hash = hash('b');
        let server_hash = hash('c');
        let reference = attachment("conflict", &expected, 0, 100);
        assert_eq!(
            AttachmentAction::Conflict,
            decide_attachment_action(
                DesktopNoteMediaSyncDirection::Bidirectional,
                &reference,
                Some(&local("conflict", &local_hash, 100)),
                Some(&server("conflict", &server_hash, 100, 0)),
                None,
                false,
            )
        );
    }

    #[test]
    fn attachment_action_respects_all_sync_directions() {
        let sha = hash('a');
        let reference = attachment("direction", &sha, 0, 100);
        let local = local("direction", &sha, 100);
        let active_server = server("direction", &sha, 100, 0);
        let different_server = server("direction", &hash('b'), 100, 0);
        let deleted_server = server("direction", "", 0, 110);

        assert_eq!(
            AttachmentAction::Upload {
                restore_deleted: false
            },
            decide_attachment_action(
                DesktopNoteMediaSyncDirection::Bidirectional,
                &reference,
                Some(&local),
                None,
                None,
                false,
            )
        );
        assert_eq!(
            AttachmentAction::Download,
            decide_attachment_action(
                DesktopNoteMediaSyncDirection::Bidirectional,
                &reference,
                None,
                Some(&active_server),
                None,
                false,
            )
        );

        assert_eq!(
            AttachmentAction::Skip,
            decide_attachment_action(
                DesktopNoteMediaSyncDirection::DownloadOnly,
                &reference,
                Some(&local),
                None,
                None,
                false,
            )
        );
        assert_eq!(
            AttachmentAction::Download,
            decide_attachment_action(
                DesktopNoteMediaSyncDirection::DownloadOnly,
                &reference,
                None,
                Some(&active_server),
                None,
                false,
            )
        );
        assert_eq!(
            AttachmentAction::RemoteDeleted(110),
            decide_attachment_action(
                DesktopNoteMediaSyncDirection::DownloadOnly,
                &reference,
                Some(&local),
                Some(&deleted_server),
                None,
                false,
            )
        );

        assert_eq!(
            AttachmentAction::Upload {
                restore_deleted: false
            },
            decide_attachment_action(
                DesktopNoteMediaSyncDirection::UploadOnly,
                &reference,
                Some(&local),
                None,
                None,
                false,
            )
        );
        assert_eq!(
            AttachmentAction::Conflict,
            decide_attachment_action(
                DesktopNoteMediaSyncDirection::UploadOnly,
                &reference,
                Some(&local),
                Some(&different_server),
                None,
                false,
            )
        );
        assert_eq!(
            AttachmentAction::Resolved(sha.clone()),
            decide_attachment_action(
                DesktopNoteMediaSyncDirection::UploadOnly,
                &reference,
                None,
                Some(&active_server),
                None,
                false,
            )
        );
        assert_eq!(
            AttachmentAction::Conflict,
            decide_attachment_action(
                DesktopNoteMediaSyncDirection::UploadOnly,
                &reference,
                Some(&local),
                Some(&deleted_server),
                None,
                false,
            )
        );

        assert!(!DesktopNoteMediaSyncDirection::DownloadOnly.sends_local_mutations());
        assert!(DesktopNoteMediaSyncDirection::DownloadOnly.applies_remote_deletions());
        assert!(DesktopNoteMediaSyncDirection::DownloadOnly.receives_remote_content());
        assert!(DesktopNoteMediaSyncDirection::UploadOnly.sends_local_mutations());
        assert!(!DesktopNoteMediaSyncDirection::UploadOnly.applies_remote_deletions());
        assert!(!DesktopNoteMediaSyncDirection::UploadOnly.receives_remote_content());
    }

    #[test]
    fn remote_only_media_must_match_reference_metadata() {
        let sha = hash('a');
        let reference = attachment("remote-only", &sha, 0, 100);
        let matching = server("remote-only", &sha, 100, 0);
        let mut wrong_size = matching.clone();
        wrong_size.size_bytes += 1;
        let mut wrong_mime = matching.clone();
        wrong_mime.mime_type = "image/jpeg".to_string();
        let mut wrong_hash = matching.clone();
        wrong_hash.sha256 = hash('b');
        let mut deleted = matching.clone();
        deleted.deleted_at_epoch_millis = 110;
        for conflicting in [wrong_size, wrong_mime, wrong_hash, deleted] {
            assert_eq!(
                AttachmentAction::Conflict,
                decide_attachment_action(
                    DesktopNoteMediaSyncDirection::UploadOnly,
                    &reference,
                    None,
                    Some(&conflicting),
                    None,
                    false,
                )
            );
        }
    }

    #[test]
    fn missing_everywhere_is_a_counted_failure_instead_of_a_successful_noop() {
        let reference = attachment("missing", &hash('a'), 0, 100);
        assert_eq!(
            AttachmentAction::MissingEverywhere,
            decide_attachment_action(
                DesktopNoteMediaSyncDirection::UploadOnly,
                &reference,
                None,
                None,
                None,
                false,
            )
        );

        let mut summary = DesktopNoteMediaSyncSummary::default();
        record_missing_attachment(&mut summary);
        assert_eq!(1, summary.failures);
        assert_eq!(1, summary.failure_messages.len());
        assert!(summary.failure_messages[0].contains("已阻止发布悬空引用"));
    }

    #[test]
    fn legacy_gap_requires_matching_authenticated_account_metadata() {
        let mut reference = attachment("legacy-gap", "", 0, 100);
        let attested = LegacyMediaReference {
            attachment_id: reference.id.clone(),
            mime_type: reference.mime_type.clone(),
            size_bytes: reference.size_bytes,
        };
        assert!(legacy_gap_matches(&reference, Some(&attested)));
        assert!(!legacy_gap_matches(&reference, None));
        reference.id = "new-reference".to_string();
        assert!(!legacy_gap_matches(&reference, Some(&attested)));
        reference.id = attested.attachment_id.clone();
        reference.size_bytes += 1;
        assert!(!legacy_gap_matches(&reference, Some(&attested)));
        reference.size_bytes = attested.size_bytes;
        reference.mime_type = "image/jpeg".to_string();
        assert!(!legacy_gap_matches(&reference, Some(&attested)));
        reference.mime_type = attested.mime_type.clone();
        reference.sha256 = hash('a');
        assert!(!legacy_gap_matches(&reference, Some(&attested)));
        let summary = DesktopNoteMediaSyncSummary {
            legacy_missing: 6,
            ..Default::default()
        };
        assert_eq!(0, summary.failures);
        assert_eq!("；6 个历史附件待恢复", summary.message_suffix());
        let mixed = DesktopNoteMediaSyncSummary {
            failures: 1,
            legacy_missing: 6,
            ..Default::default()
        };
        assert!(mixed.message_suffix().contains("失败1"));
    }

    #[test]
    fn terminal_and_counter_summary_are_stable_and_do_not_hide_conflicts() {
        let mut summary = DesktopNoteMediaSyncSummary {
            uploaded: 2,
            downloaded: 1,
            conflicts: 3,
            failures: 4,
            ..DesktopNoteMediaSyncSummary::default()
        };
        assert_eq!("；附件 上传2 下载1 冲突3 失败4", summary.message_suffix());
        let rollback = SyncClientResult {
            mode: "server_generation_rollback".to_string(),
            current_generation: 7,
            ..SyncClientResult::default()
        };
        assert!(absorb_terminal_result(&rollback, &mut summary));
        assert!(summary.server_generation_rollback);
        assert_eq!(7, summary.current_generation);
        assert_eq!(3, summary.conflicts);
    }
}
