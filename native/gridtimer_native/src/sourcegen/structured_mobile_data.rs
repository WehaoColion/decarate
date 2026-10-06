// Rust-owned source strings for the mobile structured-page feature.

pub const POLICY_CONTENTS: &str = r####"package com.ofairyo.gridtimer.data

import java.time.LocalDate

/** A field patch, never a stale replacement of the complete page. */
internal data class StructuredNotePatch(
    val noteId: String,
    val expectedUpdatedAt: Long,
    val action: String,
    val blockId: String = "",
    val key: String = "",
    val row: Int = -1,
    val column: Int = -1,
    val expectedValue: String = "",
    val value: String = "",
    val parentId: String = ""
)

internal val structuredTextKinds = setOf(
    "paragraph", "heading1", "heading2", "heading3", "bulleted_list", "numbered_list",
    "todo", "quote", "callout", "toggle", "code", "equation"
)
internal val structuredAppendKinds = setOf("paragraph", "heading2", "todo", "toggle", "table")
internal val structuredPropertyKinds = setOf("text", "number", "checkbox", "url", "date")

internal fun structuredInputError(kind: String, value: String): String? = when {
    kind == "title" && (value.isBlank() || value.length > 200 || value.contains('\n') || value.contains('\r')) -> "标题需为 1 至 200 个字符的单行文字。"
    value.length > 100_000 -> "单次编辑最多 100000 个字符，请分块保存。"
    kind == "number" && value.trim().toDoubleOrNull()?.isFinite() != true -> "请输入有效的有限数字。"
    kind == "checkbox" && value !in setOf("true", "false") -> "勾选值无效。"
    kind == "date" && !structuredDateValid(value) -> "日期使用 YYYY-MM-DD；日期范围用 / 分隔，结束不能早于开始。"
    kind == "url" && value.isNotBlank() && !runCatching {
        val uri = java.net.URI(value.trim())
        uri.scheme?.lowercase(java.util.Locale.ROOT) in setOf("https", "http") &&
            !uri.host.isNullOrBlank() && uri.userInfo == null
    }.getOrDefault(false) -> "请输入完整的 http 或 https 地址，或留空。"
    kind == "tags" && (structuredTags(value).size > 64 || structuredTags(value).any { it.length > 80 }) ->
        "最多 64 个标签，每个标签不超过 80 个字符。"
    else -> null
}

internal fun structuredDateValid(raw: String): Boolean = runCatching {
    if (raw.isBlank()) return@runCatching true
    val parts = raw.trim().split('/')
    if (parts.size !in 1..2) return@runCatching false
    val first = LocalDate.parse(parts[0].trim())
    parts.size == 1 || !LocalDate.parse(parts[1].trim()).isBefore(first)
}.getOrDefault(false)

internal fun structuredTags(raw: String): List<String> = raw.split(',', '，', '\n')
    .map(String::trim).filter(String::isNotEmpty).distinct()

internal fun structuredEditAllowed(
    document: Boolean, deleted: Boolean, encrypted: Boolean, locked: Boolean,
    richText: Boolean, supportedVersion: Boolean
): Boolean = document && !deleted && !encrypted && !locked && !richText && supportedVersion

internal fun structuredPatchCurrent(
    actualId: String, actualVersion: Long, actualValue: String?, patch: StructuredNotePatch
): Boolean = actualId == patch.noteId && actualVersion == patch.expectedUpdatedAt &&
    actualValue != null && actualValue == patch.expectedValue

internal data class StructuredOutlineNode(
    val id: String, val parentId: String = "", val kind: String = "paragraph", val title: String = ""
)

/** Cycles/dangling parents are displayed, never allowed to hang or hide the page. */
internal fun structuredAncestors(nodes: List<StructuredOutlineNode>, id: String): List<String> {
    val byId = nodes.associateBy { it.id }
    return structuredAncestors(byId, id)
}

private fun structuredAncestors(byId: Map<String, StructuredOutlineNode>, id: String): List<String> {
    val seen = hashSetOf(id)
    val result = ArrayList<String>()
    var parent = byId[id]?.parentId.orEmpty()
    while (parent.isNotBlank()) {
        if (!seen.add(parent) || result.size >= 64) return emptyList()
        val node = byId[parent] ?: break
        result += node.id
        parent = node.parentId
    }
    return result
}

internal fun structuredNodeDepths(nodes: List<StructuredOutlineNode>): Map<String, Int> {
    val byId = nodes.associateBy { it.id }
    return nodes.associate { it.id to structuredAncestors(byId, it.id).size.coerceAtMost(8) }
}

internal fun structuredVisibleNodes(
    nodes: List<StructuredOutlineNode>, collapsed: Set<String>
): List<StructuredOutlineNode> {
    val byId = nodes.associateBy { it.id }
    return nodes.filter { node ->
        structuredAncestors(byId, node.id).none { id -> id in collapsed && byId[id]?.kind == "toggle" }
    }
}

internal fun structuredExpandTo(
    nodes: List<StructuredOutlineNode>, targetId: String, collapsed: Set<String>
): Set<String> = collapsed - structuredAncestors(nodes, targetId).toSet()

/** One in-flight command; invalidation never turns a late callback into success. */
internal class StructuredSaveGate {
    private var next = 0L
    private var active: Long? = null
    private var closed = false
    @Synchronized fun begin(): Long? {
        if (closed || active != null) return null
        return (++next).also { active = it }
    }
    @Synchronized fun finish(ticket: Long, currentWorkspace: Boolean): Boolean {
        if (active != ticket) return false
        active = null
        return !closed && currentWorkspace
    }
    @Synchronized fun close() { closed = true }
}
"####;

pub const EDIT_CONTENTS: &str = r####"package com.ofairyo.gridtimer.data

import kotlinx.serialization.json.*
import java.util.UUID

internal fun JsonObject?.structuredText(key: String): String =
    (this?.get(key) as? JsonPrimitive)?.contentOrNull.orEmpty()
internal fun JsonObject?.structuredFlag(key: String): Boolean = structuredText(key) == "true"
private fun JsonObject?.supportedStructuredVersion(): Boolean =
    this == null || this["version"] == null || structuredText("version") == "1"

internal fun NoteEntry.canEditStructured(): Boolean = structuredEditAllowed(
    document = kind == NoteEntryKind.DOCUMENT,
    deleted = deletedAtEpochMillis != null,
    encrypted = encryption != null,
    locked = document.knowledge.structuredFlag("locked"),
    richText = document.richTextEnabled,
    supportedVersion = document.knowledge.supportedStructuredVersion()
)

internal fun structuredTargetToken(note: NoteEntry, patch: StructuredNotePatch): String? {
    val block = note.document.blocks.singleOrNull { it.id == patch.blockId }
    return when (patch.action) {
        "title" -> note.title
        "text" -> block?.takeIf { it.type == NoteBlockType.TEXT &&
            it.knowledge.supportedStructuredVersion() &&
            it.knowledge.structuredText("kind").ifBlank { "paragraph" } in structuredTextKinds }?.text
        "checked" -> block?.takeIf { it.knowledge.structuredText("kind") == "todo" &&
            it.knowledge.supportedStructuredVersion() }?.let { it.knowledge.structuredFlag("checked").toString() }
        "cell" -> ((block?.knowledge?.get("table") as? JsonArray)?.getOrNull(patch.row) as? JsonArray)
            ?.getOrNull(patch.column)?.let { (it as? JsonPrimitive)?.contentOrNull }
        "table_row", "table_column" -> (block?.knowledge?.get("table") as? JsonArray)?.toString()
        "property" -> (note.document.knowledge?.get("properties") as? JsonObject)?.get(patch.key)?.toString()
        "tags" -> (note.document.knowledge?.get("tags") as? JsonArray ?: JsonArray(emptyList())).toString()
        "append" -> if (patch.blockId.isBlank() || note.document.blocks.any { it.id == patch.blockId }) null
            else if (patch.parentId.isNotBlank() && note.document.blocks.singleOrNull { it.id == patch.parentId }
                ?.knowledge.structuredText("kind") != "toggle") null
            else note.document.blocks.size.toString()
        else -> null
    }
}

/** Runs inside the existing repository write transaction, against its current note. */
internal fun applyStructuredPatch(note: NoteEntry, patch: StructuredNotePatch): NoteEntry? {
    if (!note.canEditStructured() || !structuredPatchCurrent(note.id, note.updatedAtEpochMillis,
            structuredTargetToken(note, patch), patch)) return null
    if (note.document.blocks.map { it.id }.distinct().size != note.document.blocks.size) return null
    val oldDocument = note.document
    var document = oldDocument
    var title = note.title
    fun updateBlock(operation: (NoteBlock) -> NoteBlock) {
        document = document.copy(blocks = document.blocks.map { if (it.id == patch.blockId) operation(it) else it })
    }
    when (patch.action) {
        "title" -> {
            if (structuredInputError("title", patch.value) != null) return null
            title = patch.value
        }
        "text" -> {
            if (structuredInputError("text", patch.value) != null) return null
            updateBlock { it.copy(text = patch.value) }
        }
        "checked" -> {
            if (structuredInputError("checkbox", patch.value) != null) return null
            updateBlock { it.copy(knowledge = JsonObject(it.knowledge.orEmpty() + ("checked" to JsonPrimitive(patch.value == "true")))) }
        }
        "cell", "table_row", "table_column" -> {
            val block = document.blocks.single { it.id == patch.blockId }
            if (block.knowledge.structuredText("kind") != "table" || !block.knowledge.supportedStructuredVersion()) return null
            val table = block.knowledge?.get("table") as? JsonArray ?: return null
            val rows = table.map { row -> (row as? JsonArray)?.toList() ?: return null }.toMutableList()
            if (rows.isEmpty() || rows.any { row -> row.any { it !is JsonPrimitive || !it.isString } }) return null
            when (patch.action) {
                "cell" -> {
                    if (structuredInputError("text", patch.value) != null) return null
                    val row = rows.getOrNull(patch.row)?.toMutableList() ?: return null
                    if (patch.column !in row.indices) return null
                    row[patch.column] = JsonPrimitive(patch.value); rows[patch.row] = row
                }
                "table_row" -> {
                    if (rows.size >= 200 || rows.maxOf { it.size } !in 1..20) return null
                    rows += List(rows.maxOf { it.size }) { JsonPrimitive("") }
                }
                "table_column" -> {
                    if (rows.maxOf { it.size } >= 20 || rows.size > 200) return null
                    val width = rows.maxOf { it.size } + 1
                    rows.indices.forEach { index -> rows[index] = rows[index] + List(width - rows[index].size) { JsonPrimitive("") } }
                }
            }
            updateBlock { it.copy(knowledge = JsonObject(it.knowledge.orEmpty() + ("table" to JsonArray(rows.map(::JsonArray))))) }
        }
        "property" -> {
            val page = document.knowledge ?: return null
            val properties = page["properties"] as? JsonObject ?: return null
            val cell = properties[patch.key] as? JsonObject ?: return null
            val kind = cell.structuredText("kind")
            if (kind !in structuredPropertyKinds || structuredInputError(kind, patch.value) != null) return null
            // Database schema/options/formulas need their own editor, never guess their write contract.
            if (page["database"] != null && page["database"] != JsonNull) return null
            val value: JsonElement = when (kind) {
                "number" -> JsonPrimitive(patch.value.trim().toDouble())
                "checkbox" -> JsonPrimitive(patch.value == "true")
                "date" -> {
                    val parts = patch.value.trim().split('/')
                    val prior = cell["value"] as? JsonObject
                    JsonObject(prior.orEmpty() + mapOf("start" to JsonPrimitive(parts[0].trim()),
                        "end" to JsonPrimitive(parts.getOrNull(1)?.trim().orEmpty())))
                }
                else -> JsonPrimitive(patch.value)
            }
            val nextCell = JsonObject(cell + ("value" to value))
            document = document.copy(knowledge = JsonObject(page + ("properties" to JsonObject(properties + (patch.key to nextCell)))))
        }
        "tags" -> {
            if (structuredInputError("tags", patch.value) != null) return null
            document = document.copy(knowledge = JsonObject(document.knowledge.orEmpty() +
                ("tags" to JsonArray(structuredTags(patch.value).map(::JsonPrimitive)))))
        }
        "append" -> {
            if (patch.key !in structuredAppendKinds || structuredInputError("text", patch.value) != null) return null
            val meta = buildJsonObject {
                put("version", 1); put("kind", patch.key)
                if (patch.parentId.isNotBlank()) put("parentId", patch.parentId)
                if (patch.key == "todo") put("checked", false)
                if (patch.key == "toggle") put("collapsed", false)
                if (patch.key == "table") {
                    put("tableHeader", true)
                    put("table", JsonArray(List(2) { JsonArray(List(2) { JsonPrimitive("") }) }))
                }
            }
            val added = NoteBlock(id = patch.blockId, type = NoteBlockType.TEXT, text = patch.value, knowledge = meta)
            val nextBlocks = document.blocks.toMutableList()
            val position = if (patch.parentId.isBlank()) nextBlocks.size
                else nextBlocks.indexOfFirst { it.id == patch.parentId } + 1
            nextBlocks.add(position, added)
            document = document.copy(blocks = nextBlocks)
        }
        else -> return null
    }
    return note.copy(title = title, document = document, content = document.toStorageContent())
}

internal fun structuredPropertyEditorValue(cell: JsonObject): String = when (cell.structuredText("kind")) {
    "date" -> (cell["value"] as? JsonObject)?.let { date ->
        listOf(date.structuredText("start"), date.structuredText("end")).filter(String::isNotBlank).joinToString(" / ")
    }.orEmpty()
    else -> (cell["value"] as? JsonPrimitive)?.contentOrNull.orEmpty()
}

internal fun buildStructuredMobileNote(folderId: String?, now: Long): NoteEntry {
    val toggleId = UUID.randomUUID().toString()
    fun block(kind: String, text: String, id: String = UUID.randomUUID().toString(), parent: String? = null) = NoteBlock(
        id = id, type = NoteBlockType.TEXT, text = text, knowledge = buildJsonObject {
            put("version", 1); put("kind", kind)
            if (parent != null) put("parentId", parent)
            if (kind == "todo") put("checked", false)
            if (kind == "toggle") put("collapsed", false)
            if (kind == "table") { put("tableHeader", true); put("table", JsonArray(listOf(
                JsonArray(listOf(JsonPrimitive("项目"), JsonPrimitive("说明"))),
                JsonArray(listOf(JsonPrimitive(""), JsonPrimitive("")))
            ))) }
        }
    )
    val document = NoteDocument(knowledge = buildJsonObject {
        put("version", 1)
        put("tags", JsonArray(emptyList()))
        put("properties", buildJsonObject {
            put("status", buildJsonObject { put("kind", "text"); put("value", "未开始") })
            put("date", buildJsonObject { put("kind", "date"); put("value", buildJsonObject { put("start", ""); put("end", "") }) })
        })
    }, blocks = listOf(block("heading1", "项目概览"), block("paragraph", ""),
        block("todo", "待办事项"), block("toggle", "资料与笔记", toggleId),
        block("paragraph", "", parent = toggleId), block("table", "")))
    return NoteEntry(title = "结构页", kind = NoteEntryKind.DOCUMENT, folderId = folderId,
        document = document, content = document.toStorageContent(), accentSeed = "amber",
        createdAtEpochMillis = now, updatedAtEpochMillis = now)
}

internal fun structuredPatchVerified(note: NoteEntry, patch: StructuredNotePatch, expected: String?): Boolean {
    if (patch.action != "append") return expected != null && structuredTargetToken(note, patch) == expected
    val added = note.document.blocks.singleOrNull { it.id == patch.blockId } ?: return false
    return added.text == patch.value && added.knowledge.structuredText("kind") == patch.key &&
        added.knowledge.structuredText("parentId") == patch.parentId &&
        (patch.key != "table" || (added.knowledge?.get("table") as? JsonArray)?.size == 2)
}
"####;
