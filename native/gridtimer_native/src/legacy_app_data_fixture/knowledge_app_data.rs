// v2.22.52 - Atomic structured-page writes and lossless compatibility barriers.

fn knowledge_document_valid(document: &NoteDocument) -> bool {
    if document
        .knowledge
        .as_ref()
        .is_some_and(|page| page.validate().is_err())
    {
        return false;
    }
    let ids = document
        .blocks
        .iter()
        .map(|b| b.id.as_str())
        .collect::<HashSet<_>>();
    for block in &document.blocks {
        let Some(meta) = &block.knowledge else {
            continue;
        };
        if meta.validate().is_err() || block.id.is_empty() {
            return false;
        }
        let mut seen = HashSet::from([block.id.as_str()]);
        let mut parent = meta.parent_id.as_deref();
        let mut child_column = meta.column;
        while let Some(id) = parent {
            if !ids.contains(id) || !seen.insert(id) {
                return false;
            }
            let Some(owner) = document.blocks.iter().find(|b| b.id == id) else {
                return false;
            };
            let Some(container) = &owner.knowledge else {
                return false;
            };
            if !matches!(
                container.kind,
                crate::knowledge::BlockKind::Toggle
                    | crate::knowledge::BlockKind::Columns
                    | crate::knowledge::BlockKind::BulletedList
                    | crate::knowledge::BlockKind::NumberedList
            ) {
                return false;
            }
            if container.kind == crate::knowledge::BlockKind::Columns
                && child_column >= container.columns
            {
                return false;
            }
            child_column = container.column;
            parent = container.parent_id.as_deref();
        }
    }
    ids.len() == document.blocks.len() || document.blocks.iter().all(|b| b.knowledge.is_none())
}

fn knowledge_note_valid(note: &NoteEntry) -> bool {
    knowledge_document_valid(&note.document)
        && note
            .revisions
            .iter()
            .all(|version| knowledge_document_valid(&version.document))
        && note
            .versions
            .iter()
            .all(|version| knowledge_document_valid(&version.document))
}

fn knowledge_save_allowed(data: &AppData, previous: Option<&NoteEntry>, note: &NoteEntry) -> bool {
    if note.encryption.is_some() {
        return true;
    }
    if !knowledge_note_valid(note) {
        return false;
    }
    if let Some(old) = previous.filter(|old| old.encryption.is_none()) {
        if old.document.knowledge.is_some() && note.document.knowledge.is_none() {
            return false;
        }
        if old.document.knowledge.as_ref().is_some_and(|p| p.locked) {
            let mut document = note.document.clone();
            if let Some(meta) = &mut document.knowledge {
                meta.locked = true;
            }
            if document != old.document
                || note.title != old.title
                || note.content != old.content
                || note.attachments != old.attachments
            {
                return false;
            }
        }
    }
    let Some(meta) = &note.document.knowledge else {
        return true;
    };
    if let Some(database) = meta
        .parent_id
        .as_deref()
        .and_then(|id| data.notes.iter().find(|p| p.id == id))
        .and_then(|p| p.document.knowledge.as_ref())
        .and_then(|p| p.database.as_ref())
    {
        if crate::knowledge::validate_record(database, &meta.properties).is_err() {
            return false;
        }
    }
    if let Some(database) = &meta.database {
        if data
            .notes
            .iter()
            .filter(|p| {
                p.id != note.id
                    && p.deleted_at_epoch_millis.is_none()
                    && p.encryption.is_none()
                    && p.document
                        .knowledge
                        .as_ref()
                        .and_then(|m| m.parent_id.as_deref())
                        == Some(&note.id)
            })
            .any(|p| {
                p.document.knowledge.as_ref().is_some_and(|m| {
                    crate::knowledge::validate_record(database, &m.properties).is_err()
                })
            })
        {
            return false;
        }
    }
    let old_parent = previous
        .and_then(|p| p.document.knowledge.as_ref())
        .and_then(|p| p.parent_id.as_ref());
    if old_parent != meta.parent_id.as_ref() {
        let mut seen = HashSet::from([note.id.as_str()]);
        let mut parent = meta.parent_id.as_deref();
        while let Some(id) = parent {
            if !seen.insert(id) {
                return false;
            }
            let Some(owner) = data.notes.iter().find(|p| p.id == id) else {
                return false;
            };
            if owner.deleted_at_epoch_millis.is_some() || owner.encryption.is_some() {
                return false;
            }
            if owner.document.knowledge.as_ref().is_some_and(|p| p.locked) {
                return false;
            }
            parent = owner
                .document
                .knowledge
                .as_ref()
                .and_then(|p| p.parent_id.as_deref());
        }
    }
    true
}

/// All pages are validated before any state is returned. The caller atomically
/// persists this result once; a bad final row cannot leave a partial import.
pub fn upsert_knowledge_pages_app_data_json(
    raw: &str,
    pages_json: &str,
    now: i64,
) -> Option<String> {
    let mut data = parse_app_data_for_mutation(raw, now)?;
    let incoming: Vec<NoteEntry> = serde_json::from_str(pages_json).ok()?;
    if incoming.is_empty() || incoming.len() > 10_000 {
        return None;
    }
    let ids = incoming
        .iter()
        .map(|p| p.id.as_str())
        .collect::<HashSet<_>>();
    if ids.len() != incoming.len() || ids.contains("") {
        return None;
    }
    // Resolve references against the final candidate, including forward references.
    let mut candidate = data.clone();
    candidate.notes.retain(|p| !ids.contains(p.id.as_str()));
    candidate.notes.extend(incoming.iter().cloned());
    let mut normalized = Vec::with_capacity(incoming.len());
    for note in incoming {
        let previous = data.notes.iter().find(|old| old.id == note.id);
        if !knowledge_save_allowed(&candidate, previous, &note) {
            return None;
        }
        normalized.push(normalize_note_for_save(&candidate, note, previous, now)?);
    }
    let saved_ids = normalized
        .iter()
        .map(|p| p.id.as_str())
        .collect::<HashSet<_>>();
    data.notes.retain(|p| !saved_ids.contains(p.id.as_str()));
    data.notes.extend(normalized);
    serde_json::to_string(&data.sanitized(now)?).ok()
}

pub fn capture_knowledge_version_app_data_json(
    raw: &str,
    note_id: &str,
    label: &str,
    now: i64,
) -> Option<String> {
    let label = label.trim();
    if label.is_empty() || label.chars().count() > 120 {
        return None;
    }
    let mut data = parse_app_data_for_mutation(raw, now)?;
    let note = data.notes.iter_mut().find(|p| {
        p.id == note_id && p.deleted_at_epoch_millis.is_none() && p.encryption.is_none()
    })?;
    let revision = next_mutation_revision(note_revision_epoch_millis(note), now)?;
    let mut snapshot = note.snapshot_for_history(now.max(0), revision);
    snapshot.label = label.into();
    note.revisions.insert(0, snapshot);
    note.updated_at_epoch_millis = revision;
    serde_json::to_string(&data.sanitized(now)?).ok()
}

fn knowledge_hierarchy_valid(notes: &[NoteEntry]) -> bool {
    let index = notes
        .iter()
        .filter(|p| p.encryption.is_none() && p.deleted_at_epoch_millis.is_none())
        .map(|p| (p.id.as_str(), p))
        .collect::<HashMap<_, _>>();
    let mut complete = HashSet::new();
    for start in index.keys() {
        let mut seen = HashSet::new();
        let mut current = Some(*start);
        while let Some(id) = current {
            if complete.contains(id) {
                break;
            }
            if !seen.insert(id) {
                return false;
            }
            current = index
                .get(id)
                .and_then(|p| p.document.knowledge.as_ref())
                .and_then(|m| m.parent_id.as_deref());
        }
        complete.extend(seen);
    }
    true
}
