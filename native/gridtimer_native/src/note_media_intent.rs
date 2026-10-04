// v0.0.0.1 - Keep note-scoped attachment removals separate from final media deletion.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const MAX_DETACHMENTS: usize = 10_000;
const MAX_KEY_BYTES: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AttachmentIdentity {
    pub sha256: String,
    pub size_bytes: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NoteAttachmentDetachIntent {
    pub detached_at_epoch_millis: i64,
    pub observed_attachment_revision: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<AttachmentIdentity>,
}

pub(crate) type NoteAttachmentDetachments =
    BTreeMap<String, BTreeMap<String, NoteAttachmentDetachIntent>>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LegacyMediaNormalization {
    pub cleanup_candidates: BTreeMap<String, i64>,
    pub unresolved_ids: BTreeSet<String>,
}

fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_KEY_BYTES
        && key.trim() == key
        && !key.chars().any(char::is_control)
}

fn valid_identity(identity: &AttachmentIdentity) -> bool {
    identity.size_bytes >= 0
        && identity.sha256.len() == 64
        && identity
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn validate_detachments(value: &NoteAttachmentDetachments) -> Result<(), String> {
    let mut count = 0usize;
    for (note_id, attachments) in value {
        if !valid_key(note_id) || attachments.is_empty() {
            return Err("note attachment intent has an invalid note key".into());
        }
        count = count.saturating_add(attachments.len());
        if count > MAX_DETACHMENTS {
            return Err("note attachment intent capacity exceeded".into());
        }
        for (attachment_id, intent) in attachments {
            if !valid_key(attachment_id)
                || intent.observed_attachment_revision < 0
                || intent.detached_at_epoch_millis <= intent.observed_attachment_revision
                || intent
                    .identity
                    .as_ref()
                    .is_some_and(|identity| !valid_identity(identity))
            {
                return Err("note attachment intent has an invalid identity or revision".into());
            }
        }
    }
    Ok(())
}

pub(crate) fn merge_detachments(
    target: &mut NoteAttachmentDetachments,
    incoming: &NoteAttachmentDetachments,
) -> Result<(), String> {
    validate_detachments(target)?;
    validate_detachments(incoming)?;
    // Validate a candidate so failure never partially changes the caller's policy.
    let mut next = target.clone();
    for (note_id, attachments) in incoming {
        let entries = next.entry(note_id.clone()).or_default();
        for (attachment_id, intent) in attachments {
            match entries.get_mut(attachment_id) {
                Some(previous) => {
                    if let (Some(left), Some(right)) = (&previous.identity, &intent.identity) {
                        if left != right {
                            return Err(
                                "note attachment intent changes an immutable identity".into()
                            );
                        }
                    }
                    let identity = previous
                        .identity
                        .clone()
                        .or_else(|| intent.identity.clone());
                    if (
                        intent.detached_at_epoch_millis,
                        intent.observed_attachment_revision,
                    ) > (
                        previous.detached_at_epoch_millis,
                        previous.observed_attachment_revision,
                    ) {
                        *previous = intent.clone();
                    }
                    previous.identity = identity;
                }
                None => {
                    entries.insert(attachment_id.clone(), intent.clone());
                }
            }
        }
    }
    validate_detachments(&next)?;
    *target = next;
    Ok(())
}

fn records<'a>(value: &'a Value, field: &str) -> impl Iterator<Item = &'a Value> {
    value
        .get(field)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn revision(value: &Value) -> i64 {
    value
        .get("updatedAtEpochMillis")
        .and_then(Value::as_i64)
        .or_else(|| value.get("createdAtEpochMillis").and_then(Value::as_i64))
        .unwrap_or(0)
}

fn attachment_identity(value: &Value) -> Result<Option<AttachmentIdentity>, String> {
    let sha256 = value.get("sha256").and_then(Value::as_str).unwrap_or("");
    if sha256.is_empty() {
        return Ok(None);
    }
    let identity = AttachmentIdentity {
        sha256: sha256.to_ascii_lowercase(),
        size_bytes: value.get("sizeBytes").and_then(Value::as_i64).unwrap_or(0),
    };
    if !valid_identity(&identity) {
        return Err("note attachment pre-image has an invalid immutable identity".into());
    }
    Ok(Some(identity))
}

fn insert_observed(
    result: &mut NoteAttachmentDetachments,
    note_id: &str,
    attachment: &Value,
    detached_at: i64,
) -> Result<(), String> {
    let Some(id) = attachment.get("id").and_then(Value::as_str) else {
        return Ok(());
    };
    let observed = revision(attachment);
    if detached_at <= observed || observed < 0 {
        return Ok(());
    }
    let intent = NoteAttachmentDetachIntent {
        detached_at_epoch_millis: detached_at,
        observed_attachment_revision: observed,
        identity: attachment_identity(attachment)?,
    };
    if !valid_key(note_id) || !valid_key(id) {
        return Err("note attachment pre-image has an invalid key".into());
    }
    let entries = result.entry(note_id.to_owned()).or_default();
    if let Some(previous) = entries.get_mut(id) {
        if let (Some(left), Some(right)) = (&previous.identity, &intent.identity) {
            if left != right {
                return Err("note attachment pre-images disagree on immutable identity".into());
            }
        }
        let identity = previous
            .identity
            .clone()
            .or_else(|| intent.identity.clone());
        if (
            intent.detached_at_epoch_millis,
            intent.observed_attachment_revision,
        ) > (
            previous.detached_at_epoch_millis,
            previous.observed_attachment_revision,
        ) {
            *previous = intent;
        }
        previous.identity = identity;
    } else {
        entries.insert(id.to_owned(), intent);
    }
    Ok(())
}

/// Only an operation's retained pre-image can establish observed removal.
/// A plain difference between two arbitrary replicas is not deletion evidence.
pub(crate) fn infer_snapshot_detachments(
    snapshot: &Value,
) -> Result<NoteAttachmentDetachments, String> {
    if !snapshot.is_object() {
        return Err("note attachment intent snapshot is not an object".into());
    }
    let mut result = NoteAttachmentDetachments::new();
    for note in records(snapshot, "notes") {
        if note.get("encryption").is_some_and(|value| !value.is_null()) {
            continue;
        }
        let Some(note_id) = note.get("id").and_then(Value::as_str) else {
            continue;
        };
        let current_revision = revision(note);
        if current_revision <= 0 {
            continue;
        }
        let current_ids = records(note, "attachments")
            .filter_map(|value| value.get("id").and_then(Value::as_str))
            .collect::<BTreeSet<_>>();
        for previous in records(note, "revisions") {
            // Synthesized merge-loser snapshots retain the loser's old revision.
            // A mutation pre-image instead carries the newly committed revision.
            if revision(previous) != current_revision
                || !crate::app_data::is_note_mutation_preimage(note_id, previous, current_revision)
            {
                continue;
            }
            for attachment in records(previous, "attachments") {
                let Some(id) = attachment.get("id").and_then(Value::as_str) else {
                    continue;
                };
                if !current_ids.contains(id) {
                    insert_observed(&mut result, note_id, attachment, current_revision)?;
                }
            }
        }
    }
    validate_detachments(&result)?;
    Ok(result)
}

/// A verified pre-image also binds attachments owned by a permanently deleted
/// note. This does not grant account-wide deletion of those attachments.
pub(crate) fn infer_transition_detachments(
    before: &Value,
    after: &Value,
) -> Result<NoteAttachmentDetachments, String> {
    let mut result = infer_snapshot_detachments(after)?;
    let deletions = tombstones(after, "note");
    for note in records(before, "notes") {
        let Some(note_id) = note.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(deleted_at) = deletions.get(note_id).copied() else {
            continue;
        };
        if revision(note) >= deleted_at {
            continue;
        }
        for attachment in records(note, "attachments") {
            insert_observed(&mut result, note_id, attachment, deleted_at)?;
        }
        for historical in records(note, "revisions").chain(records(note, "versions")) {
            for attachment in records(historical, "attachments") {
                insert_observed(&mut result, note_id, attachment, deleted_at)?;
            }
        }
    }
    validate_detachments(&result)?;
    Ok(result)
}

/// Project current references only. Product versions, revisions and conflict
/// payloads stay available to an explicit restoration operation.
pub(crate) fn project_current_detachments(
    snapshot: &mut Value,
    detachments: &NoteAttachmentDetachments,
) -> Result<bool, String> {
    validate_detachments(detachments)?;
    let Some(notes) = snapshot.get_mut("notes").and_then(Value::as_array_mut) else {
        return Ok(false);
    };
    let mut changed = false;
    for note in notes {
        if note.get("encryption").is_some_and(|value| !value.is_null()) {
            continue;
        }
        let Some(intents) = note
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| detachments.get(id))
        else {
            continue;
        };
        let mut suppressed = BTreeSet::<String>::new();
        for (attachment_id, intent) in intents {
            let matches = records(note, "attachments")
                .filter(|attachment| {
                    attachment.get("id").and_then(Value::as_str) == Some(attachment_id.as_str())
                })
                .collect::<Vec<_>>();
            for attachment in &matches {
                let actual = attachment_identity(attachment)?;
                if let Some(expected) = &intent.identity {
                    if actual.as_ref().is_some_and(|actual| expected != actual) {
                        return Err(
                            "current attachment conflicts with its detached immutable identity"
                                .into(),
                        );
                    }
                    // A later timestamp cannot restore a known identity after
                    // removing its descriptor. Old incomplete copies can still
                    // be suppressed by the floor without granting restoration.
                    if revision(attachment) > intent.detached_at_epoch_millis
                        && (actual.is_none()
                            || attachment
                                .get("sizeBytes")
                                .and_then(Value::as_i64)
                                .is_none())
                    {
                        return Err(
                            "restored attachment lacks its detached immutable identity".into()
                        );
                    }
                }
            }
            if matches
                .iter()
                .map(|attachment| revision(attachment))
                .max()
                .unwrap_or(0)
                <= intent.detached_at_epoch_millis
            {
                suppressed.insert(attachment_id.clone());
            }
        }
        if !suppressed.is_empty() {
            changed |=
                crate::app_data::remove_current_note_attachment_references(note, &suppressed)?;
        }
    }
    Ok(changed)
}

fn tombstones(snapshot: &Value, kind: &str) -> BTreeMap<String, i64> {
    let mut result = BTreeMap::new();
    for tombstone in records(snapshot, "tombstones") {
        if tombstone.get("entityType").and_then(Value::as_str) != Some(kind) {
            continue;
        }
        let (Some(id), Some(deleted_at)) = (
            tombstone.get("entityId").and_then(Value::as_str),
            tombstone
                .get("deletedAtEpochMillis")
                .and_then(Value::as_i64),
        ) else {
            continue;
        };
        if valid_key(id) && deleted_at >= 0 {
            result
                .entry(id.to_owned())
                .and_modify(|floor: &mut i64| *floor = (*floor).max(deleted_at))
                .or_insert(deleted_at);
        }
    }
    result
}

fn value_references_attachment(value: &Value, attachment_id: &str) -> bool {
    records(value, "attachments")
        .any(|attachment| attachment.get("id").and_then(Value::as_str) == Some(attachment_id))
        || records(value, "attachmentIds").any(|id| id.as_str() == Some(attachment_id))
        || value
            .get("document")
            .and_then(|document| document.get("blocks"))
            .and_then(Value::as_array)
            .is_some_and(|blocks| {
                blocks.iter().any(|block| {
                    block.get("attachmentId").and_then(Value::as_str) == Some(attachment_id)
                })
            })
        || ["content", "richTextPlainText"]
            .into_iter()
            .filter_map(|field| value.get(field).and_then(Value::as_str))
            .any(|text| {
                text.contains(&format!("note-image://{attachment_id}"))
                    || text.contains(&format!("data-note-image=\"{attachment_id}\""))
                    || text.contains(&format!("data-note-image='{attachment_id}'"))
            })
}

fn historical_revision(value: &Value) -> Option<i64> {
    value
        .get("updatedAtEpochMillis")
        .and_then(Value::as_i64)
        .or_else(|| value.get("capturedAtEpochMillis").and_then(Value::as_i64))
        .or_else(|| value.get("createdAtEpochMillis").and_then(Value::as_i64))
}

fn note_may_retain_attachment(note: &Value, attachment_id: &str, deleted_at: i64) -> bool {
    if note.get("encryption").is_some_and(|value| !value.is_null()) {
        return true;
    }
    value_references_attachment(note, attachment_id)
        || records(note, "revisions")
            .chain(records(note, "versions"))
            .any(|historical| match historical_revision(historical) {
                // A missing or invalid history clock remains ambiguous. Only a
                // known snapshot from strictly before deletion is obsolete.
                Some(revision) if revision < deleted_at => false,
                _ => note_may_retain_attachment(historical, attachment_id, deleted_at),
            })
}

pub(crate) fn normalize_legacy_media_tombstones(
    trusted_before: &Value,
    incoming: &Value,
    merged: &mut Value,
    detachments: &NoteAttachmentDetachments,
    proven_global_deletions: &BTreeMap<String, i64>,
) -> Result<LegacyMediaNormalization, String> {
    validate_detachments(detachments)?;
    let mut result = LegacyMediaNormalization::default();
    let mut media_floors = BTreeMap::new();
    for source in [&*merged, trusted_before, incoming] {
        for kind in ["noteAttachment", "noteMedia"] {
            for (id, floor) in tombstones(source, kind) {
                media_floors
                    .entry(id)
                    .and_modify(|previous: &mut i64| *previous = (*previous).max(floor))
                    .or_insert(floor);
            }
        }
    }
    let note_deletions = tombstones(merged, "note");
    let old_note_deletions = tombstones(trusted_before, "note");
    let incoming_note_deletions = tombstones(incoming, "note");
    let mut protect_current = BTreeMap::<String, i64>::new();
    for (attachment_id, deleted_at) in media_floors {
        let global_floor = proven_global_deletions
            .get(&attachment_id)
            .copied()
            .unwrap_or(0);
        if global_floor >= deleted_at && global_floor > 0 {
            continue;
        }
        let scoped = detachments.values().any(|entries| {
            entries
                .get(&attachment_id)
                .is_some_and(|intent| intent.detached_at_epoch_millis >= deleted_at)
        });
        // An already known whole-note deletion can replay its co-issued cleanup
        // marker after the deleted note's pre-image has been removed. This only
        // retains a cleanup candidate; it never authorizes physical deletion.
        let known_whole_note_replay = incoming_note_deletions.iter().any(|(id, floor)| {
            *floor == deleted_at
                && old_note_deletions
                    .get(id)
                    .is_some_and(|known| known >= floor)
        });
        if !scoped && !known_whole_note_replay {
            let retained_or_unknown = records(merged, "notes").any(|note| {
                if note
                    .get("id")
                    .and_then(Value::as_str)
                    .and_then(|id| note_deletions.get(id))
                    .is_some_and(|floor| revision(note) <= *floor)
                {
                    return false;
                }
                note_may_retain_attachment(note, &attachment_id, deleted_at)
            }) || records(merged, "syncConflictHistory")
                .filter_map(|entry| entry.get("payload"))
                .any(|note| note_may_retain_attachment(note, &attachment_id, deleted_at));
            if retained_or_unknown {
                result.unresolved_ids.insert(attachment_id.clone());
            }
            continue;
        }
        result
            .cleanup_candidates
            .insert(attachment_id.clone(), deleted_at);
        protect_current.insert(attachment_id, global_floor);
    }
    if let Some(values) = merged.get_mut("tombstones").and_then(Value::as_array_mut) {
        values.retain_mut(|value| {
            if !matches!(
                value.get("entityType").and_then(Value::as_str),
                Some("noteAttachment" | "noteMedia")
            ) {
                return true;
            }
            let Some(floor) = value
                .get("entityId")
                .and_then(Value::as_str)
                .and_then(|id| protect_current.get(id))
                .copied()
            else {
                return true;
            };
            if floor == 0 {
                return false;
            }
            value["deletedAtEpochMillis"] = Value::from(floor);
            true
        });
    }
    Ok(result)
}
