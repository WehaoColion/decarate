// v2.22.47 - Read advanced Windows pages without passing them through a flat editor.

pub const PATH: &str = "com/ofairyo/gridtimer/ui/StructuredKnowledgeReader.kt";
pub const CONTENTS: &str = r#"package com.ofairyo.gridtimer.ui

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import com.ofairyo.gridtimer.data.NoteEntry
import com.ofairyo.gridtimer.data.NoteBlockType
import com.ofairyo.gridtimer.data.resolvedDocument
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

@Composable
internal fun StructuredKnowledgeReader(note: NoteEntry, onBack: () -> Unit, onPreviewImage: (String) -> Unit) {
    val document = note.resolvedDocument()
    val page = document.knowledge
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(20.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
        TextButton(onClick = onBack) { Text("返回") }
        Text(page.text("icon") + " " + note.title, style = MaterialTheme.typography.headlineMedium)
        Text("结构化页面 · 在电脑端编辑", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        (page?.get("tags") as? JsonArray)?.takeIf { it.isNotEmpty() }?.let { tags ->
            Text(tags.joinToString(" · ") { (it as? JsonPrimitive)?.contentOrNull.orEmpty() }, color = MaterialTheme.colorScheme.primary)
        }
        (page?.get("properties") as? JsonObject)?.forEach { (key, value) ->
            val label = when (key) { "status" -> "状态"; "priority" -> "优先级"; "date" -> "日期"; else -> key }
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(16.dp)) { Text(label, Modifier.width(96.dp), color = MaterialTheme.colorScheme.onSurfaceVariant); Text(knowledgeCellText(value), Modifier.weight(1f)) }
        }
        (page?.get("database") as? JsonObject)?.let { database ->
            Text("数据库", style = MaterialTheme.typography.titleMedium)
            (database["fields"] as? JsonArray)?.forEach { field ->
                val item = field as? JsonObject
                if (!item.flag("deleted")) Text(item.text("name"))
            }
        }
        if (document.richTextEnabled && document.richTextPlainText.isNotBlank()) Text(document.richTextPlainText)
        document.blocks.filter { !(document.richTextEnabled && it.type == NoteBlockType.TEXT && it.knowledge == null) }.forEach { block ->
            val meta = block.knowledge
            val kind = meta.text("kind")
            val parents = mutableSetOf(block.id)
            var parent = meta.text("parentId")
            var depth = 0
            while (parent.isNotBlank() && parents.add(parent) && depth < 12) { depth += 1; parent = document.blocks.find { it.id == parent }?.knowledge.text("parentId") }
            Column(Modifier.fillMaxWidth().padding(start = (depth * 10).dp)) {
                when {
                    block.type == NoteBlockType.IMAGE -> TextButton(onClick = { block.attachmentId?.let(onPreviewImage) }) { Text(block.caption.ifBlank { "查看图片" }) }
                    kind == "divider" -> HorizontalDivider()
                    kind == "todo" -> Row { Checkbox(checked = meta.flag("checked"), onCheckedChange = null); Text(block.text, Modifier.padding(top = 12.dp)) }
                    kind == "heading1" -> Text(block.text, style = MaterialTheme.typography.headlineSmall)
                    kind == "heading2" -> Text(block.text, style = MaterialTheme.typography.titleLarge)
                    kind == "heading3" -> Text(block.text, style = MaterialTheme.typography.titleMedium)
                    kind == "table" -> Column(Modifier.horizontalScroll(rememberScrollState())) {
                        (meta?.get("table") as? JsonArray)?.forEach { row -> Row { (row as? JsonArray)?.forEach { cell -> Text((cell as? JsonPrimitive)?.contentOrNull.orEmpty(), Modifier.width(150.dp).padding(8.dp)) } } }
                    }
                    kind == "code" || kind == "equation" -> Surface(color = MaterialTheme.colorScheme.surfaceVariant, shape = MaterialTheme.shapes.small) { Text(block.text, Modifier.fillMaxWidth().padding(12.dp), fontFamily = FontFamily.Monospace) }
                    kind == "quote" || kind == "callout" -> Surface(color = MaterialTheme.colorScheme.surfaceVariant, shape = MaterialTheme.shapes.small) { Text(block.text, Modifier.fillMaxWidth().padding(12.dp)) }
                    kind == "bulleted_list" -> Text("• " + block.text)
                    kind == "numbered_list" -> Text("· " + block.text)
                    kind == "columns" || kind == "table_of_contents" -> Unit
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
        (page?.get("comments") as? JsonArray)?.filter { (it as? JsonObject)?.get("deletedAt") == JsonNull || (it as? JsonObject)?.get("deletedAt") == null }?.takeIf { it.isNotEmpty() }?.let { comments ->
            HorizontalDivider(); Text("批注", style = MaterialTheme.typography.titleMedium)
            comments.forEach { comment -> val item = comment as? JsonObject; Text((if (item.flag("resolved")) "已解决 · " else "") + item.text("body")) }
        }
        Spacer(Modifier.height(120.dp))
    }
}
"#;

pub fn render(path: &str, source: &str) -> Result<String, String> {
    if path.ends_with("/NoteEditorUi.kt") || source.contains("internal fun NoteEditorContent(") {
        let needle = ") {\n    val context = LocalContext.current\n    val focusManager = LocalFocusManager.current\n    val titleFocusRequester = remember { FocusRequester() }";
        if !source.contains(needle) {
            return Err("structured document reader insertion point missing".into());
        }
        return Ok(source.replacen(needle, ") {\n    if (note.hasStructuredKnowledge()) {\n        EncryptedNoteSecureWindowEffect(note.encryption != null)\n        StructuredKnowledgeReader(note, onBack, onRequestPreviewAttachment)\n        return\n    }\n    val context = LocalContext.current\n    val focusManager = LocalFocusManager.current\n    val titleFocusRequester = remember { FocusRequester() }", 1));
    }
    if source.contains("internal fun SmartisanNoteEditorContent(") {
        let needle = ") {\n    val context = androidx.compose.ui.platform.LocalContext.current\n    val workspaceKey = LocalNoteMediaWorkspaceKey.current";
        if !source.contains(needle) {
            return Err("structured sticky reader insertion point missing".into());
        }
        return Ok(source.replacen(needle, ") {\n    if (note.hasStructuredKnowledge()) {\n        EncryptedNoteSecureWindowEffect(note.encryption != null)\n        StructuredKnowledgeReader(note, onBack) { id -> onRequestPreviewAttachment(null, id) }\n        return\n    }\n    val context = androidx.compose.ui.platform.LocalContext.current\n    val workspaceKey = LocalNoteMediaWorkspaceKey.current", 1));
    }
    Ok(source.into())
}
