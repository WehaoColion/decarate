// v2.22.39 - Reconcile deleted attachments without replacing unsaved text or composition.
// v2.22.38 - Bind edit receipts to field revisions and workspaces; repair report findings offline.

pub fn render(path: &str, base: &str) -> Result<String, String> {
    match path {
        "com/ofairyo/gridtimer/ui/GridTimerScreen.kt" => timer_screen(base),
        "com/ofairyo/gridtimer/data/TimerRepository.kt" => repository(base),
        "com/ofairyo/gridtimer/ui/TimerViewModel.kt" => view_model(base),
        "com/ofairyo/gridtimer/ui/NoteDocumentEditor.kt" => document_editor(base),
        "com/ofairyo/gridtimer/ui/SmartisanNoteUi.kt" => recovery_records(base),
        "com/ofairyo/gridtimer/diagnostics/DiagnosticsExporter.kt" => diagnostics(base),
        _ => Ok(base.to_owned()),
    }
}

fn diagnostics(base: &str) -> Result<String, String> {
    let mut source = base.to_owned();
    replace(&mut source, "                        append(\"rss_kb=${reason.rss}\")",
        "                        appendLine(\"rss_kb=${reason.rss}\")\n                        appendLine(\"pid=${reason.pid}\")\n                        appendLine(\"process=${reason.processName}\")\n                        if (reason.reason == ApplicationExitInfo.REASON_ANR) {\n                            append(collectAnrTrace { reason.traceInputStream })\n                        }")?;
    source.push_str(ANR_TRACE);
    Ok(source)
}

pub const ANR_TRACE: &str = r####"

// Called only from the existing diagnostic export worker. Native tombstones
// are protobufs and must not be mislabelled as ANR text.
internal fun collectAnrTrace(open: () -> java.io.InputStream?): String {
    return try {
        val input = open() ?: return "trace_status=not_available"
        input.use {
            val prefixLimit = 128 * 1024
            val mainLimit = 64 * 1024
            val scanLimit = 4 * 1024 * 1024
            val prefix = java.io.ByteArrayOutputStream(prefixLimit)
            val main = java.io.ByteArrayOutputStream()
            val line = java.io.ByteArrayOutputStream()
            var scanned = 0
            var scanTruncated = false
            var lineTruncated = false
            var mainTruncated = false
            var mainFound = false
            var insideMain = false
            var mainStart = 0
            var lineStart = 0

            fun finishLine() {
                val bytes = line.toByteArray()
                val text = String(bytes, Charsets.UTF_8)
                if (text.startsWith("\"main\" ") && !mainFound) {
                    mainFound = true
                    insideMain = true
                    mainStart = lineStart
                } else if (insideMain && (text.startsWith("\"") || text.startsWith("----- end"))) {
                    insideMain = false
                }
                if (insideMain) {
                    val retained = minOf(bytes.size, mainLimit - main.size())
                    main.write(bytes, 0, retained)
                    mainTruncated = mainTruncated || retained < bytes.size || lineTruncated
                }
                line.reset()
                lineTruncated = false
                lineStart = scanned
            }

            val buffer = ByteArray(8192)
            while (true) {
                var read = it.read(buffer, 0, minOf(buffer.size, scanLimit + 1 - scanned))
                if (read == 0) {
                    val single = it.read()
                    if (single < 0) break
                    buffer[0] = single.toByte()
                    read = 1
                }
                if (read < 0) break
                val remainingPrefix = prefixLimit - prefix.size()
                if (remainingPrefix > 0) prefix.write(buffer, 0, minOf(read, remainingPrefix))
                for (index in 0 until read) {
                    scanned += 1
                    if (scanned > scanLimit) {
                        scanTruncated = true
                        break
                    }
                    if (line.size() < 16 * 1024) line.write(buffer[index].toInt()) else lineTruncated = true
                    if (buffer[index] == 10.toByte()) finishLine()
                }
                if (scanTruncated) break
            }
            if (line.size() > 0 || lineTruncated) finishLine()
            val extraMain = mainFound && (mainStart + main.size() > prefix.size() || mainTruncated)
            val captured = prefix.size() + if (extraMain) main.size() else 0
            buildString {
                appendLine("trace_status=captured")
                appendLine("trace_bytes=$captured")
                appendLine("trace_prefix_bytes=${prefix.size()}")
                appendLine("trace_truncated=${scanned > prefixLimit}")
                appendLine("trace_scanned_bytes=${minOf(scanned, scanLimit)}")
                appendLine("trace_scan_truncated=$scanTruncated")
                appendLine("trace_main_found=$mainFound")
                appendLine("trace_main_truncated=$mainTruncated")
                append(String(prefix.toByteArray(), Charsets.UTF_8))
                if (extraMain) {
                    appendLine("\ntrace_main_section")
                    append(String(main.toByteArray(), Charsets.UTF_8))
                }
            }
        }
    } catch (error: Exception) {
        "trace_status=read_failed\ntrace_error=${error.javaClass.simpleName}"
    }
}
"####;

fn replace(source: &mut String, old: &str, new: &str) -> Result<(), String> {
    if source.matches(old).count() != 1 {
        return Err(format!("Android fix anchor must occur once: {old}"));
    }
    *source = source.replacen(old, new, 1);
    Ok(())
}

fn replace_section(
    source: &mut String,
    start: &str,
    end: &str,
    edit: impl FnOnce(&mut String) -> Result<(), String>,
) -> Result<(), String> {
    let from = source
        .find(start)
        .ok_or_else(|| format!("Missing {start}"))?;
    let to = source[from..]
        .find(end)
        .ok_or_else(|| format!("Missing {end}"))?
        + from;
    let old = source[from..to].to_owned();
    let mut new = old.clone();
    edit(&mut new)?;
    replace(source, &old, &new)
}

fn repository(base: &str) -> Result<String, String> {
    let mut source = base.to_owned();
    for (start, end) in [
        (
            "    suspend fun updateSlotTitle(",
            "    suspend fun updateSlotNote(",
        ),
        (
            "    suspend fun updateSlotNote(",
            "    suspend fun setSlotCategory(",
        ),
    ] {
        replace_section(&mut source, start, end, |section| {
            replace(
                section,
                ": String) {",
                ": String, expectedWorkspaceKey: String? = null): Boolean {",
            )?;
            replace(section, "        updateData { data ->", "        return updateData(timerOnly = true, expectedWorkspaceKey = expectedWorkspaceKey) { data ->")
        })?;
    }
    Ok(source)
}

fn timer_screen(base: &str) -> Result<String, String> {
    let mut source = base.to_owned();
    replace(&mut source,
        "                    val safeValue = maxLength?.let { limit ->\n                        when {\n                            updatedValue.text.length <= limit -> updatedValue\n                            value.text.length > limit -> updatedValue\n                            else -> value\n                        }\n                    } ?: updatedValue",
        "                    val safeValue = if (value.composition != null) updatedValue else\n                        maxLength?.let { boundedFieldEdit(value, updatedValue, it) } ?: updatedValue")?;
    if !source.contains("import androidx.compose.material.icons.rounded.Save\n") {
        replace(&mut source, "import androidx.compose.material.icons.Icons\n",
            "import androidx.compose.material.icons.Icons\nimport androidx.compose.material.icons.rounded.Save\n")?;
    }
    replace(&mut source,
        "    val appTopPadding = with(density) { WindowInsets.statusBars.getTop(density).toDp() } + 12.dp",
        "    val appTopPadding = 12.dp")?;
    if !source.contains("import androidx.compose.ui.draw.clipToBounds") {
        replace(
            &mut source,
            "import androidx.compose.ui.draw.clip\n",
            "import androidx.compose.ui.draw.clip\nimport androidx.compose.ui.draw.clipToBounds\n",
        )?;
    }
    replace(&mut source,
        "    Box(modifier = Modifier.fillMaxSize()) {\n        LazyVerticalGrid(",
        "    Box(modifier = Modifier.fillMaxSize()\n        .windowInsetsPadding(WindowInsets.statusBars.only(WindowInsetsSides.Top))\n        .clipToBounds()) {\n        LazyVerticalGrid(")?;
    replace_section(
        &mut source,
        "private fun FocusCommandDeck(",
        "\n}\n",
        |section| {
            replace(section, "label = \"打开详情\"", "label = \"详情\"")?;
            replace(
                section,
                "HomeFocusKind.RUNNING -> \"继续计时\"",
                "HomeFocusKind.RUNNING -> \"暂停\"",
            )?;
            replace(
                section,
                "HomeFocusKind.RESUME -> \"继续任务\"",
                "HomeFocusKind.RESUME -> \"继续\"",
            )?;
            replace(
                section,
                "HomeFocusKind.START_FRESH -> \"开始计时\"",
                "HomeFocusKind.START_FRESH -> \"开始\"",
            )?;
            replace(
                section,
                "modifier = Modifier.weight(1.1f)",
                "modifier = Modifier.weight(1f)",
            )?;
            replace(
                section,
                "modifier = Modifier.weight(0.9f)",
                "modifier = Modifier.weight(1f)",
            )
        },
    )?;
    replace(&mut source,
        "                            note = timerDetailSummaryNote(HISTORY_SUMMARY_TODAY),",
        "                            note = if (trimmedSearchQuery.isNotEmpty() || filterCategoryId != null || filterSlotId != null)\n                                \"当前筛选内今日完成的时长\" else \"所有格子今日完成的时长\",")?;
    replace(&mut source,
        "                            note = historyWeekSummaryNote(filteredArchivedTasks.size),",
        "                            note = if (trimmedSearchQuery.isNotEmpty() || filterCategoryId != null || filterSlotId != null)\n                                \"当前筛选内近7天完成的时长\" else \"所有格子近7天完成的时长\",")?;
    replace(&mut source, "                    TimerDetailScreen(\n                        slot = slot,",
        "                    TimerDetailScreen(\n                        workspaceKey = activeWorkspaceKey,\n                        slot = slot,")?;
    replace(
        &mut source,
        "private fun TimerDetailScreen(\n    slot: TimerSlot,",
        "private fun TimerDetailScreen(\n    workspaceKey: String,\n    slot: TimerSlot,",
    )?;
    replace(&mut source, "    var newCategoryName by remember(slot.id) { mutableStateOf(\"\") }",
        "    val titleDraft = remember(slot.id, workspaceKey) { TimerFieldDraft(slot.title, slot.titleUpdatedAtEpochMillis, 24) }\n    val noteDraft = remember(slot.id, workspaceKey) { TimerFieldDraft(slot.note, slot.noteUpdatedAtEpochMillis, 60) }\n    LaunchedEffect(slot.title, slot.titleUpdatedAtEpochMillis) { titleDraft.accept(TimerFieldReceipt(slot.title, slot.titleUpdatedAtEpochMillis)) }\n    LaunchedEffect(slot.note, slot.noteUpdatedAtEpochMillis) { noteDraft.accept(TimerFieldReceipt(slot.note, slot.noteUpdatedAtEpochMillis)) }\n    DisposableEffect(titleDraft, noteDraft) {\n        onDispose {\n            titleDraft.commit()?.let { request -> onTitleChange(request.text) { titleDraft.acknowledge(request, it) } }\n            noteDraft.commit()?.let { request -> onNoteChange(request.text) { noteDraft.acknowledge(request, it) } }\n        }\n    }\n    var newCategoryName by remember(slot.id, workspaceKey) { mutableStateOf(\"\") }")?;
    for (field, label) in [("title", "Title"), ("note", "Note")] {
        replace(
            &mut source,
            &format!("    on{label}Change: (String) -> Unit,"),
            &format!("    on{label}Change: (String, (TimerFieldReceipt?) -> Unit) -> Unit,"),
        )?;
        replace(&mut source,
            &format!("on{label}Change = {{ viewModel.updateSlot{label}(slot.id, it) }}"),
            &format!("on{label}Change = {{ value, applied -> viewModel.updateSlot{label}(slot.id, value, workspaceKey = activeWorkspaceKey, onApplied = applied) }}"))?;
        replace(&mut source,
            &format!("                                value = slot.{field},\n                                onValueChange = on{label}Change,"),
            &format!("                                value = {field}Draft.value,\n                                onValueChange = {{ updated ->\n                                    {field}Draft.edit(updated)?.let {{ submission ->\n                                        on{label}Change(submission.text) {{ applied -> {field}Draft.acknowledge(submission, applied) }}\n                                    }}\n                                }},\n                                onFocusChanged = {{ focused ->\n                                    if (!focused) {field}Draft.commit()?.let {{ submission ->\n                                        on{label}Change(submission.text) {{ applied -> {field}Draft.acknowledge(submission, applied) }}\n                                    }}\n                                }},"))?;
    }
    replace(
        &mut source,
        "@Composable\ninternal fun SmartisanTextField(\n    value: String,",
        &format!(
            "{TIMER_DRAFT}\n@Composable\ninternal fun SmartisanTextField(\n    value: String,"
        ),
    )?;
    replace(&mut source,
        "                                maxLines = 3,\n                            )\n                        }\n                    }\n                }\n\n                item {\n                    StaggeredReveal(index = 3,",
        "                                maxLines = 3,\n                            )\n                            if (titleDraft.saveFailed || noteDraft.saveFailed) {\n                                Text(\"修改未保存，请重试\", color = MaterialTheme.colorScheme.error)\n                                PhysicalButton(\n                                    label = \"重试保存\", icon = Icons.Rounded.Save, accent = accent, filled = false,\n                                    onClick = {\n                                        titleDraft.retry()?.let { request -> onTitleChange(request.text) { titleDraft.acknowledge(request, it) } }\n                                        noteDraft.retry()?.let { request -> onNoteChange(request.text) { noteDraft.acknowledge(request, it) } }\n                                    }\n                                )\n                            }\n                        }\n                    }\n                }\n\n                item {\n                    StaggeredReveal(index = 3,")?;
    Ok(source)
}

pub const TIMER_DRAFT: &str = r####"internal data class TimerFieldSubmission(val generation: Long, val text: String)
internal data class TimerFieldReceipt(val text: String, val revision: Long)

internal fun boundedFieldEdit(base: TextFieldValue, next: TextFieldValue, limit: Int): TextFieldValue {
    require(limit >= 0)
    if (next.composition != null) return next
    // Legacy overlong text may be shortened without silently discarding its tail.
    val allowed = maxOf(limit, base.text.length)
    if (next.text.length <= allowed) return next
    fun splitsPair(text: String, at: Int): Boolean = at > 0 && at < text.length &&
        text[at - 1].isHighSurrogate() && text[at].isLowSurrogate()
    var prefix = base.text.commonPrefixWith(next.text).length
    if (splitsPair(base.text, prefix) || splitsPair(next.text, prefix)) prefix -= 1
    var suffix = 0
    while (suffix < base.text.length - prefix && suffix < next.text.length - prefix &&
        base.text[base.text.lastIndex - suffix] == next.text[next.text.lastIndex - suffix]) suffix += 1
    if (splitsPair(base.text, base.text.length - suffix) || splitsPair(next.text, next.text.length - suffix)) suffix -= 1
    val insertedEnd = next.text.length - suffix
    var acceptedEnd = prefix + minOf(insertedEnd - prefix, (allowed - prefix - suffix).coerceAtLeast(0))
    if (splitsPair(next.text, acceptedEnd)) acceptedEnd -= 1
    val text = next.text.substring(0, acceptedEnd) + next.text.substring(insertedEnd)
    fun position(at: Int): Int = when {
        at <= acceptedEnd -> at
        at >= insertedEnd -> at - (insertedEnd - acceptedEnd)
        else -> acceptedEnd
    }.coerceIn(0, text.length)
    return next.copy(text = text, selection = TextRange(position(next.selection.start), position(next.selection.end)))
}

internal class TimerFieldDraft(initial: String, initialRevision: Long = 0L, private val maxLength: Int? = null) {
    var value by mutableStateOf(TextFieldValue(initial))
        private set
    private var generation = 0L
    private var acceptedRevision = initialRevision
    private var pending: TimerFieldSubmission? = null
    private var compositionBase: TextFieldValue? = null
    var saveFailed by mutableStateOf(false)
        private set

    fun edit(updated: TextFieldValue): TimerFieldSubmission? {
        if (updated.composition != null) {
            if (compositionBase == null) compositionBase = value
            value = updated
            return null
        }
        val limited = maxLength?.let { boundedFieldEdit(compositionBase ?: value, updated, it) } ?: updated
        compositionBase = null
        val changed = limited.text != value.text || value.composition != limited.composition
        value = limited
        if (!changed) return null
        return submit()
    }

    fun commit(): TimerFieldSubmission? {
        if (value.composition == null) return null
        return edit(value.copy(composition = null))
    }

    private fun submit(): TimerFieldSubmission {
        generation += 1L
        saveFailed = false
        return TimerFieldSubmission(generation, value.text.trimStart()).also { pending = it }
    }

    fun retry(): TimerFieldSubmission? = if (saveFailed) submit() else null

    fun acknowledge(submission: TimerFieldSubmission, receipt: TimerFieldReceipt?) {
        if (pending?.generation != submission.generation) return
        if (receipt == null || receipt.text != submission.text || receipt.revision < acceptedRevision) {
            saveFailed = true
            return
        }
        pending = null
        saveFailed = false
        accept(receipt)
    }

    fun accept(receipt: TimerFieldReceipt) {
        if (value.composition != null || pending != null) return
        if (receipt.revision < acceptedRevision) return
        acceptedRevision = receipt.revision
        val stored = receipt.text
        if (stored != value.text) {
            val removed = (value.text.length - stored.length).coerceAtLeast(0)
            value = TextFieldValue(stored, TextRange(
                (value.selection.start - removed).coerceIn(0, stored.length),
                (value.selection.end - removed).coerceIn(0, stored.length)
            ))
        }
    }
}
"####;

fn view_model(base: &str) -> Result<String, String> {
    let mut source = base.to_owned();
    for (field, label) in [("title", "Title"), ("note", "Note")] {
        replace(&mut source,
            &format!("    fun updateSlot{label}(slotId: Int, {field}: String) {{\n        viewModelScope.launch {{\n            repository.updateSlot{label}(slotId, {field})\n        }}\n    }}"),
            &format!("    internal fun updateSlot{label}(slotId: Int, {field}: String, workspaceKey: String = currentWorkspaceKey(), onApplied: (TimerFieldReceipt?) -> Unit = {{}}) {{\n        viewModelScope.launch {{\n            val accepted = repository.updateSlot{label}(slotId, {field}, expectedWorkspaceKey = workspaceKey)\n            val slot = appData.value.slots.firstOrNull {{ it.id == slotId }}\n            onApplied(if (accepted && currentWorkspaceKey() == workspaceKey && slot != null)\n                TimerFieldReceipt(slot.{field}, slot.{field}UpdatedAtEpochMillis) else null)\n        }}\n    }}"))?;
    }
    Ok(source)
}

fn document_editor(base: &str) -> Result<String, String> {
    let mut source = recovery_records(base)?;
    replace(&mut source,
        "    val blankDraftDeleted = mutableStateOf(false)\n\n    init {",
        &format!("    val blankDraftDeleted = mutableStateOf(false)\n{ATTACHMENT_RECONCILIATION}\n    init {{"))?;
    replace(&mut source,
        "    val blankDraftDeletionEligibleAtEntry = editorSession.blankDraftDeletionEligibleAtEntry",
        "    val attachmentIds = remember(note.attachments) { note.attachments.map { it.id }.toSet() }\n    LaunchedEffect(editorSession, attachmentIds) {\n        editorSession.acceptAttachmentIds(attachmentIds)\n    }\n    val blankDraftDeletionEligibleAtEntry = editorSession.blankDraftDeletionEligibleAtEntry")?;
    replace(&mut source,
        "    val previewNote = remember(titleText, markdownEnabled, blocks, accentSeed, pinned, folderId) {",
        "    val previewNote = remember(titleText, markdownEnabled, blocks, accentSeed, pinned, folderId, note.attachments) {")?;
    replace_section(
        &mut source,
        "internal class DocumentEditorSessionViewModel : ViewModel() {",
        "private data class NoteOutlineItem(",
        |section| {
            *section = DOCUMENT_SESSIONS.to_owned();
            Ok(())
        },
    )?;
    replace(&mut source, "    val editorWorkspaceKey = remember(note.id) { workspaceKey }",
        "    val editorWorkspaceKey = remember(note.id, workspaceKey) { workspaceKey }\n    val editorEntryId = rememberSaveable(note.id, workspaceKey) { java.util.UUID.randomUUID().toString() }")?;
    replace(
        &mut source,
        "    val editorSession = remember(note.id) {",
        "    val editorSession = remember(note.id, workspaceKey, editorEntryId) {",
    )?;
    replace(&mut source, "            noteId = note.id,\n            initial = initialDraft,",
        "            noteId = note.id,\n            workspaceKey = editorWorkspaceKey,\n            entryId = editorEntryId,\n            initial = initialDraft,")?;
    replace(
        &mut source,
        "                editorSessionViewModel.clear(note.id)",
        "                editorSessionViewModel.clear(note.id, editorWorkspaceKey, editorSession)",
    )?;
    replace(&mut source, "        val completePersistence: (Boolean) -> Unit = { persisted ->\n            if (persisted) {\n                lastDurableExitDraft.value = candidate\n                blankDraftDeleted.value = candidate.isDocumentCanvasBlankDraft() && needsBlankDeletion\n            }\n            onComplete(persisted)\n        }",
        "        saveState = DocumentEditorSaveState.SAVING\n        saveRequestGeneration += 1L\n        val persistenceGeneration = saveRequestGeneration\n        val completePersistence: (Boolean) -> Unit = { persisted ->\n            if (persistenceGeneration == saveRequestGeneration) {\n                saveState = if (persisted) DocumentEditorSaveState.LOCAL_SAVED else DocumentEditorSaveState.FAILED\n                if (persisted) {\n                    lastDurableExitDraft.value = candidate\n                    blankDraftDeleted.value = candidate.isDocumentCanvasBlankDraft() && needsBlankDeletion\n                }\n            }\n            onComplete(persisted)\n        }")?;
    Ok(source)
}

const ATTACHMENT_RECONCILIATION: &str = r####"
    private var observedAttachmentIds: Set<String>? = null

    fun acceptAttachmentIds(current: Set<String>) {
        val previous = observedAttachmentIds
        observedAttachmentIds = current.toSet()
        if (previous == null) return
        val removed = previous - current
        if (removed.isEmpty()) return
        fun clean(source: List<NoteBlock>): List<NoteBlock> =
            source.filterNot { it.attachmentId in removed }
        val next = clean(blocks.value)
        if (next != blocks.value) {
            blocks.value = next.ifEmpty { listOf(NoteBlock(type = NoteBlockType.TEXT)) }
            // TextFieldValue objects retain selection and an active IME composition.
            // Pending imports have their own identifiers and are never removed here.
            saveRequestGeneration.value += 1L
            lastDurableExitDraft.value = null
        }
        for (stack in listOf(undoStack, redoStack)) {
            for (index in stack.indices) {
                val snapshot = stack[index]
                val retained = clean(snapshot.blocks)
                if (retained != snapshot.blocks) stack[index] = snapshot.copy(blocks = retained)
            }
        }
    }
"####;

pub const DOCUMENT_SESSIONS: &str = r####"internal class DocumentEditorSessionViewModel : ViewModel() {
    private data class Entry(val entryId: String, val session: DocumentEditorSession)
    private val sessions = mutableMapOf<Pair<String, String>, Entry>()

    fun session(
        noteId: String,
        workspaceKey: String,
        entryId: String,
        initial: NoteDraftState,
        blankDraftDeletionEligibleAtEntry: Boolean
    ): DocumentEditorSession {
        val key = workspaceKey to noteId
        val existing = sessions[key]
        val clean = existing?.session?.saveState?.value in setOf(
            DocumentEditorSaveState.SAVED, DocumentEditorSaveState.LOCAL_SAVED
        )
        // Configuration changes keep the same entry. A new visit uses the
        // repository's current metadata; an unsaved draft remains recoverable.
        val session = if (existing == null || (existing.entryId != entryId && clean)) {
            DocumentEditorSession(initial, blankDraftDeletionEligibleAtEntry)
        } else existing.session
        sessions[key] = Entry(entryId, session)
        return session
    }

    fun clear(noteId: String, workspaceKey: String, expected: DocumentEditorSession) {
        val key = workspaceKey to noteId
        if (sessions[key]?.session === expected) sessions.remove(key)
    }
}

"####;

fn recovery_records(base: &str) -> Result<String, String> {
    let mut source = base.to_owned();
    if !source.contains("import androidx.compose.foundation.lazy.itemsIndexed") {
        replace(&mut source, "import androidx.compose.foundation.lazy.items\n",
            "import androidx.compose.foundation.lazy.items\nimport androidx.compose.foundation.lazy.itemsIndexed\n")?;
    }
    replace_section(
        &mut source,
        "private fun NoteRevisionHistoryDialog(",
        "\n}\n",
        |section| {
            replace(section, "    Dialog(onDismissRequest = onDismiss) {",
            "    val captureFormatter = remember { java.time.format.DateTimeFormatter\n        .ofPattern(\"yyyy-MM-dd HH:mm:ss.SSS\").withZone(java.time.ZoneId.systemDefault()) }\n    Dialog(onDismissRequest = onDismiss) {")?;
            replace(section, "items(revisions, key = NoteRevisionSnapshot::id) { revision ->",
            "itemsIndexed(revisions, key = { _, revision -> revision.id }) { index, revision ->")?;
            replace(section, "text = safeFormat(dateTimeFormatter, revision.capturedAtEpochMillis, \"--\"),",
            "text = \"记录 ${revisions.size - index} · ${safeFormat(captureFormatter, revision.capturedAtEpochMillis, \"--\")}\",")?;
            *section = section
                .replace("恢复到这个版本", "恢复此记录")
                .replace("未命名版本", "未命名记录")
                .replace("该版本没有正文预览。", "此记录没有正文预览。");
            Ok(())
        },
    )?;
    Ok(source)
}

#[cfg(test)]
mod tests {
    #[test]
    fn final_source_pipeline_contains_all_phone_fixes() {
        for path in [
            "com/ofairyo/gridtimer/ui/GridTimerScreen.kt",
            "com/ofairyo/gridtimer/data/TimerRepository.kt",
            "com/ofairyo/gridtimer/ui/TimerViewModel.kt",
            "com/ofairyo/gridtimer/ui/NoteDocumentEditor.kt",
            "com/ofairyo/gridtimer/ui/SmartisanNoteUi.kt",
        ] {
            let base = crate::kotlin_sources::SOURCES
                .iter()
                .find(|s| s.path == path)
                .unwrap()
                .contents;
            let source = crate::android_performance_override::render(path, base).unwrap();
            let source = crate::android_responsiveness::render(path, &source).unwrap();
            let source = crate::diagnostics_export_backend::render(path, &source).unwrap();
            let source = crate::diagnostics_collection_backend::render(path, &source).unwrap();
            let source = crate::diagnostics_export_ui::render(path, &source).unwrap();
            let source = crate::android_snapshot_memory::render(path, &source).unwrap();
            let source = crate::android_backup_availability::render(path, &source).unwrap();
            let source = crate::android_timer_latency::render(path, &source).unwrap();
            let source = crate::android_note_latency::render(path, &source).unwrap();
            let rendered = super::render(path, &source).unwrap();
            if path.ends_with("NoteDocumentEditor.kt") {
                assert!(rendered.contains("private data class NoteOutlineItem("));
            }
        }
    }
}
