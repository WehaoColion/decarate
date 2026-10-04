// v0.0.4 - Exchange and reconcile compact reference identities without reparsing ciphertext history.
// v0.0.3 - Reconcile retained conflict files without replacing current notes.
// v0.0.2 - Include authenticated content hashes in byte transfer planning.
// v0.0.1 - Exchange bounded private metadata before reconciling media tombstones.
use super::*;
use crate::private_media_protocol::{
    PrivateMediaQuery, PRIVATE_MEDIA_MAX_QUERIES, PRIVATE_MEDIA_MAX_REQUEST_BYTES,
};

fn batches(queries: Vec<PrivateMediaQuery>) -> Result<Vec<Vec<PrivateMediaQuery>>, String> {
    let mut batches = Vec::new();
    let mut current = Vec::new();
    // Account for field names, nonce and the maximum restore receipt.
    let mut bytes = 8192;
    for query in queries {
        let length = serde_json::to_vec(&query).map_err(|e| e.to_string())?.len() + 1;
        if length + 8192 > PRIVATE_MEDIA_MAX_REQUEST_BYTES {
            return Err("加密附件引用超过单次同步容量".into());
        }
        if current.len() == PRIVATE_MEDIA_MAX_QUERIES
            || bytes + length > PRIVATE_MEDIA_MAX_REQUEST_BYTES
        {
            batches.push(std::mem::take(&mut current));
            bytes = 8192;
        }
        bytes += length;
        current.push(query);
    }
    if !current.is_empty() {
        batches.push(current);
    }
    Ok(batches)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn exchange(
    policy: &crate::desktop_state_store::DesktopPrivacyPolicy,
    references: &crate::desktop_private_media_index::DesktopPrivateMediaReferences,
    manifest: &SyncClientResult,
    url: &str,
    token: &str,
    user: &str,
    server: &str,
    namespace: &str,
    generation: i64,
    receipt: &str,
    direction: DesktopNoteMediaSyncDirection,
    summary: &mut DesktopNoteMediaSyncSummary,
) -> Option<crate::desktop_state_store::DesktopPrivacyPolicy> {
    let mut effective = policy.clone();
    summary.recovery_sources_pending = summary
        .recovery_sources_pending
        .saturating_add(references.unverified_recovery_sources());
    let queries = match references.queries(policy, direction.sends_local_mutations()) {
        Ok(value) => value,
        Err(error) => {
            summary.record_failure(error.to_string());
            return None;
        }
    };
    if queries.is_empty() {
        return Some(effective);
    }
    if manifest.private_media_protocol_version != 1 {
        // Older servers still sync ordinary AppData. Unknown private references
        // retain bytes and remain visible as pending work.
        summary.private_media_pending += queries.len();
        return Some(effective);
    }
    let batches = match batches(queries) {
        Ok(value) => value,
        Err(error) => {
            summary.record_failure(error);
            return None;
        }
    };
    for batch in batches {
        if media_sync_cancelled(summary) {
            return None;
        }
        let outcome = crate::sync_core::desktop_private_media_exchange(
            url, user, token, server, namespace, generation, receipt, batch,
        );
        if absorb_terminal_result(&outcome.result, summary) {
            return None;
        }
        let Some(verified) = outcome.verified else {
            summary.record_failure(operation_message(&outcome.result, "加密附件引用同步失败"));
            return None;
        };
        summary.private_media_pending += verified
            .entries()
            .iter()
            .filter(|entry| entry.declaration.is_none())
            .count();
        for entry in verified.entries() {
            if !references.contains(&entry.note_id, &entry.envelope_sha256) {
                summary.record_failure("附件引用与当前核对记录不一致");
                return None;
            }
            let query = PrivateMediaQuery {
                note_id: entry.note_id.clone(),
                envelope_sha256: entry.envelope_sha256.clone(),
                declaration: entry.declaration.clone(),
            };
            let declaration = match query.parsed_declaration() {
                Ok(Some(value)) => value,
                Ok(None) => continue,
                Err(error) => {
                    summary.record_failure(error);
                    return None;
                }
            };
            effective = match effective.including_indexed_sealed_media(
                references,
                &entry.note_id,
                declaration,
            ) {
                Ok(value) => value,
                Err(error) => {
                    summary.record_failure(error.to_string());
                    return None;
                }
            };
        }
        if direction.receives_remote_content() {
            summary.verified_private_media.push(verified);
        }
    }
    Some(effective)
}

pub(super) fn include_content_plan(
    plan: &mut ReconcilePlan,
    references: &crate::desktop_private_media_index::DesktopPrivateMediaReferences,
    policy: &crate::desktop_state_store::DesktopPrivacyPolicy,
    server: &BTreeMap<String, MediaManifestItem>,
    direction: DesktopNoteMediaSyncDirection,
    summary: &mut DesktopNoteMediaSyncSummary,
) {
    let scope = references.scope(policy);
    if scope.unknown_sealed_notes() > 0 {
        summary.record_failure("请解锁加密笔记以核对附件，再重新同步");
    } else if !scope.is_complete() {
        summary.record_failure("附件引用尚未完整核对，未确认同步完成");
    }
    if direction.sends_local_mutations() {
        plan.media_deletions.retain(|deletion| {
            if !scope.is_complete() || scope.ids().contains(&deletion.attachment_id) {
                summary.deferred_reference_deletions += 1;
                false
            } else {
                true
            }
        });
    }
    // Match each identity once; repeated linear scans become costly for large notes.
    let mut attachments = std::mem::take(&mut plan.attachments)
        .into_iter()
        .map(|attachment| (attachment.id.clone(), attachment))
        .collect::<BTreeMap<_, _>>();
    let mut private_ids = scope.sealed_ids().iter().collect::<Vec<_>>();
    private_ids.sort();
    for id in private_ids {
        let Some(hash) = scope.sealed_content_hash(id) else {
            summary.record_failure("加密附件缺少一致的文件指纹，请解锁关联笔记后重试");
            attachments.remove(id);
            continue;
        };
        // A retained reference proves content identity, not a user's decision to
        // reverse a historical deletion. Only the existing explicit restore path
        // may cross a tombstone; otherwise retain both bytes and deletion intent.
        if !plan.explicitly_restored_attachment_ids.contains(id)
            && (plan.attachment_deletion_by_id.contains_key(id)
                || plan.media_deletion_by_attachment_id.contains_key(id)
                || server
                    .get(id)
                    .is_some_and(|item| item.deleted_at_epoch_millis > 0))
        {
            summary.conflicts += 1;
            attachments.remove(id);
            continue;
        }
        if let Some(existing) = attachments.get_mut(id) {
            if !existing.sha256.is_empty() && !existing.sha256.eq_ignore_ascii_case(hash) {
                summary.conflicts += 1;
                attachments.remove(id);
            } else {
                existing.sha256 = hash.to_owned();
            }
        } else {
            attachments.insert(
                id.clone(),
                AttachmentReference {
                    id: id.clone(),
                    sha256: hash.to_owned(),
                    updated_at_epoch_millis: scope.sealed_revision(id).max(1),
                    ..Default::default()
                },
            );
        }
    }
    plan.attachments = attachments.into_values().collect();
}
