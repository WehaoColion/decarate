// Prevent the document-level AI entry from crashing when the current page or
// saveable UI state cannot be represented safely.

pub const SCREEN: &str = "com/ofairyo/gridtimer/ui/NoteStudioSheet.kt";

fn replace_once(source: &mut String, before: &str, after: &str) -> Result<(), String> {
    if source.matches(before).count() != 1 {
        return Err(format!("whole-document AI guard anchor is not unique: {before}"));
    }
    *source = source.replacen(before, after, 1);
    Ok(())
}

pub fn render(path: &str, source: &str) -> Result<String, String> {
    if path != SCREEN {
        return Ok(source.to_owned());
    }
    let mut rendered = source.to_owned();

    // SnapshotStateList is transient dialog state. Persisting the container through
    // rememberSaveable is unsafe when a document-owned saveable state holder is
    // replaced by the AI dialog; keep only the list contents in memory for this run.
    replace_once(
        &mut rendered,
        "    val agentSelectedIds = rememberSaveable(workspaceKey, identity, selectedFolderId) { mutableStateListOf<String>() }",
        "    val agentSelectedIds = remember(workspaceKey, identity, selectedFolderId) { mutableStateListOf<String>() }",
    )?;

    // A document-level entry is source-grounded by definition.
    replace_once(
        &mut rendered,
        "    var queryMode by rememberSaveable(workspaceKey, identity, priorityNoteId) { mutableStateOf(KnowledgeAiMode.DIRECT) }",
        "    var queryMode by rememberSaveable(workspaceKey, identity, priorityNoteId) { mutableStateOf(if (priorityNoteId == null) KnowledgeAiMode.DIRECT else KnowledgeAiMode.KNOWLEDGE) }",
    )?;

    // Legacy or malformed page content must not terminate composition while opening
    // the dialog. If extraction fails, the existing empty-source UI explains that no
    // readable source is available instead of crashing the process.
    replace_once(
        &mut rendered,
        r####"        knowledgeAiSourcesForMode(queryMode) {
            buildKnowledgeSourceCandidates(appData, question, selectedFolderId, searchScope, priorityNoteId)
        }"####,
        r####"        knowledgeAiSourcesForMode(queryMode) {
            runCatching {
                buildKnowledgeSourceCandidates(appData, question, selectedFolderId, searchScope, priorityNoteId)
            }.getOrElse { emptyList() }
        }"####,
    )?;

    // Resolve the requested page against the latest AppData before showing AI. Only
    // its id enters dialog state; the body remains in AppData and existing source
    // construction continues to emit bounded excerpts.
    replace_once(
        &mut rendered,
        r####"                    onAskKnowledge = { note ->
                        knowledgePriorityNoteId = note.id
                        knowledgeDialogVisible = true
                    }"####,
        r####"                    onAskKnowledge = { note ->
                        val current = appData.notes.firstOrNull { candidate ->
                            candidate.id == note.id && !candidate.isDeleted() && !candidate.isEncryptionLocked()
                        }
                        if (current == null) {
                            Toast.makeText(context, "当前文档不可读取，请刷新或先解锁后再询问。", Toast.LENGTH_LONG).show()
                        } else {
                            knowledgePriorityNoteId = current.id
                            knowledgeDialogVisible = true
                        }
                    }"####,
    )?;

    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unrelated_files_are_unchanged() {
        assert_eq!(render("other.kt", "sentinel").unwrap(), "sentinel");
    }

    #[test]
    fn duplicate_anchors_fail_loudly() {
        let mut value = "x x".to_string();
        assert!(replace_once(&mut value, "x", "y").is_err());
    }
}
