// v2.23.2.16 - Scope preset branches and reject missing, repeated or reinjected editor hooks.
//! FlowUs gap batch: safe mobile structured-page edits, local folds and outline navigation.
//! Keep the protected reader guard; never send advanced data through the flat editor.
pub const PATH: &str = "com/ofairyo/gridtimer/ui/StructuredKnowledgeReader.kt";
pub const POLICY_PATH: &str = "com/ofairyo/gridtimer/data/StructuredEditPolicy.kt";
pub const EDIT_PATH: &str = "com/ofairyo/gridtimer/data/StructuredNoteEditing.kt";
pub const TEST_PATH: &str = "com/ofairyo/gridtimer/data/StructuredEditPolicyTest.kt";
pub const EDIT_TEST_PATH: &str = "com/ofairyo/gridtimer/data/StructuredNoteEditingTest.kt";

fn replace_once(source: &mut String, before: &str, after: &str) -> Result<(), String> {
    if source.matches(before).count() != 1 {
        return Err(format!(
            "structured editing anchor missing or repeated: {}",
            before.lines().next().unwrap_or("")
        ));
    }
    *source = source.replacen(before, after, 1);
    Ok(())
}

pub fn render(path: &str, base: &str) -> Result<String, String> {
    let mut source = base.to_owned();
    if path.ends_with("/TimerRepository.kt") {
        if source.contains("    internal suspend fun applyStructuredNotePatch(") {
            return Err("structured repository patch hook is already present".into());
        }
        let anchor = "    suspend fun upsertNoteDurablyResult(\n";
        replace_once(&mut source, anchor, &format!("{REPOSITORY_WRITE}{anchor}"))?;
    } else if path.ends_with("/TimerViewModel.kt") {
        if source.contains("    internal fun applyStructuredNotePatch(") {
            return Err("structured view-model patch hook is already present".into());
        }
        let anchor = "    fun upsertNoteAndFlush(\n";
        replace_once(&mut source, anchor, &format!("{VIEW_MODEL_WRITE}{anchor}"))?;
    } else if path.ends_with("/NoteStudioSheet.kt") {
        replace_once(&mut source,
            "internal enum class NoteDraftPreset {\n    BLANK,\n    CHECKLIST,\n    MEETING,\n    REVIEW\n}",
            "internal enum class NoteDraftPreset {\n    BLANK,\n    CHECKLIST,\n    MEETING,\n    REVIEW,\n    STRUCTURED\n}")?;
        let preset_anchor = "internal fun buildPresetNote(\n    preset: NoteDraftPreset,\n    folderId: String? = null,\n    now: Long = System.currentTimeMillis()\n): NoteEntry {\n    val (title, body, accentSeed, markdownEnabled) = when (preset) {\n";
        replace_once(&mut source, preset_anchor, &format!("{preset_anchor}        NoteDraftPreset.STRUCTURED -> return com.ofairyo.gridtimer.data.buildStructuredMobileNote(folderId, now)\n"))?;
        let sticky_anchor = "internal fun buildStickyPresetNote(\n    preset: NoteDraftPreset,\n    folderId: String? = null,\n    now: Long = System.currentTimeMillis()\n): NoteEntry {\n    val (title, body, accentSeed, markdownEnabled) = when (preset) {\n";
        replace_once(&mut source, sticky_anchor, &format!("{sticky_anchor}        NoteDraftPreset.STRUCTURED -> throw IllegalArgumentException(\"结构页只能在知识模块创建\")\n"))?;
        let anchor = "private fun NotePresetRow(\n    onCreatePreset: (NoteDraftPreset) -> Unit\n) {\n    LazyRow(horizontalArrangement = Arrangement.spacedBy(12.dp)) {";
        replace_once(&mut source, anchor, &format!("{anchor}{PRESET_CARD}"))?;
    } else if path.ends_with("/NoteEditorUi.kt")
        || source.contains("internal fun NoteEditorContent(")
    {
        let needle = ") {\n    val context = LocalContext.current\n    val focusManager = LocalFocusManager.current\n    val titleFocusRequester = remember { FocusRequester() }";
        let replacement = ") {\n    if (note.hasStructuredKnowledge()) {\n        val structuredWorkspace = LocalNoteMediaWorkspaceKey.current\n        EncryptedNoteSecureWindowEffect(note.encryption != null)\n        StructuredKnowledgeReader(note = note, onBack = onBack, onWrite = { patch, done ->\n            viewModel.applyStructuredNotePatch(patch, structuredWorkspace, done)\n        }, onPreviewImage = onRequestPreviewAttachment)\n        return\n    }\n    val context = LocalContext.current\n    val focusManager = LocalFocusManager.current\n    val titleFocusRequester = remember { FocusRequester() }";
        replace_once(&mut source, needle, replacement)?;
    } else if source.contains("internal fun SmartisanNoteEditorContent(") {
        let needle = ") {\n    val context = androidx.compose.ui.platform.LocalContext.current\n    val workspaceKey = LocalNoteMediaWorkspaceKey.current";
        replace_once(&mut source, needle, ") {\n    if (note.hasStructuredKnowledge()) {\n        EncryptedNoteSecureWindowEffect(note.encryption != null)\n        StructuredKnowledgeReader(note, onBack) { id -> onRequestPreviewAttachment(null, id) }\n        return\n    }\n    val context = androidx.compose.ui.platform.LocalContext.current\n    val workspaceKey = LocalNoteMediaWorkspaceKey.current")?;
    }
    Ok(source)
}

const PRESET_CARD: &str = r####"
        item {
            NotePresetCard(
                title = "结构页",
                description = "在手机上编辑待办、折叠内容、表格与属性。",
                icon = Icons.Rounded.Title,
                accent = notesAccentColor,
                onClick = { onCreatePreset(NoteDraftPreset.STRUCTURED) }
            )
        }
"####;

const VIEW_MODEL_WRITE: &str = r####"    internal fun applyStructuredNotePatch(
        patch: com.ofairyo.gridtimer.data.StructuredNotePatch,
        expectedWorkspaceKey: String,
        onComplete: (Boolean, String) -> Unit
    ) {
        enqueueNoteSaveMutation(onComplete = { result ->
            val current = currentWorkspaceKey() == expectedWorkspaceKey
            onComplete(result.committed && current, if (!current) "工作区已改变，未向当前页面发布旧结果。"
                else if (result.committed) "" else result.detail.ifBlank { "保存未完成，原内容仍保留。" })
        }) {
            repository.applyStructuredNotePatch(patch, expectedWorkspaceKey)
        }
    }

"####;

const REPOSITORY_WRITE: &str = r####"    internal suspend fun applyStructuredNotePatch(
        patch: StructuredNotePatch,
        expectedWorkspaceKey: String
    ): NoteSaveResult {
        awaitInitialized()
        var accepted = false
        var expectedToken: String? = null
        val attempt = withContext(NonCancellable) {
            updateDataDetailed(
                expectedWorkspaceKey = expectedWorkspaceKey,
                persistImmediately = true,
                publishAfterDurable = true
            ) { data ->
                // This comparison is inside the existing write mutex, not only in Compose.
                // Apply one field to the CURRENT snapshot, preserving every unrelated value.
                val current = data.notes.singleOrNull { it.id == patch.noteId }
                    ?: return@updateDataDetailed data
                val candidate = applyStructuredPatch(current, patch)
                    ?: return@updateDataDetailed data
                expectedToken = structuredTargetToken(candidate, patch)
                accepted = true
                upsertNoteInData(data = data, note = candidate, timestamp = now())
            }
        }
        if (!accepted) return NoteSaveResult.failed(
            failure = NoteSaveFailure.READ_ONLY_PROTECTION,
            detail = "页面已改变、已锁定或此字段暂不支持修改。输入仍保留，请重新核对后编辑。"
        )
        if (!attempt.succeeded) return NoteSaveResult.failed(
            failure = attempt.failure ?: NoteSaveFailure.FLUSH_FAILED,
            detail = "保存未完成，输入仍保留；请检查存储和工作区后重试。"
        )
        val committed = attempt.snapshot?.notes?.singleOrNull { it.id == patch.noteId }
        if (committed == null || !structuredPatchVerified(committed, patch, expectedToken)) {
            return NoteSaveResult.failed(failure = NoteSaveFailure.FLUSH_FAILED,
                detail = "保存后的字段核对未通过，请重新打开检查，勿重复覆盖。")
        }
        return NoteSaveResult.committed()
    }

"####;

#[cfg(test)]
mod tests {
    use super::*;

    fn source(path: &str) -> &'static str {
        crate::kotlin_sources::SOURCES
            .iter()
            .find(|s| s.path.ends_with(path))
            .unwrap()
            .contents
    }

    #[test]
    fn real_structured_hooks_reject_missing_duplicate_and_reapplied_input() {
        for (path, hook) in [
            ("/TimerRepository.kt", "    suspend fun upsertNoteDurablyResult(\n"),
            ("/TimerViewModel.kt", "    fun upsertNoteAndFlush(\n"),
            ("/NoteStudioSheet.kt", "internal enum class NoteDraftPreset {\n    BLANK,\n    CHECKLIST,\n    MEETING,\n    REVIEW\n}"),
            ("/NoteDocumentEditor.kt", ") {\n    val context = LocalContext.current\n    val focusManager = LocalFocusManager.current\n    val titleFocusRequester = remember { FocusRequester() }"),
            ("/SmartisanNoteUi.kt", ") {\n    val context = androidx.compose.ui.platform.LocalContext.current\n    val workspaceKey = LocalNoteMediaWorkspaceKey.current"),
        ] {
            let base = source(path);
            let once = render(path, base).expect(path);
            assert!(render(path, &once).is_err(), "reapplied {path}");
            assert!(render(path, &base.replace(hook, "missing hook")).is_err(), "missing {path}");
            assert!(render(path, &format!("{base}\n{hook}")).is_err(), "duplicate {path}");
        }
        let path = "/NoteStudioSheet.kt";
        for name in ["buildPresetNote", "buildStickyPresetNote"] {
            let base = source(path).replace(&format!("internal fun {name}("), "missing preset(");
            assert!(render(path, &base).is_err(), "missing {name}");
        }
    }
}

#[path = "structured_mobile_data.rs"]
mod mobile_data;
#[path = "structured_mobile_tests.rs"]
mod mobile_tests;
pub use mobile_data::{EDIT_CONTENTS, POLICY_CONTENTS};
pub use mobile_tests::{EDIT_TEST_CONTENTS, TEST_CONTENTS};

pub const CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.listSaver
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import com.ofairyo.gridtimer.data.*
import kotlinx.serialization.json.*
import java.util.UUID

internal fun NoteEntry.hasStructuredKnowledge(): Boolean = document.knowledge != null || document.blocks.any { it.knowledge != null }

private val structuredPatchSaver = listSaver<StructuredNotePatch?, String>(
    save = { patch -> patch?.let { listOf(it.noteId, it.expectedUpdatedAt.toString(), it.action,
        it.blockId, it.key, it.row.toString(), it.column.toString(), it.expectedValue, it.value, it.parentId) } ?: emptyList() },
    restore = { if (it.size != 10) null else StructuredNotePatch(it[0], it[1].toLong(), it[2],
        it[3], it[4], it[5].toInt(), it[6].toInt(), it[7], it[8], it[9]) }
)

private fun knowledgeCellText(value: JsonElement?): String {
    val cell = value as? JsonObject ?: return ""
    val content = cell["value"]
    return when (cell.structuredText("kind")) {
        "date" -> structuredPropertyEditorValue(cell)
        "relation", "multi_select" -> (content as? JsonArray)?.joinToString("、") { (it as? JsonPrimitive)?.contentOrNull.orEmpty() }.orEmpty()
        "checkbox" -> if ((content as? JsonPrimitive)?.booleanOrNull == true) "已勾选" else "未勾选"
        else -> (content as? JsonPrimitive)?.contentOrNull.orEmpty()
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun StructuredKnowledgeReader(
    note: NoteEntry,
    onBack: () -> Unit,
    onWrite: ((StructuredNotePatch, (Boolean, String) -> Unit) -> Unit)? = null,
    onPreviewImage: (String) -> Unit
) {
    val workspace = LocalNoteMediaWorkspaceKey.current
    val document = note.resolvedDocument()
    val page = document.knowledge
    val editable = onWrite != null && note.canEditStructured()
    var saving by remember(note.id, workspace) { mutableStateOf(false) }
    var message by remember(note.id, workspace) { mutableStateOf("") }
    var draft by rememberSaveable(note.id, workspace, stateSaver = structuredPatchSaver) { mutableStateOf<StructuredNotePatch?>(null) }
    var outlineOpen by rememberSaveable(note.id, workspace) { mutableStateOf(false) }
    var insertOpen by rememberSaveable(note.id, workspace) { mutableStateOf(false) }
    var scrollTarget by remember(note.id, workspace) { mutableStateOf<String?>(null) }
    var collapsed by rememberSaveable(note.id, workspace) { mutableStateOf(
        document.blocks.filter { it.knowledge.structuredFlag("collapsed") }.map { it.id }) }
    val gate = remember(note.id, workspace) { StructuredSaveGate() }
    DisposableEffect(gate) { onDispose { gate.close() } }
    val listState = rememberLazyListState()
    val nodes = remember(document.blocks) { document.blocks.map { block ->
        StructuredOutlineNode(block.id, block.knowledge.structuredText("parentId"),
            block.knowledge.structuredText("kind").ifBlank { "paragraph" }, block.text)
    } }
    val visibleIds = remember(nodes, collapsed) { structuredVisibleNodes(nodes, collapsed.toSet()).map { it.id }.toSet() }
    val blocks = remember(document, visibleIds) { document.blocks.filter { it.id in visibleIds &&
        !(document.richTextEnabled && it.type == NoteBlockType.TEXT && it.knowledge == null) } }
    val outline = remember(nodes) { nodes.filter { it.kind in setOf("heading1", "heading2", "heading3") } }
    val depths = remember(nodes) { structuredNodeDepths(nodes) }
    val latestNote by rememberUpdatedState(note)
    val latestWorkspace by rememberUpdatedState(workspace)
    LaunchedEffect(editable) {
        if (!editable) { draft = null; insertOpen = false }
    }

    fun patch(action: String, blockId: String = "", key: String = "", row: Int = -1,
              column: Int = -1, value: String = "", parentId: String = ""): StructuredNotePatch? {
        val request = StructuredNotePatch(note.id, note.updatedAtEpochMillis, action, blockId, key, row, column, value = value, parentId = parentId)
        return structuredTargetToken(note, request)?.let { request.copy(expectedValue = it) }
    }
    fun submit(request: StructuredNotePatch) {
        val writer = onWrite ?: return
        if (!editable || saving || request.noteId != latestNote.id) return
        val ticket = gate.begin() ?: return
        saving = true; message = "正在保存…"
        try { writer(request) { ok, detail ->
            if (gate.finish(ticket, latestWorkspace == workspace && latestNote.id == request.noteId)) {
                saving = false
                message = if (ok) "本地已保存" else detail.ifBlank { "保存未完成，修改仍保留在输入框中。" }
                if (ok) draft = null
            }
        } } catch (_: Exception) {
            if (gate.finish(ticket, latestWorkspace == workspace)) {
                saving = false; message = "保存请求未能启动，输入仍保留，请重试。"
            }
        }
    }
    fun openDraft(request: StructuredNotePatch?) {
        if (request == null || saving || !editable) return
        if (request.value.length > 100_000 || request.expectedValue.length > 120_000) {
            message = "内容超过本次安全编辑容量，请在原编辑端拆分后再修改。"; return
        }
        message = ""; draft = request
    }
    fun locate(node: StructuredOutlineNode) {
        collapsed = structuredExpandTo(nodes, node.id, collapsed.toSet()).toList()
        outlineOpen = false; scrollTarget = node.id
    }
    LaunchedEffect(scrollTarget, blocks) {
        val target = scrollTarget ?: return@LaunchedEffect
        val index = blocks.indexOfFirst { it.id == target }
        if (index >= 0) { listState.animateScrollToItem(index + 1); scrollTarget = null }
    }
    BackHandler(enabled = draft == null && !outlineOpen && !insertOpen) { if (!saving) onBack() }
    Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.safeDrawing).imePadding()) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            TextButton(enabled = !saving, onClick = onBack) { Text("返回") }
            TextButton(onClick = { outlineOpen = true }, enabled = outline.isNotEmpty()) { Text("目录 ${outline.size}") }
            if (editable) TextButton(enabled = !saving, onClick = { insertOpen = true }) { Text("新增块") }
        }
        LazyColumn(state = listState, modifier = Modifier.weight(1f), contentPadding = PaddingValues(20.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)) {
            item(key = "structured_header") {
                Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    Text(page.structuredText("icon") + " " + note.title, style = MaterialTheme.typography.headlineMedium)
                    if (editable) TextButton(enabled = !saving, onClick = { openDraft(patch("title", value = note.title)) }) { Text("编辑标题") }
                    Text(if (editable) "结构页 · 点击编辑块或属性" else "只读结构页 · 原始内容受到保护",
                        style = MaterialTheme.typography.labelMedium)
                    if (message.isNotBlank()) Text(message, color = MaterialTheme.colorScheme.primary)
                    if (!editable) Text(when {
                        page.structuredFlag("locked") -> "页面已锁定，本机不会修改内容。"
                        note.encryption != null -> "加密结构页暂只读，不在此路径产生明文修改。"
                        document.richTextEnabled -> "复杂富文本暂只读，可继续在原编辑端修改。"
                        else -> "当前页面版本或入口暂不支持安全编辑。"
                    }, style = MaterialTheme.typography.bodySmall)
                    val tags = page?.get("tags") as? JsonArray
                    if (tags != null && tags.isNotEmpty()) Text(tags.joinToString(" · ") { (it as? JsonPrimitive)?.contentOrNull.orEmpty() })
                    if (editable) TextButton(enabled = !saving, onClick = {
                        openDraft(patch("tags", value = tags?.joinToString("\n") { (it as? JsonPrimitive)?.contentOrNull.orEmpty() }.orEmpty()))
                    }) { Text("编辑标签") }
                    (page?.get("properties") as? JsonObject)?.forEach { (key, value) ->
                        val cell = value as? JsonObject
                        val label = when (key) { "status" -> "状态"; "priority" -> "优先级"; "date" -> "日期"; else -> key }
                        val writable = editable && cell?.structuredText("kind") in structuredPropertyKinds &&
                            page.structuredPropertiesEditable()
                        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            Column(Modifier.weight(1f)) { Text(label, style = MaterialTheme.typography.labelMedium); Text(knowledgeCellText(value).ifBlank { "未填写" }) }
                            if (writable) TextButton(enabled = !saving, onClick = {
                                openDraft(patch("property", key = key, value = structuredPropertyEditorValue(cell!!)))
                            }) { Text("编辑") }
                        }
                    }
                    (page?.get("database") as? JsonObject)?.let { database ->
                        Text("多维表结构 · 当前只读", style = MaterialTheme.typography.titleMedium)
                        (database["fields"] as? JsonArray)?.forEach { field ->
                            val item = field as? JsonObject
                            if (!item.structuredFlag("deleted")) Text(item.structuredText("name"))
                        }
                    }
                    if (document.richTextEnabled && document.richTextPlainText.isNotBlank()) Text(document.richTextPlainText)
                }
            }
            itemsIndexed(blocks, key = { index, block -> "structured_block_${block.id}_$index" }) { _, block ->
                val meta = block.knowledge
                val kind = meta.structuredText("kind").ifBlank { "paragraph" }
                Column(Modifier.fillMaxWidth().padding(start = ((depths[block.id] ?: 0) * 10).dp),
                    verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    when {
                        block.type == NoteBlockType.IMAGE -> TextButton(onClick = { block.attachmentId?.let(onPreviewImage) }) { Text(block.caption.ifBlank { "查看图片" }) }
                        kind == "toggle" -> TextButton(onClick = {
                            collapsed = if (block.id in collapsed) collapsed - block.id else collapsed + block.id
                        }) { Text((if (block.id in collapsed) "展开  " else "收起  ") + block.text.ifBlank { "折叠内容" }) }
                        kind == "divider" -> HorizontalDivider()
                        kind == "todo" -> Row {
                            Checkbox(checked = meta.structuredFlag("checked"), enabled = editable && !saving,
                                onCheckedChange = if (editable) { checked -> patch("checked", block.id, value = checked.toString())?.let(::submit) } else null)
                            Text(block.text.ifBlank { "待办事项" }, Modifier.weight(1f).padding(top = 12.dp))
                        }
                        kind == "heading1" -> Text(block.text, style = MaterialTheme.typography.headlineSmall)
                        kind == "heading2" -> Text(block.text, style = MaterialTheme.typography.titleLarge)
                        kind == "heading3" -> Text(block.text, style = MaterialTheme.typography.titleMedium)
                        kind == "table" -> {
                            val table = meta?.get("table") as? JsonArray ?: JsonArray(emptyList())
                            Column(Modifier.horizontalScroll(rememberScrollState())) {
                                table.forEachIndexed { rowIndex, row -> Row {
                                    (row as? JsonArray)?.forEachIndexed { columnIndex, cell ->
                                        val value = (cell as? JsonPrimitive)?.contentOrNull.orEmpty()
                                        if (editable) OutlinedButton(enabled = !saving, modifier = Modifier.width(150.dp).heightIn(min = 48.dp),
                                            onClick = { openDraft(patch("cell", block.id, row = rowIndex, column = columnIndex, value = value)) }) {
                                            Text(value.ifBlank { "空白单元格" })
                                        } else Text(value, Modifier.width(150.dp).padding(8.dp))
                                    }
                                } }
                            }
                            if (editable) FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                TextButton(enabled = !saving, onClick = { patch("table_row", block.id)?.let(::submit) }) { Text("加一行") }
                                TextButton(enabled = !saving, onClick = { patch("table_column", block.id)?.let(::submit) }) { Text("加一列") }
                            }
                        }
                        kind == "code" || kind == "equation" -> Surface(color = MaterialTheme.colorScheme.surfaceVariant, shape = MaterialTheme.shapes.small) {
                            Text(block.text, Modifier.fillMaxWidth().padding(12.dp), fontFamily = FontFamily.Monospace)
                        }
                        kind == "quote" || kind == "callout" -> Surface(color = MaterialTheme.colorScheme.surfaceVariant, shape = MaterialTheme.shapes.small) {
                            Text(block.text, Modifier.fillMaxWidth().padding(12.dp))
                        }
                        kind == "bulleted_list" -> Text("• " + block.text)
                        kind == "numbered_list" -> Text("· " + block.text)
                        kind == "table_of_contents" -> TextButton(onClick = { outlineOpen = true }) { Text("查看页面目录") }
                        kind == "columns" -> Text("分栏内容", style = MaterialTheme.typography.labelSmall)
                        kind in setOf("file", "audio", "video") -> {
                            val attachment = note.attachments.find { it.id == block.attachmentId }
                            Text(attachment?.displayName ?: block.text.ifBlank { "附件" })
                            if (attachment != null) Text("${attachment.sizeBytes} 字节", style = MaterialTheme.typography.labelSmall)
                            else if (meta.structuredText("url").isNotBlank()) Text(meta.structuredText("url"))
                        }
                        else -> Text(block.text.ifBlank { "空白内容" })
                    }
                    if (editable && kind == "toggle") TextButton(enabled = !saving, onClick = {
                        collapsed = collapsed - block.id
                        openDraft(patch("append", UUID.randomUUID().toString(), key = "paragraph", parentId = block.id))
                    }) { Text("添加子块") }
                    if (editable && block.type == NoteBlockType.TEXT && kind in structuredTextKinds) {
                        TextButton(enabled = !saving, onClick = { openDraft(patch("text", block.id, value = block.text)) }) { Text("编辑块") }
                    }
                }
            }
            item(key = "structured_comments") {
                (page?.get("comments") as? JsonArray)?.filter { (it as? JsonObject)?.get("deletedAt") == JsonNull ||
                    (it as? JsonObject)?.get("deletedAt") == null }?.takeIf { it.isNotEmpty() }?.let { comments ->
                    Column { HorizontalDivider(); Text("批注", style = MaterialTheme.typography.titleMedium)
                        comments.forEach { item -> val comment = item as? JsonObject
                            Text((if (comment.structuredFlag("resolved")) "已解决 · " else "") + comment.structuredText("body"))
                        }
                    }
                }
            }
        }
    }
    if (outlineOpen) AlertDialog(onDismissRequest = { outlineOpen = false }, title = { Text("页面目录") }, text = {
        LazyColumn(Modifier.heightIn(max = 420.dp)) {
            if (outline.isEmpty()) item { Text("添加标题块后可在此定位。") }
            items(outline) { node -> TextButton(modifier = Modifier.fillMaxWidth(), onClick = { locate(node) }) {
                Text(node.title.ifBlank { "未命名标题" }, Modifier.fillMaxWidth().padding(start = ((node.kind.last().digitToInt() - 1) * 12).dp))
            } }
        }
    }, confirmButton = { TextButton(onClick = { outlineOpen = false }) { Text("关闭") } })
    if (insertOpen) AlertDialog(onDismissRequest = { insertOpen = false }, title = { Text("新增内容块") }, text = {
        Column { listOf("paragraph" to "正文", "heading2" to "标题", "todo" to "待办", "toggle" to "折叠块", "table" to "简单表格").forEach { (kind, label) ->
            TextButton(onClick = {
                insertOpen = false
                val request = patch("append", UUID.randomUUID().toString(), key = kind)
                if (kind == "table") request?.let(::submit) else openDraft(request)
            }) { Text(label) }
        } }
    }, confirmButton = { TextButton(onClick = { insertOpen = false }) { Text("取消") } })
    val currentDraft = draft
    if (currentDraft != null && editable) {
        val cell = (page?.get("properties") as? JsonObject)?.get(currentDraft.key) as? JsonObject
        val inputKind = when (currentDraft.action) { "property" -> cell.structuredText("kind"); "tags" -> "tags"; "title" -> "title"; else -> "text" }
        val validation = structuredInputError(inputKind, currentDraft.value)
        val stale = !structuredPatchCurrent(note.id, note.updatedAtEpochMillis, structuredTargetToken(note, currentDraft), currentDraft)
        AlertDialog(onDismissRequest = { if (!saving) draft = null }, title = { Text(if (currentDraft.action == "property") "编辑 ${currentDraft.key}" else "编辑内容") },
            text = { Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                if (inputKind == "checkbox") Row { Checkbox(currentDraft.value == "true", { draft = currentDraft.copy(value = it.toString()) }, enabled = !saving); Text("已勾选") }
                else OutlinedTextField(value = currentDraft.value, onValueChange = { draft = currentDraft.copy(value = it) },
                    modifier = Modifier.fillMaxWidth().heightIn(max = 300.dp), enabled = !saving,
                    label = { Text(when (inputKind) { "date" -> "YYYY-MM-DD 或 开始 / 结束"; "tags" -> "每行一个标签"; else -> "内容" }) })
                if (validation != null) Text(validation, color = MaterialTheme.colorScheme.error)
                if (stale) Text("页面已更新或目标已改变。输入仍保留，请复制后取消并重新打开编辑，不会覆盖新版本。", color = MaterialTheme.colorScheme.error)
                if (message.isNotBlank()) Text(message)
            } },
            confirmButton = { TextButton(enabled = editable && !saving && validation == null && !stale, onClick = { submit(currentDraft) }) { Text(if (saving) "保存中" else "保存") } },
            dismissButton = { TextButton(enabled = !saving, onClick = { draft = null }) { Text("取消") } })
    }
}
"####;
