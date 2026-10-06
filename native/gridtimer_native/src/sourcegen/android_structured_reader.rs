//! Mobile reading tools for structured pages; no document mutations.
pub const CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui

import android.content.ClipData
import android.content.ClipboardManager
import android.widget.Toast
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.ofairyo.gridtimer.data.NoteEntry
import com.ofairyo.gridtimer.data.NoteBlock
import com.ofairyo.gridtimer.data.NoteBlockType
import com.ofairyo.gridtimer.data.NoteDocument
import com.ofairyo.gridtimer.data.isEncryptionLocked
import com.ofairyo.gridtimer.data.resolvedDocument
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.*

internal fun NoteEntry.hasStructuredKnowledge(): Boolean = document.knowledge != null || document.blocks.any { it.knowledge != null }
private fun JsonObject?.text(name: String): String = (this?.get(name) as? JsonPrimitive)?.contentOrNull.orEmpty()
private fun JsonObject?.flag(name: String): Boolean = text(name) == "true"
private fun knowledgeCellText(value: JsonElement?): String {
    val cell = value as? JsonObject ?: return ""
    val content = cell["value"]
    return when (cell.text("kind")) {
        "date" -> (content as? JsonObject)?.let { listOf(it.text("start"), it.text("end")).filter(String::isNotBlank).distinct().joinToString(" 至 ") }.orEmpty()
        "relation", "multi_select" -> (content as? JsonArray)?.joinToString("、") { (it as? JsonPrimitive)?.contentOrNull.orEmpty() }.orEmpty()
        "checkbox" -> if ((content as? JsonPrimitive)?.booleanOrNull == true) "已勾选" else "未勾选"
        else -> (content as? JsonPrimitive)?.contentOrNull.orEmpty()
    }
}
private data class ReaderData(val blocks: List<NoteBlock>, val plan: ReaderPlan)

@Composable
internal fun StructuredKnowledgeReader(note: NoteEntry, onBack: () -> Unit, onPreviewImage: (String) -> Unit) {
    // Preserve the existing secure-window owner and do not read a locked payload.
    if (note.isEncryptionLocked()) {
        BackHandler(onBack = onBack)
        Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.safeDrawing).padding(20.dp)) {
            TextButton(onClick = onBack) { Text("返回") }
            Text("页面已锁定，请返回后解锁。")
        }
        return
    }
    val document = note.resolvedDocument()
    val workspace = LocalNoteMediaWorkspaceKey.current
    // No plaintext/query/folding state is restored into another document or account.
    key(note.id, workspace, document) {
        StructuredReaderBody(note, document, onBack, onPreviewImage)
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun StructuredReaderBody(note: NoteEntry, document: NoteDocument, onBack: () -> Unit, onPreviewImage: (String) -> Unit) {
    val context = LocalContext.current
    val keyboard = LocalSoftwareKeyboardController.current
    val listState = rememberLazyListState()
    var data by remember { mutableStateOf<ReaderData?>(null) }
    var failed by remember { mutableStateOf(false) }
    var attempt by remember { mutableStateOf(0) }
    var folded by remember { mutableStateOf<Set<Int>>(emptySet()) }
    var dialog by remember { mutableStateOf<String?>(null) }
    var query by remember { mutableStateOf("") }
    var search by remember { mutableStateOf<Pair<String, List<Int>>?>(null) }
    var pendingTarget by remember { mutableStateOf<Int?>(null) }
    var highlighted by remember { mutableStateOf<Int?>(null) }

    LaunchedEffect(document, attempt) {
        failed = false
        try {
            val loaded = withContext(Dispatchers.Default) {
                val blocks = document.blocks.filter {
                    !(document.richTextEnabled && it.type == NoteBlockType.TEXT && it.knowledge == null)
                }
                val input = blocks.map { block ->
                    coroutineContext.ensureActive()
                    val meta = block.knowledge
                    val table = (meta?.get("table") as? JsonArray)?.joinToString("\n") { row ->
                        (row as? JsonArray)?.joinToString("\t") { (it as? JsonPrimitive)?.contentOrNull.orEmpty() }.orEmpty()
                    }.orEmpty()
                    ReaderBlock(block.id, meta.text("kind"),
                        listOf(block.text, block.caption, table).filter(String::isNotBlank).joinToString("\n"),
                        meta.text("parentId"), meta.flag("collapsed"), meta.flag("checked"))
                }
                ReaderData(blocks, readerPlan(input) { coroutineContext.ensureActive() })
            }
            folded = readerInitialFolds(loaded.plan)
            data = loaded
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { failed = true }
    }
    LaunchedEffect(data, query) {
        val current = data ?: return@LaunchedEffect
        val captured = query
        val found = withContext(Dispatchers.Default) {
            readerMatches(current.plan, captured) { coroutineContext.ensureActive() }
        }
        search = captured to found
    }
    val current = data
    val visible = remember(current, folded) { current?.let { readerVisible(it.plan, folded) }.orEmpty() }
    fun locate(index: Int) {
        val snapshot = data ?: return
        if (index !in snapshot.blocks.indices) return
        folded = readerReveal(snapshot.plan, index, folded)
        highlighted = index
        pendingTarget = index
        dialog = null
        keyboard?.hide()
    }
    LaunchedEffect(pendingTarget, visible) {
        val target = pendingTarget ?: return@LaunchedEffect
        val position = readerLazyPosition(visible, target) ?: return@LaunchedEffect
        listState.animateScrollToItem(position)
        pendingTarget = null
    }
    BackHandler { if (dialog != null) dialog = null else onBack() }

    Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.safeDrawing).imePadding()) {
        FlowRow(Modifier.fillMaxWidth().padding(horizontal = 8.dp), horizontalArrangement = Arrangement.SpaceBetween) {
            TextButton(onClick = onBack) { Text("返回") }
            TextButton(onClick = { dialog = "outline" }, enabled = current != null) { Text("目录") }
            TextButton(onClick = { dialog = "search" }, enabled = current != null) { Text("查找正文") }
            TextButton(onClick = { dialog = "tasks" }, enabled = current != null) { Text("待办定位") }
        }
        if (failed) {
            Column(Modifier.padding(20.dp)) {
                Text("页面结构暂时无法整理，原文未被修改。")
                TextButton(onClick = { attempt++ }) { Text("重试") }
            }
        } else if (current == null) {
            Column(Modifier.padding(20.dp)) { CircularProgressIndicator(); Text("正在整理页面结构…") }
        } else {
            LazyColumn(state = listState, modifier = Modifier.weight(1f),
                contentPadding = PaddingValues(20.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                item(key = "reader_metadata") {
                    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                        val page = document.knowledge
                        Text(page.text("icon") + " " + note.title, style = MaterialTheme.typography.headlineMedium)
                        Text("结构化页面 · 正文仍在电脑端编辑；折叠和定位仅影响本次阅读。",
                            style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        val tasks = current.plan.tasks
                        if (tasks.isNotEmpty()) Text("待办：已完成 ${tasks.size - current.plan.pending.size} / ${tasks.size}")
                        if (current.plan.blocks.any { it.kind == "toggle" }) {
                            FlowRow {
                                TextButton(onClick = { folded = emptySet(); pendingTarget = null }) { Text("展开全部") }
                                TextButton(onClick = {
                                    folded = current.plan.blocks.indices.filter { current.plan.blocks[it].kind == "toggle" }.toSet()
                                    pendingTarget = null
                                }) { Text("收起全部") }
                            }
                        }
                        if (current.plan.malformed.isNotEmpty()) Text("${current.plan.malformed.size} 个块的层级异常，已展开显示，不隐藏原内容。")
                        (page?.get("tags") as? JsonArray)?.takeIf { it.isNotEmpty() }?.let { tags ->
                            Text(tags.joinToString(" · ") { (it as? JsonPrimitive)?.contentOrNull.orEmpty() }, color = MaterialTheme.colorScheme.primary)
                        }
                        (page?.get("properties") as? JsonObject)?.forEach { (name, value) ->
                            val label = when (name) { "status" -> "状态"; "priority" -> "优先级"; "date" -> "日期"; else -> name }
                            Text("$label：${knowledgeCellText(value)}")
                        }
                        (page?.get("database") as? JsonObject)?.let { database ->
                            Text("数据库字段", style = MaterialTheme.typography.titleMedium)
                            (database["fields"] as? JsonArray)?.forEach { field ->
                                val value = field as? JsonObject
                                if (!value.flag("deleted")) Text(value.text("name"))
                            }
                        }
                        if (document.richTextEnabled && document.richTextPlainText.isNotBlank()) Text(document.richTextPlainText)
                    }
                }
                items(visible, key = { "reader_block_$it" }) { index ->
                    val block = current.blocks[index]
                    val meta = block.knowledge
                    val kind = meta.text("kind")
                    val depth = current.plan.ancestors[index].size.coerceAtMost(8)
                    Surface(modifier = Modifier.fillMaxWidth().padding(start = (depth * 10).dp),
                        color = MaterialTheme.colorScheme.surface,
                        border = if (highlighted == index) BorderStroke(1.dp, MaterialTheme.colorScheme.primary) else null,
                        shape = MaterialTheme.shapes.small) {
                        Column(Modifier.fillMaxWidth().padding(4.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                            when {
                                block.type == NoteBlockType.IMAGE -> TextButton(onClick = { block.attachmentId?.let(onPreviewImage) }) { Text(block.caption.ifBlank { "查看图片" }) }
                                kind == "toggle" -> TextButton(onClick = {
                                    folded = readerToggle(current.plan, index, folded); pendingTarget = null
                                }, modifier = Modifier.fillMaxWidth()) {
                                    Text((if (index in folded) "展开：" else "收起：") + block.text.ifBlank { "折叠块" }, Modifier.fillMaxWidth())
                                }
                                kind == "divider" -> HorizontalDivider()
                                kind == "todo" -> Row {
                                    Checkbox(checked = meta.flag("checked"), onCheckedChange = null)
                                    Text(block.text, Modifier.weight(1f).padding(top = 12.dp))
                                }
                                kind == "heading1" -> Text(block.text, style = MaterialTheme.typography.headlineSmall)
                                kind == "heading2" -> Text(block.text, style = MaterialTheme.typography.titleLarge)
                                kind == "heading3" -> Text(block.text, style = MaterialTheme.typography.titleMedium)
                                kind == "table_of_contents" -> TextButton(onClick = { dialog = "outline" }) { Text("查看本页目录（${current.plan.headings.size} 项）") }
                                kind == "table" -> Column(Modifier.horizontalScroll(rememberScrollState())) {
                                    val header = (meta?.get("tableHeader") as? JsonPrimitive)?.booleanOrNull != false
                                    (meta?.get("table") as? JsonArray)?.forEachIndexed { rowIndex, row ->
                                        Row {
                                            (row as? JsonArray)?.forEach { cell ->
                                                Text((cell as? JsonPrimitive)?.contentOrNull.orEmpty(), Modifier.width(150.dp).padding(8.dp),
                                                    fontWeight = if (header && rowIndex == 0) FontWeight.Bold else FontWeight.Normal)
                                            }
                                        }
                                    }
                                }
                                kind == "code" || kind == "equation" -> {
                                    TextButton(onClick = {
                                        val copied = runCatching {
                                            requireNotNull(context.getSystemService(ClipboardManager::class.java))
                                                .setPrimaryClip(ClipData.newPlainText("内容", block.text))
                                        }.isSuccess
                                        Toast.makeText(context, if (copied) "已复制" else "复制失败，请重试。", Toast.LENGTH_SHORT).show()
                                    }) { Text(if (kind == "code") "复制代码" else "复制公式源码") }
                                    Surface(color = MaterialTheme.colorScheme.surfaceVariant, shape = MaterialTheme.shapes.small) {
                                        Text(block.text, Modifier.fillMaxWidth().padding(12.dp), fontFamily = FontFamily.Monospace)
                                    }
                                }
                                kind == "quote" || kind == "callout" -> Surface(color = MaterialTheme.colorScheme.surfaceVariant, shape = MaterialTheme.shapes.small) {
                                    Text(block.text, Modifier.fillMaxWidth().padding(12.dp))
                                }
                                kind == "bulleted_list" -> Text("• " + block.text)
                                kind == "numbered_list" -> Text("${current.plan.ordinals[index] ?: 1}. ${block.text}")
                                kind == "columns" -> {
                                    Text("分栏内容（手机按原顺序显示）", style = MaterialTheme.typography.labelSmall)
                                    if (block.text.isNotBlank()) Text(block.text)
                                }
                                kind == "file" || kind == "audio" || kind == "video" -> {
                                    val file = note.attachments.find { it.id == block.attachmentId }
                                    Text(file?.displayName ?: block.text.ifBlank { "附件" }, style = MaterialTheme.typography.titleSmall)
                                    if (file != null) Text("${file.sizeBytes} 字节", style = MaterialTheme.typography.labelSmall)
                                    else if (meta.text("url").isNotBlank()) Text(meta.text("url"))
                                }
                                else -> Text(block.text)
                            }
                        }
                    }
                }
                item(key = "reader_comments") {
                    val comments = (document.knowledge?.get("comments") as? JsonArray).orEmpty()
                    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        comments.filter { (it as? JsonObject)?.get("deletedAt") == JsonNull || (it as? JsonObject)?.get("deletedAt") == null }
                            .forEach { comment ->
                                val value = comment as? JsonObject
                                Text((if (value.flag("resolved")) "已解决批注：" else "批注：") + value.text("body"))
                            }
                    }
                }
            }
        }
    }
    val mode = dialog
    if (mode != null && current != null) {
        val matches = search?.takeIf { it.first == query }?.second
        val choices = when (mode) { "outline" -> current.plan.headings; "tasks" -> current.plan.pending; else -> matches.orEmpty() }
        AlertDialog(onDismissRequest = { dialog = null },
            title = { Text(when (mode) { "outline" -> "本页目录"; "tasks" -> "未完成待办"; else -> "查找正文块" }) },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    if (mode == "search") {
                        OutlinedTextField(value = query, onValueChange = { query = it }, singleLine = true,
                            label = { Text("输入要查找的文字") }, modifier = Modifier.fillMaxWidth())
                        Text("查找正文块、图片说明和表格；富文本区、属性和批注不纳入定位。", style = MaterialTheme.typography.labelSmall)
                    }
                    if (mode == "tasks") Text("点击定位，不改变完成状态。")
                    Text(when {
                        mode == "search" && query.isBlank() -> "请输入关键词。"
                        mode == "search" && matches == null -> "正在查找…"
                        choices.isEmpty() -> "没有匹配项目。"
                        else -> "共 ${choices.size} 项；点击后自动展开所属折叠块并定位。"
                    })
                    LazyColumn(Modifier.fillMaxWidth().heightIn(max = 360.dp)) {
                        items(choices, key = { it }) { index ->
                            TextButton(onClick = { locate(index) }, modifier = Modifier.fillMaxWidth()) {
                                Text(current.plan.blocks[index].text.ifBlank { "未命名块" }, Modifier.fillMaxWidth(), maxLines = 3, overflow = TextOverflow.Ellipsis)
                            }
                        }
                    }
                }
            },
            confirmButton = { TextButton(onClick = { dialog = null }) { Text("关闭") } })
    }
}

// BEGIN STRUCTURED READER POLICY
internal data class ReaderBlock(
    val id: String, val kind: String, val text: String,
    val parent: String = "", val collapsed: Boolean = false, val checked: Boolean = false
)
internal data class ReaderPlan(
    val blocks: List<ReaderBlock>, val ancestors: List<List<Int>>,
    val ordinals: List<Int?>, val malformed: Set<Int>
) {
    val headings: List<Int> get() = blocks.indices.filter { blocks[it].kind in setOf("heading1", "heading2", "heading3") }
    val tasks: List<Int> get() = blocks.indices.filter { blocks[it].kind == "todo" }
    val pending: List<Int> get() = tasks.filter { !blocks[it].checked }
}

// Invalid/ambiguous ancestry is displayed at the root, never silently hidden.
internal fun readerPlan(input: List<ReaderBlock>, checkpoint: () -> Unit = {}): ReaderPlan {
    val blocks = input.toList()
    val byId = blocks.indices.groupBy { blocks[it].id }
    val malformed = mutableSetOf<Int>()
    val ancestors = blocks.indices.map { index ->
        checkpoint()
        val seen = mutableSetOf(index)
        val chain = mutableListOf<Int>()
        var parent = blocks[index].parent
        var valid = true
        while (parent.isNotEmpty()) {
            val candidates = byId[parent].orEmpty()
            val target = candidates.singleOrNull()
            if (target == null || !seen.add(target) || chain.size >= 64) {
                valid = false
                break
            }
            chain += target
            parent = blocks[target].parent
        }
        if (!valid) { malformed += index; emptyList() } else chain.toList()
    }
    val runs = mutableMapOf<Int?, Int>()
    val ordinals = blocks.indices.map { index ->
        checkpoint()
        val parent = ancestors[index].firstOrNull()
        if (blocks[index].kind == "numbered_list") {
            ((runs[parent] ?: 0) + 1).also { runs[parent] = it }
        } else { runs[parent] = 0; null }
    }
    return ReaderPlan(blocks, ancestors, ordinals, malformed.toSet())
}
internal fun readerInitialFolds(plan: ReaderPlan): Set<Int> = plan.blocks.indices.filter {
    plan.blocks[it].kind == "toggle" && plan.blocks[it].collapsed
}.toSet()
internal fun readerVisible(plan: ReaderPlan, folded: Set<Int>): List<Int> = plan.blocks.indices.filter { index ->
    plan.ancestors[index].none { it in folded && plan.blocks[it].kind == "toggle" }
}
internal fun readerReveal(plan: ReaderPlan, target: Int, folded: Set<Int>): Set<Int> =
    if (target in plan.blocks.indices) folded - plan.ancestors[target].toSet() else folded
internal fun readerToggle(plan: ReaderPlan, target: Int, folded: Set<Int>): Set<Int> =
    if (target !in plan.blocks.indices || plan.blocks[target].kind != "toggle") folded
    else if (target in folded) folded - target else folded + target
internal fun readerMatches(plan: ReaderPlan, query: String, checkpoint: () -> Unit = {}): List<Int> {
    val term = query.trim()
    if (term.isEmpty()) return emptyList()
    return plan.blocks.indices.filter { index ->
        checkpoint()
        plan.blocks[index].text.contains(term, ignoreCase = true)
    }
}
internal fun readerLazyPosition(visible: List<Int>, target: Int): Int? =
    visible.indexOf(target).takeIf { it >= 0 }?.plus(1) // The metadata item is always first.
// END STRUCTURED READER POLICY
"####;
