//! Android knowledge navigation, lexical links and backlinks. No data migration.
//! Sources, UI and regression tests are emitted from Rust, not patched Kotlin outputs.
pub const SCREEN_PATH: &str = "com/ofairyo/gridtimer/ui/NoteStudioSheet.kt";
pub const INDEX_PATH: &str = "com/ofairyo/gridtimer/ui/KnowledgeNavigationIndex.kt";
pub const UI_PATH: &str = "com/ofairyo/gridtimer/ui/KnowledgeNavigation.kt";
pub const TEST_PATH: &str = "com/ofairyo/gridtimer/ui/KnowledgeNavigationIndexTest.kt";
pub const SNAPSHOT_TEST_PATH: &str = "com/ofairyo/gridtimer/ui/KnowledgeNavigationSnapshotTest.kt";

fn replace_once(source: &mut String, before: &str, after: &str) -> Result<(), String> {
    let count = source.matches(before).count();
    if count != 1 {
        return Err(format!("knowledge navigation anchor count {count}: {before}"));
    }
    *source = source.replacen(before, after, 1);
    Ok(())
}

pub fn render(path: &str, source: &str) -> Result<String, String> {
    if path != SCREEN_PATH {
        return Ok(source.to_owned());
    }
    let mut result = source.to_owned();
    for name in ["androidx.compose.foundation.layout.heightIn", "androidx.compose.ui.platform.testTag"] {
        let import = format!("import {name}\n");
        if !result.contains(&import) {
            replace_once(&mut result, "package com.ofairyo.gridtimer.ui\n",
                &format!("package com.ofairyo.gridtimer.ui\n{import}"))?;
        }
    }
    replace_once(&mut result, STATE_ANCHOR, STATE)?;
    replace_once(&mut result, ENTRY_ANCHOR, ENTRY)?;
    Ok(result)
}

const STATE_ANCHOR: &str = "    val notebookDocuments = remember(appData.notes) { appData.activeNotebookDocuments() }";
const STATE: &str = r####"    val navigationWorkspaceKey = LocalNoteMediaWorkspaceKey.current
    var knowledgeNavigationVisible by remember(navigationWorkspaceKey) { mutableStateOf(false) }
    if (knowledgeNavigationVisible && !trashMode) {
        KnowledgeNavigationDialog(
            appData = appData,
            workspaceKey = navigationWorkspaceKey,
            onDismiss = { knowledgeNavigationVisible = false },
            onOpenNote = { note ->
                knowledgeNavigationVisible = false
                onOpenNote(note)
            }
        )
    }
    val notebookDocuments = remember(appData.notes) { appData.activeNotebookDocuments() }"####;
const ENTRY_ANCHOR: &str = r####"        if (!trashMode) {
            item {
                KnowledgeAskEntryCard("####;
const ENTRY: &str = r####"        if (!trashMode) {
            item(key = "knowledge-navigation-entry") {
                androidx.compose.material3.OutlinedButton(
                    onClick = { knowledgeNavigationVisible = true },
                    modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)
                        .testTag("knowledge_open_navigation")
                ) { Text("关联与检索") }
            }
            item {
                KnowledgeAskEntryCard("####;

pub const INDEX_CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui

import java.net.URLDecoder
import java.text.Normalizer
import java.util.Locale

/** Ephemeral navigation data. No file writes, model requests or decrypted-note storage. */
internal data class KnowledgeNavPage(
    val id: String,
    val title: String,
    val folder: String = "",
    val text: String = "",
    val metadataTags: List<String> = emptyList(),
    val bodyAvailable: Boolean = true,
    val updatedAt: Long = 0L
)
internal data class KnowledgeNavHeading(val level: Int, val title: String, val line: Int)
internal data class KnowledgeNavLink(
    val target: String,
    val anchor: String,
    val label: String,
    val embedded: Boolean,
    val line: Int,
    val candidates: List<String>
)
internal data class KnowledgeNavIndexedPage(
    val page: KnowledgeNavPage,
    val links: List<KnowledgeNavLink>,
    val tags: Set<String>,
    val headings: List<KnowledgeNavHeading>,
    val searchable: String,
    val mentionText: String
)
internal enum class KnowledgeNavFilter { ALL, UNCONNECTED, UNRESOLVED }
internal class KnowledgeNavIndex(
    val workspaceKey: String,
    val pages: Map<String, KnowledgeNavIndexedPage>,
    val incoming: Map<String, Set<String>>,
    val tagCounts: Map<String, Int>,
    val bodyOmissions: Int
) {
    val resolvedLinks: Int = pages.values.sumOf { it.links.count { link -> link.candidates.size == 1 } }
    val unresolvedLinks: Int = pages.values.sumOf { it.links.count { link -> link.candidates.size != 1 } }

    fun search(query: String, tag: String? = null, filter: KnowledgeNavFilter = KnowledgeNavFilter.ALL, checkCancelled: () -> Unit = {}): List<KnowledgeNavIndexedPage> {
        val terms = query.trim().split(Regex("\\s+")).filter(String::isNotEmpty).map(::knowledgeNavKey)
        val selectedTag = tag?.removePrefix("#")?.let(::knowledgeNavKey)
        return pages.values.filter { item ->
            checkCancelled()
            terms.all { it in item.searchable } &&
                (selectedTag == null || item.tags.any { it == selectedTag || it.startsWith("$selectedTag/") }) &&
                when (filter) {
                    KnowledgeNavFilter.ALL -> true
                    KnowledgeNavFilter.UNCONNECTED -> item.page.bodyAvailable &&
                        incoming[item.page.id].isNullOrEmpty() && item.links.isEmpty()
                    KnowledgeNavFilter.UNRESOLVED -> item.links.any { it.candidates.size != 1 }
                }
        }.sortedWith(compareBy<KnowledgeNavIndexedPage> {
            val title = knowledgeNavKey(it.page.title)
            when { terms.isNotEmpty() && title == terms.joinToString(" ") -> 0
                terms.isNotEmpty() && title.startsWith(terms.first()) -> 1
                else -> 2 }
        }.thenByDescending { it.page.updatedAt }.thenBy { knowledgeNavKey(it.page.title) }.thenBy { it.page.id })
    }

    /** On demand, not an all-pairs pass. Linked spans and code are already masked. */
    fun mentionsOf(id: String, checkCancelled: () -> Unit = {}): List<KnowledgeNavIndexedPage> {
        val title = pages[id]?.page?.title?.trim()?.takeIf { it.length >= 2 } ?: return emptyList()
        val needle = knowledgeNavKey(title)
        return pages.values.filter { item ->
            checkCancelled()
            item.page.id != id && item.page.bodyAvailable &&
                item.links.none { id in it.candidates } && hasMention(item.mentionText, needle)
        }.sortedBy { knowledgeNavKey(it.page.title) }
    }

    /** Avoid producing a link which would resolve to a different or ambiguous document. */
    fun wikiLink(id: String): String? {
        val item = pages[id]?.page ?: return null
        if (item.title.isBlank() || item.title.any { it in "[]|#^/\\:" || it.isISOControl() }) return null
        if (item.folder.any { it in "[]|#^\\:" || it.isISOControl() }) return null
        val target = listOf(item.folder.trim('/'), item.title).filter(String::isNotBlank).joinToString("/")
        val candidates = pages.values.filter {
            knowledgeNavTargetKey(listOf(it.page.folder.trim('/'), it.page.title).filter(String::isNotBlank).joinToString("/")) == knowledgeNavTargetKey(target)
        }
        if (candidates.size != 1) return null
        // An unqualified name could also match a same-title note in another folder.
        if (item.folder.isBlank() && pages.values.count { knowledgeNavTargetKey(it.page.title) == knowledgeNavTargetKey(item.title) } != 1) return null
        return "[[$target]]"
    }
}

/** Compose effect keys must notice equal-content replacement snapshots as well. */
internal class KnowledgeNavSourceKey(private val notes: Any, private val folders: Any) {
    override fun equals(other: Any?): Boolean = other is KnowledgeNavSourceKey && notes === other.notes && folders === other.folders
    override fun hashCode(): Int = 31 * System.identityHashCode(notes) + System.identityHashCode(folders)
}

internal fun knowledgeNavMayIndex(isDocument: Boolean, deleted: Boolean, encrypted: Boolean): Boolean =
    isDocument && !deleted && !encrypted

internal fun knowledgeNavKey(value: String): String =
    Normalizer.normalize(value.trim(), Normalizer.Form.NFC).lowercase(Locale.ROOT)

private fun knowledgeNavTargetKey(value: String): String {
    val trimmed = value.trim()
    return knowledgeNavKey(if (trimmed.endsWith(".md", true)) trimmed.dropLast(3) else trimmed)
}

private fun hasMention(text: String, needle: String): Boolean {
    var start = 0
    while (start <= text.length - needle.length) {
        val at = text.indexOf(needle, start)
        if (at < 0) return false
        fun latinWord(ch: Char?) = ch != null && (ch in 'a'..'z' || ch in '0'..'9' || ch == '_')
        if (!(latinWord(needle.first()) && latinWord(text.getOrNull(at - 1))) &&
            !(latinWord(needle.last()) && latinWord(text.getOrNull(at + needle.length)))) return true
        start = at + 1
    }
    return false
}

private data class KnowledgeNavSyntax(
    val links: List<Triple<IntRange, String, Boolean>>,
    val tags: Set<String>, val headings: List<KnowledgeNavHeading>,
    val visible: String, val mentions: String
)

/** Bounded lexical scans; source is never modified. This is not a full Markdown parser. */
private fun knowledgeNavSyntax(text: String, checkCancelled: () -> Unit): KnowledgeNavSyntax {
    val visible = text.toCharArray()
    fun hide(from: Int, until: Int) {
        for (i in from until until.coerceAtMost(visible.size)) if (visible[i] != '\n' && visible[i] != '\r') visible[i] = ' '
    }
    var fenceChar = '\u0000'
    var fenceLength = 0
    var htmlComment = false
    var obsidianComment = false
    var offset = 0
    text.splitToSequence('\n').forEach { line ->
        checkCancelled()
        // Strip only scanning prefixes for quote/list fenced code, never source data.
        val fenceView = line.trimStart().replace(Regex("^(?:>\\s*)+"), "")
            .replaceFirst(Regex("^ {0,3}(?:[-+*]|[0-9]{1,9}[.)])[ \\t]+"), "")
        val marker = Regex("^ {0,3}(`{3,}|~{3,})(.*)$").find(fenceView)
        if (fenceChar != '\u0000') {
            hide(offset, offset + line.length)
            if (marker != null && marker.groupValues[1].first() == fenceChar &&
                marker.groupValues[1].length >= fenceLength && marker.groupValues[2].isBlank()) fenceChar = '\u0000'
        } else if (!htmlComment && !obsidianComment && marker != null && (marker.groupValues[1].first() != '`' || !marker.groupValues[2].contains('`'))) {
            fenceChar = marker.groupValues[1].first(); fenceLength = marker.groupValues[1].length
            hide(offset, offset + line.length)
        } else if (line.startsWith("    ") || line.startsWith("\t")) {
            hide(offset, offset + line.length)
        } else {
            var i = 0
            while (i < line.length) {
                if (i % 4096 == 0) checkCancelled()
                when {
                    htmlComment -> {
                        val end = line.indexOf("-->", i)
                        hide(offset + i, offset + if (end < 0) line.length else end + 3)
                        if (end < 0) break
                        htmlComment = false; i = end + 3
                    }
                    obsidianComment -> {
                        val end = line.indexOf("%%", i)
                        hide(offset + i, offset + if (end < 0) line.length else end + 2)
                        if (end < 0) break
                        obsidianComment = false; i = end + 2
                    }
                    line.startsWith("<!--", i) -> { htmlComment = true; hide(offset + i, offset + i + 4); i += 4 }
                    line.startsWith("%%", i) -> { obsidianComment = true; hide(offset + i, offset + i + 2); i += 2 }
                    line[i] == '\\' -> { hide(offset + i, offset + i + 2); i += 2 }
                    line[i] == '`' -> {
                        var count = 1
                        while (line.getOrNull(i + count) == '`') count++
                        val markerText = "`".repeat(count)
                        var end = line.indexOf(markerText, i + count)
                        while (end >= 0 && (line.getOrNull(end - 1) == '`' || line.getOrNull(end + count) == '`')) end = line.indexOf(markerText, end + count)
                        if (end >= 0) { hide(offset + i, offset + end + count); i = end + count } else i += count
                    }
                    else -> i++
                }
            }
        }
        offset += line.length + 1
    }
    val afterBlocks = String(visible)
    val ticks = Regex("`+").findAll(afterBlocks).toList()
    val nextByLength = mutableMapOf<Int, Int>()
    val nextSame = IntArray(ticks.size) { -1 }
    for (i in ticks.indices.reversed()) {
        nextSame[i] = nextByLength.put(ticks[i].value.length, i) ?: -1
    }
    var tick = 0
    while (tick < ticks.size) {
        checkCancelled()
        val close = nextSame[tick]
        if (close >= 0 && !afterBlocks.substring(ticks[tick].range.last + 1, ticks[close].range.first).contains("\n\n")) {
            hide(ticks[tick].range.first, ticks[close].range.last + 1)
            tick = close + 1
        } else tick++
    }
    val clean = String(visible)
    val mentions = visible.copyOf()
    val links = mutableListOf<Triple<IntRange, String, Boolean>>()
    // Non-greedy bounded matches cannot span lines or nested square brackets.
    Regex("(!?)\\[\\[([^\\[\\]\\r\\n]{1,2048})]]").findAll(clean).forEach { match ->
        links += Triple(match.range, match.groupValues[2], match.groupValues[1] == "!")
        for (i in match.range) mentions[i] = ' '
    }
    Regex("(!?)\\[([^\\[\\]\\r\\n]{0,512})]\\(([^\\s()]{1,2048})\\)").findAll(String(mentions)).forEach { match ->
        run {
            val raw = match.groupValues[3]
            val destination = runCatching { URLDecoder.decode(raw.replace("+", "%2B"), "UTF-8") }.getOrNull()
            if (destination != null && !destination.contains(Regex("^[A-Za-z][A-Za-z0-9+.-]*:")) &&
                !destination.startsWith("//") && (destination.substringBefore('#').endsWith(".md", true) || destination.startsWith('#'))) {
                links += Triple(match.range, "$destination|${match.groupValues[2]}", match.groupValues[1] == "!")
            }
            // External URLs/labels are not automatic unlinked mentions either.
            for (i in match.range) mentions[i] = ' '
        }
    }
    val tags = linkedSetOf<String>()
    Regex("(?<![\\p{L}\\p{N}_/#\\\\])#([\\p{L}\\p{N}_-]+(?:/[\\p{L}\\p{N}_-]+)*)").findAll(String(mentions)).forEach { match ->
        val value = match.groupValues[1]
        if (value.any { it.isLetter() || it == '_' || it == '-' }) tags += knowledgeNavKey(value)
    }
    val headings = mutableListOf<KnowledgeNavHeading>()
    clean.lineSequence().forEachIndexed { index, line ->
        Regex("^ {0,3}(#{1,6})[ \\t]+(.+?)\\s*#*\\s*$").matchEntire(line)?.let {
            headings += KnowledgeNavHeading(it.groupValues[1].length, it.groupValues[2].trim(), index + 1)
        }
    }
    return KnowledgeNavSyntax(links.sortedBy { it.first.first }, tags, headings, clean, String(mentions))
}

internal fun buildKnowledgeNavIndex(
    workspaceKey: String,
    input: List<KnowledgeNavPage>,
    checkCancelled: () -> Unit = {}
): KnowledgeNavIndex {
    require(workspaceKey.isNotBlank()) { "Missing knowledge workspace" }
    require(input.all { it.id.isNotBlank() } && input.map { it.id }.distinct().size == input.size) { "Duplicate or blank note identity" }
    val targets = mutableMapOf<String, MutableSet<String>>()
    input.forEach { page ->
        checkCancelled()
        if (page.title.isNotBlank()) {
            targets.getOrPut(knowledgeNavTargetKey(page.title)) { linkedSetOf() }.add(page.id)
            val path = listOf(page.folder.trim('/'), page.title).filter(String::isNotBlank).joinToString("/")
            targets.getOrPut(knowledgeNavTargetKey(path)) { linkedSetOf() }.add(page.id)
        }
    }
    val incoming = input.associate { it.id to linkedSetOf<String>() }
    val pages = linkedMapOf<String, KnowledgeNavIndexedPage>()
    val tagCounts = sortedMapOf<String, Int>()
    input.forEach { page ->
        checkCancelled()
        val syntax = knowledgeNavSyntax(if (page.bodyAvailable) page.text else "", checkCancelled)
        var lineCursor = 0
        var currentLinkLine = 1
        val links = syntax.links.map { (range, raw, embedded) ->
            while (lineCursor < range.first) {
                if (lineCursor % 4096 == 0) checkCancelled()
                if (syntax.visible[lineCursor++] == '\n') currentLinkLine++
            }
            var target = raw.substringBefore('|').substringBefore('#').trim()
            val anchor = raw.substringBefore('|').substringAfter('#', "").trim()
            if (target.endsWith(".md", true)) target = target.dropLast(3)
            val safe = !target.startsWith('/') && !target.contains('\\') &&
                target.split('/').none { it == ".." || it == "." } &&
                target.none { it.isISOControl() || it == ':' }
            val candidates = when {
                target.isBlank() && anchor.isNotBlank() -> listOf(page.id)
                !safe || target.isBlank() -> emptyList()
                else -> targets[knowledgeNavKey(target)]?.sorted().orEmpty()
            }
            if (candidates.size == 1 && candidates.single() != page.id) incoming.getValue(candidates.single()).add(page.id)
            KnowledgeNavLink(target, anchor, raw.substringAfter('|', raw.substringBefore('|')).trim(), embedded,
                currentLinkLine, candidates)
        }
        val tags = syntax.tags + page.metadataTags.map { knowledgeNavKey(it.removePrefix("#")) }.filter(String::isNotBlank)
        tags.forEach { tag -> tagCounts[tag] = (tagCounts[tag] ?: 0) + 1 }
        pages[page.id] = KnowledgeNavIndexedPage(page, links, tags, syntax.headings,
            knowledgeNavKey("${page.title}\n${page.folder}\n${syntax.visible}"), knowledgeNavKey(syntax.mentions))
    }
    return KnowledgeNavIndex(workspaceKey, pages, incoming.mapValues { it.value.toSet() }, tagCounts, input.count { !it.bodyAvailable })
}
"####;

pub const UI_CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.widget.Toast
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.SecureFlagPolicy
import com.ofairyo.gridtimer.data.*
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.*

private const val KNOWLEDGE_NAV_PAGE_SIZE = 40
private const val KNOWLEDGE_NAV_NOTE_CHAR_LIMIT = 512_000
private const val KNOWLEDGE_NAV_TOTAL_CHAR_LIMIT = 8_000_000

internal data class KnowledgeNavSnapshot(
    val workspaceKey: String,
    val sourceNotes: List<NoteEntry>,
    val sourceFolders: List<NoteFolder>,
    val index: KnowledgeNavIndex,
    val excludedEncrypted: Int
)

/** Exclude ciphertext AND temporary plaintext envelopes before accessing any note content. */
internal fun buildKnowledgeNavSnapshot(
    workspaceKey: String,
    notes: List<NoteEntry>,
    folders: List<NoteFolder>,
    checkCancelled: () -> Unit = {}
): KnowledgeNavSnapshot {
    val folderNames = folders.associate { it.id to it.name }
    var budget = KNOWLEDGE_NAV_TOTAL_CHAR_LIMIT
    var encrypted = 0
    val pages = mutableListOf<KnowledgeNavPage>()
    for (note in notes) {
        checkCancelled()
        if (note.kind == NoteEntryKind.DOCUMENT && !note.isDeleted() && note.encryption != null) encrypted++
        if (!knowledgeNavMayIndex(note.kind == NoteEntryKind.DOCUMENT, note.isDeleted(), note.encryption != null)) continue
        val document = note.resolvedDocument()
        // Complex imported pages stay navigable via the existing safe reader.
        // Never interpret rich HTML, structured code cells or truncated text as links.
        val simple = !document.richTextEnabled && !note.hasStructuredKnowledge()
        val textSize = document.blocks.filter { it.type == NoteBlockType.TEXT }
            .sumOf { it.text.length.toLong() + 1L }
        val available = simple && textSize <= KNOWLEDGE_NAV_NOTE_CHAR_LIMIT && textSize <= budget
        val text = if (available) document.blocks.filter { it.type == NoteBlockType.TEXT }
            .joinToString("\n") { it.text } else ""
        budget -= text.length
        val tags = (document.knowledge?.get("tags") as? JsonArray)?.mapNotNull {
            (it as? JsonPrimitive)?.contentOrNull
        }.orEmpty()
        pages += KnowledgeNavPage(note.id, note.displayTitle(), folderNames[note.folderId].orEmpty(),
            text, tags, available, note.updatedAtEpochMillis)
    }
    return KnowledgeNavSnapshot(workspaceKey, notes, folders,
        buildKnowledgeNavIndex(workspaceKey, pages, checkCancelled), encrypted)
}

private data class KnowledgeNavQueryResult(
    val index: KnowledgeNavIndex, val query: String, val tag: String?,
    val filter: KnowledgeNavFilter, val pages: List<KnowledgeNavIndexedPage>
)

@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun KnowledgeNavigationDialog(
    appData: AppData,
    workspaceKey: String,
    onDismiss: () -> Unit,
    onOpenNote: (NoteEntry) -> Unit
) {
    val context = LocalContext.current
    val latestData by rememberUpdatedState(appData)
    val latestWorkspace by rememberUpdatedState(LocalNoteMediaWorkspaceKey.current)
    val latestOpen by rememberUpdatedState(onOpenNote)
    var snapshot by remember(workspaceKey) { mutableStateOf<KnowledgeNavSnapshot?>(null) }
    var failure by remember(workspaceKey) { mutableStateOf(false) }
    var retry by remember(workspaceKey) { mutableStateOf(0) }
    var query by remember(workspaceKey) { mutableStateOf("") }
    var selectedTag by remember(workspaceKey) { mutableStateOf<String?>(null) }
    var filter by remember(workspaceKey) { mutableStateOf(KnowledgeNavFilter.ALL) }
    var selectedId by remember(workspaceKey) { mutableStateOf<String?>(null) }
    var showTags by remember(workspaceKey) { mutableStateOf(false) }
    var notice by remember(workspaceKey) { mutableStateOf("") }
    var visibleCount by remember(workspaceKey, selectedId, query, selectedTag, filter, showTags) { mutableStateOf(KNOWLEDGE_NAV_PAGE_SIZE) }

    val sourceKey = KnowledgeNavSourceKey(appData.notes, appData.noteFolders)
    LaunchedEffect(workspaceKey, sourceKey, retry) {
        snapshot = null
        failure = false
        try {
            val built = withContext(Dispatchers.Default) {
                val worker = currentCoroutineContext()
                buildKnowledgeNavSnapshot(workspaceKey, appData.notes, appData.noteFolders) { worker.ensureActive() }
            }
            if (latestWorkspace == workspaceKey && latestData.notes === built.sourceNotes &&
                latestData.noteFolders === built.sourceFolders) snapshot = built
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { failure = true }
    }
    // Reject stale snapshots during the composition BEFORE a replacement worker runs.
    val current = snapshot?.takeIf { latestWorkspace == workspaceKey &&
        it.sourceNotes === appData.notes && it.sourceFolders === appData.noteFolders }
    val index = current?.index
    val selected = selectedId?.let { index?.pages?.get(it) }
    var queryResult by remember(workspaceKey) { mutableStateOf<KnowledgeNavQueryResult?>(null) }
    var mentions by remember(index, selectedId) { mutableStateOf<List<KnowledgeNavIndexedPage>?>(null) }
    LaunchedEffect(index, query, selectedTag, filter) {
        queryResult = null
        if (index != null) {
            val result = withContext(Dispatchers.Default) {
                val worker = currentCoroutineContext()
                index.search(query, selectedTag, filter) { worker.ensureActive() }
            }
            queryResult = KnowledgeNavQueryResult(index, query, selectedTag, filter, result)
        }
    }
    LaunchedEffect(index, selectedId) {
        val target = selectedId
        if (index != null && target != null) mentions = withContext(Dispatchers.Default) {
            val worker = currentCoroutineContext()
            index.mentionsOf(target) { worker.ensureActive() }
        }
    }
    val results = queryResult?.takeIf { it.index === index && it.query == query &&
        it.tag == selectedTag && it.filter == filter }?.pages

    fun openEditor(id: String) {
        if (latestWorkspace != workspaceKey) { onDismiss(); return }
        val live = latestData.notes.firstOrNull { it.id == id } ?: return
        if (!knowledgeNavMayIndex(live.kind == NoteEntryKind.DOCUMENT, live.isDeleted(), live.encryption != null)) {
            notice = "文档状态已变化，请重新选择。"; return
        }
        onDismiss()
        latestOpen(live) // Existing editor/unlock/save route, not a second editor.
    }
    fun copyLink(id: String) {
        if (latestWorkspace != workspaceKey || current == null ||
            current.sourceNotes !== latestData.notes || current.sourceFolders !== latestData.noteFolders) return
        val text = index?.wikiLink(id)
        if (text == null) { notice = "标题或路径重名、或含保留符号，暂不生成可能指向错误文档的双链。"; return }
        (context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager)
            .setPrimaryClip(ClipData.newPlainText("文档双链", text))
        Toast.makeText(context, "已复制双链，可粘贴到正文", Toast.LENGTH_SHORT).show()
    }
    fun back() { if (selectedId != null) selectedId = null else onDismiss() }

    Dialog(onDismissRequest = ::back, properties = DialogProperties(
        usePlatformDefaultWidth = false, securePolicy = SecureFlagPolicy.SecureOn
    )) {
        BackHandler(onBack = ::back)
        Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
            Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.safeDrawing).testTag("knowledge_navigation")) {
                Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp), horizontalArrangement = Arrangement.SpaceBetween) {
                    TextButton(onClick = ::back) { Text(if (selectedId != null) "返回检索" else "关闭") }
                    Text(if (selectedId != null) "文档关系" else "关联与检索",
                        modifier = Modifier.weight(1f).padding(vertical = 16.dp), maxLines = 1, overflow = TextOverflow.Ellipsis, fontWeight = FontWeight.SemiBold)
                    if (selected != null) TextButton(onClick = { openEditor(selected.page.id) }) { Text("打开文档") }
                }
                if (index == null) {
                    Column(Modifier.padding(20.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                        Text(if (failure) "本机索引暂时无法建立，原文档未修改。" else "正在本机整理文档关系…")
                        if (failure) OutlinedButton(onClick = { retry++ }) { Text("重试") } else LinearProgressIndicator(Modifier.fillMaxWidth())
                    }
                } else {
                    if (selectedId == null) {
                        OutlinedTextField(value = query, onValueChange = { query = it.take(200) },
                            label = { Text("查找标题、正文或资料库") }, singleLine = true,
                            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp).testTag("knowledge_navigation_search"))
                        FlowRow(Modifier.padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                            FilterChip(selected = !showTags && filter == KnowledgeNavFilter.ALL, onClick = { showTags = false; filter = KnowledgeNavFilter.ALL }, label = { Text("全部") })
                            FilterChip(selected = showTags, onClick = { showTags = !showTags }, label = { Text("标签") })
                            FilterChip(selected = !showTags && filter == KnowledgeNavFilter.UNCONNECTED, onClick = { showTags = false; filter = KnowledgeNavFilter.UNCONNECTED }, label = { Text("未见双链") })
                            FilterChip(selected = !showTags && filter == KnowledgeNavFilter.UNRESOLVED, onClick = { showTags = false; filter = KnowledgeNavFilter.UNRESOLVED }, label = { Text("待解析") })
                            if (selectedTag != null) InputChip(selected = true, onClick = { selectedTag = null }, label = { Text("#$selectedTag ×") })
                        }
                    }
                    key(selectedId, query, selectedTag, filter, showTags) {
                        LazyColumn(Modifier.weight(1f).fillMaxWidth(), contentPadding = PaddingValues(16.dp),
                            verticalArrangement = Arrangement.spacedBy(12.dp)) {
                            item(key = "coverage") {
                                Text("${index.pages.size} 篇文档 · ${index.resolvedLinks} 处已解析引用 · ${index.unresolvedLinks} 处待解析",
                                    style = MaterialTheme.typography.bodySmall)
                                Text("仅在本机整理，不向 AI 发送；${current?.excludedEncrypted ?: 0} 篇加密文档未纳入。",
                                    style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                                if (index.bodyOmissions > 0) Text("${index.bodyOmissions} 篇结构化、富文本或超出本次容量的正文未扫描；未见关联不代表没有关联。",
                                    style = MaterialTheme.typography.bodySmall)
                                if (notice.isNotBlank()) Text(notice, color = MaterialTheme.colorScheme.error)
                            }
                            if (selectedId != null && selected == null) item { Text("该文档已删除、加密或不在当前索引中。返回后重新选择。") }
                            else if (selected != null) {
                                item(key = "selected") {
                                    KnowledgeNavPanel {
                                        Text(selected.page.title, style = MaterialTheme.typography.titleLarge)
                                        Text(selected.page.folder.ifBlank { "未入库" }, style = MaterialTheme.typography.bodySmall)
                                        TextButton(onClick = { copyLink(selected.page.id) }) { Text("复制双链") }
                                        Text("双链以标题或资料库/标题解析；重命名后需检查待解析项。", style = MaterialTheme.typography.bodySmall)
                                    }
                                }
                                item(key = "outgoing-title") { Text("本页链接 ${selected.links.size}", fontWeight = FontWeight.SemiBold) }
                                if (selected.links.isEmpty()) item { Text(if (selected.page.bodyAvailable) "尚未识别到文档双链。" else "此文档正文未扫描。") }
                                items(selected.links.take(visibleCount).withIndex().toList(), key = { "out-${it.index}" }) { (_, link) ->
                                    KnowledgeNavPanel {
                                        Text(link.label.ifBlank { link.target }, fontWeight = FontWeight.Medium)
                                        Text("第 ${link.line} 行" + if (link.anchor.isBlank()) "" else " · 定位标记 #${link.anchor}（当前打开所属文档）",
                                            style = MaterialTheme.typography.bodySmall)
                                        if (link.embedded) Text("嵌入引用：本页只展示关联，不展开嵌入内容。", style = MaterialTheme.typography.bodySmall)
                                        when (link.candidates.size) {
                                            0 -> Text("未解析：目标可能未创建、更名或未纳入索引。", color = MaterialTheme.colorScheme.error)
                                            1 -> TextButton(onClick = { selectedId = link.candidates.single() }) { Text("查看目标关系") }
                                            else -> {
                                                Text("有 ${link.candidates.size} 篇同名文档，请明确选择：")
                                                if (link.candidates.size > 12) Text("先显示 12 篇，其余可返回检索；建议用资料库/标题区分。", style = MaterialTheme.typography.bodySmall)
                                                link.candidates.take(12).forEach { id ->
                                                    val candidate = index.pages.getValue(id).page
                                                    TextButton(onClick = { selectedId = id }) { Text("${candidate.folder.ifBlank { "未入库" }} / ${candidate.title}") }
                                                }
                                            }
                                        }
                                    }
                                }
                                val incoming = index.incoming[selected.page.id].orEmpty().mapNotNull(index.pages::get)
                                item(key = "incoming-title") { Text("引用本页 ${incoming.size}", fontWeight = FontWeight.SemiBold) }
                                if (incoming.isEmpty()) item { Text("当前已扫描范围内没有反向链接。") }
                                items(incoming.take(visibleCount), key = { "in-${it.page.id}" }) { page -> KnowledgeNavResult(page, { selectedId = page.page.id }, { openEditor(page.page.id) }) }
                                item(key = "mentions-title") { Text("未建双链的提及 ${mentions?.size ?: "…"}", fontWeight = FontWeight.SemiBold) }
                                if (mentions?.isEmpty() == true) item { Text("未找到独立提及；仅匹配至少两个字符的当前标题。", style = MaterialTheme.typography.bodySmall) }
                                items(mentions.orEmpty().take(visibleCount), key = { "mention-${it.page.id}" }) { page -> KnowledgeNavResult(page, { selectedId = page.page.id }, { openEditor(page.page.id) }) }
                                item(key = "outline-title") { Text("标题大纲 ${selected.headings.size}", fontWeight = FontWeight.SemiBold) }
                                items(selected.headings.take(visibleCount).withIndex().toList(), key = { "heading-${it.index}" }) { (_, heading) ->
                                    Text("${"  ".repeat(heading.level - 1)}${heading.title} · 第 ${heading.line} 行", style = MaterialTheme.typography.bodyMedium)
                                }
                                if (maxOf(selected.links.size, incoming.size, mentions?.size ?: 0, selected.headings.size) > visibleCount) item(key = "more-detail") {
                                    TextButton(onClick = { visibleCount += KNOWLEDGE_NAV_PAGE_SIZE }) { Text("继续显示关联与大纲") }
                                }
                            } else if (showTags) {
                                val tags = index.tagCounts.entries.filter { query.isBlank() || it.key.contains(knowledgeNavKey(query.removePrefix("#"))) }
                                item { Text("${tags.size} 个标签。点击筛选文档；父标签会包含其子标签。", style = MaterialTheme.typography.bodySmall) }
                                items(tags.take(visibleCount), key = { "tag-${it.key}" }) { tag ->
                                    OutlinedButton(onClick = { selectedTag = tag.key; query = ""; showTags = false; filter = KnowledgeNavFilter.ALL }, modifier = Modifier.fillMaxWidth()) { Text("#${tag.key} · ${tag.value} 篇直接标记") }
                                }
                                if (tags.size > visibleCount) item { TextButton(onClick = { visibleCount += KNOWLEDGE_NAV_PAGE_SIZE }) { Text("显示更多标签") } }
                            } else {
                                item {
                                    Text(if (results == null) "正在检索…" else "找到 ${results.size} 篇", style = MaterialTheme.typography.bodySmall)
                                    if (filter == KnowledgeNavFilter.UNCONNECTED) Text("仅指当前已扫描范围内未见双链，不等同于没有其他语义联系。", style = MaterialTheme.typography.bodySmall)
                                }
                                items(results.orEmpty().take(visibleCount), key = { "result-${it.page.id}" }) { page -> KnowledgeNavResult(page, { selectedId = page.page.id }, { openEditor(page.page.id) }) }
                                if (results?.isEmpty() == true) item { Text("没有匹配文档。清空关键词或标签后重试。") }
                                if ((results?.size ?: 0) > visibleCount) item { TextButton(onClick = { visibleCount += KNOWLEDGE_NAV_PAGE_SIZE }) { Text("显示更多文档") } }
                            }
                            item(key = "help") { Text("识别 [[文档]]、[[文档|显示名]]、[[文档#标题]] 与简单的 [文字](文档.md)。\n本次不包含行内自动补全、重命名自动更新、嵌入渲染或图谱画布。", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun KnowledgeNavPanel(content: @Composable ColumnScope.() -> Unit) {
    Surface(Modifier.fillMaxWidth(), shape = MaterialTheme.shapes.medium,
        color = MaterialTheme.colorScheme.surfaceContainerLow) {
        Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(6.dp), content = content)
    }
}

@Composable
private fun KnowledgeNavResult(page: KnowledgeNavIndexedPage, onRelations: () -> Unit, onOpen: () -> Unit) {
    KnowledgeNavPanel {
        Text(page.page.title, fontWeight = FontWeight.Medium)
        Text(page.page.folder.ifBlank { "未入库" }, style = MaterialTheme.typography.bodySmall)
        if (page.page.bodyAvailable) Text(page.page.text.take(240).replace('\n', ' '), maxLines = 2, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodySmall)
        else Text("正文未扫描，仅可按标题查找", style = MaterialTheme.typography.bodySmall)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            TextButton(onClick = onRelations) { Text("查看关联") }
            TextButton(onClick = onOpen) { Text("打开文档") }
        }
    }
}
"####;

pub const TEST_CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui

import org.junit.Assert.*
import org.junit.Test
import java.util.Locale

class KnowledgeNavigationIndexTest {
    private fun page(id: String, title: String = id, text: String = "", folder: String = "", body: Boolean = true) = KnowledgeNavPage(id, title, folder, text, bodyAvailable = body)
    private fun index(vararg pages: KnowledgeNavPage) = buildKnowledgeNavIndex("workspace", pages.toList())
    private fun KnowledgeNavIndex.links(id: String) = pages.getValue(id).links

    @Test fun emptyWorkspaceIsUsable() {
        val i = index(); assertTrue(i.pages.isEmpty()); assertEquals(0, i.unresolvedLinks); assertTrue(i.search("").isEmpty())
    }
    @Test fun wikiLinksCreateIncomingAndOutgoingRelations() {
        val i = index(page("one", text = "[[two]]"), page("two"))
        assertEquals(listOf("two"), i.links("one").single().candidates)
        assertEquals(setOf("one"), i.incoming["two"])
    }
    @Test fun aliasAndHeadingAreRetainedWithoutInventingBlockNavigation() {
        val l = index(page("a", text = "[[b#概要|另一显示名]]"), page("b")).links("a").single()
        assertEquals("b", l.target); assertEquals("概要", l.anchor); assertEquals("另一显示名", l.label)
    }
    @Test fun blockReferenceResolvesItsOwningDocumentOnly() {
        val l = index(page("a", text = "[[b#^block-1]]"), page("b")).links("a").single()
        assertEquals("^block-1", l.anchor); assertEquals(listOf("b"), l.candidates)
    }
    @Test fun selfAnchorsDoNotCreateForeignBacklinks() {
        val i = index(page("a", text = "[[#标题]]"))
        assertEquals(listOf("a"), i.links("a").single().candidates); assertTrue(i.incoming["a"].isNullOrEmpty())
    }
    @Test fun embedIsAReferenceNotAutomaticallyExecutedContent() {
        val l = index(page("a", text = "![[b]]"), page("b")).links("a").single()
        assertTrue(l.embedded); assertEquals(listOf("b"), l.candidates)
    }
    @Test fun markdownMdLinkDecodesSpaceAndKeepsLiteralPlus() {
        val i = index(page("a", text = "[x](A%20B.md) [y](C++.md#part)"), page("b", "A B"), page("c", "C++"))
        assertEquals(listOf("b", "c"), i.links("a").map { it.candidates.single() })
    }
    @Test fun externalUnsafeAndMalformedDestinationsDoNotBecomeInternalLinks() {
        val i = index(page("a", text = "[x](https://site/x.md) [x](file:///x.md) [x](javascript:x.md) [x](//host/x.md) [x](bad%QQ.md)"))
        assertTrue(i.links("a").isEmpty())
    }
    @Test fun traversalAndUriWikilinksNeverResolveToNotes() {
        val i = index(page("a", text = "[[../secret]] [[https://site]] [[/secret]]"), page("b", "../secret"))
        assertTrue(i.links("a").all { it.candidates.isEmpty() })
    }
    @Test fun missingTargetIsVisibleForRepairNotSilentlyDropped() {
        val i = index(page("a", text = "[[unwritten]]"))
        assertEquals(1, i.unresolvedLinks); assertEquals(listOf("a"), i.search("", filter = KnowledgeNavFilter.UNRESOLVED).map { it.page.id })
    }
    @Test fun duplicateTitlesAreExplicitlyAmbiguous() {
        val i = index(page("a", text = "[[Same]]"), page("b", "Same", folder = "left"), page("c", "Same", folder = "right"))
        assertEquals(listOf("b", "c"), i.links("a").single().candidates)
        assertTrue(i.incoming["b"].isNullOrEmpty()); assertTrue(i.incoming["c"].isNullOrEmpty())
    }
    @Test fun folderQualifiedLinkDisambiguatesTitle() {
        val i = index(page("a", text = "[[right/Same]]"), page("b", "Same", folder = "left"), page("c", "Same", folder = "right"))
        assertEquals(listOf("c"), i.links("a").single().candidates)
    }
    @Test fun sameFolderDuplicatesStillDoNotChooseFirstResult() {
        val i = index(page("a", "Same", folder = "folder"), page("b", "Same", folder = "folder"))
        assertNull(i.wikiLink("a")); assertNull(i.wikiLink("b"))
    }
    @Test fun explicitMdExtensionAndFilenameKeysAgree() {
        val i = index(page("a", text = "[[B.md]]"), page("b", "B.md"))
        assertEquals(listOf("b"), i.links("a").single().candidates)
        assertEquals("[[B.md]]", i.wikiLink("b"))
    }
    @Test fun titleNormalizationUsesNfcAndRootLocale() {
        val old = Locale.getDefault()
        try {
            Locale.setDefault(Locale("tr", "TR"))
            val i = index(page("a", text = "[[IDEA]] [[Cafe\u0301]]"), page("b", "idea"), page("c", "Café"))
            assertEquals(listOf("b", "c"), i.links("a").map { it.candidates.single() })
        } finally { Locale.setDefault(old) }
    }
    @Test fun fencedInlineAndIndentedCodeDoNotProduceTagsLinksOrHeadings() {
        val i = index(page("a", text = "```md\n# hidden\n[[b]] #tag\n```\n`[[b]] #tag`\n    [[b]] #tag\n[[c]]"), page("b"), page("c"))
        assertEquals(listOf("c"), i.links("a").map { it.candidates.single() }); assertTrue(i.pages.getValue("a").tags.isEmpty()); assertTrue(i.pages.getValue("a").headings.isEmpty())
    }
    @Test fun quotedFenceIsNotIndexedAsOrdinaryProse() {
        val i = index(page("a", text = "> ```\n> [[b]]\n> ```\n[[c]]"), page("b"), page("c"))
        assertEquals(listOf("c"), i.links("a").map { it.candidates.single() })
    }
    @Test fun longerFenceCannotBeClosedByShorterMarker() {
        val i = index(page("a", text = "````\n```\n[[b]]\n````\n[[c]]"), page("b"), page("c"))
        assertEquals(listOf("c"), i.links("a").map { it.candidates.single() })
    }
    @Test fun escapedSyntaxAndMultilineCommentsRemainHidden() {
        val i = index(page("a", text = "\\[[b]] \\#tag\n<!-- [[b]]\n#tag -->\n%% [[b]]\n#tag %%\n[[c]]"), page("b"), page("c"))
        assertEquals(listOf("c"), i.links("a").map { it.candidates.single() }); assertTrue(i.pages.getValue("a").tags.isEmpty())
    }
    @Test fun nestedTagsFilterDescendantsAndCountsArePerDocument() {
        val i = index(page("a", text = "#写作/人物 #写作/人物"), page("b", text = "#写作/人物/配角"), page("c", text = "#写作/大纲"))
        assertEquals(1, i.tagCounts["写作/人物"])
        assertEquals(setOf("a", "b"), i.search("", tag = "写作/人物").map { it.page.id }.toSet())
    }
    @Test fun headingAnchorAndNumericHashAreNotTags() {
        val i = index(page("a", text = "# 一级\n## 二级\n[[b#fragment]] #123 #real"), page("b"))
        assertEquals(setOf("real"), i.pages.getValue("a").tags)
        assertEquals(listOf(1, 2), i.pages.getValue("a").headings.map { it.level })
    }
    @Test fun linksRememberSourceLineNumbers() {
        val i = index(page("a", text = "first\n\n[[b]]\n[[b#h]]"), page("b"))
        assertEquals(listOf(3, 4), i.links("a").map { it.line })
    }
    @Test fun duplicateOccurrencesDoNotMultiplyBacklinkDocuments() {
        val i = index(page("a", text = "[[b]] [[b]]"), page("b"))
        assertEquals(2, i.links("a").size); assertEquals(setOf("a"), i.incoming["b"])
    }
    @Test fun allSearchTermsMustMatchAndTitleExactMatchRanksFirst() {
        val i = index(page("a", "计划", "写作", "项目"), page("b", "其他", "计划 写作", ""))
        assertEquals(listOf("a", "b"), i.search("计划").map { it.page.id })
        assertEquals(listOf("a"), i.search("项目 写作").map { it.page.id }); assertTrue(i.search("不存在").isEmpty())
    }
    @Test fun unconnectedFilterDoesNotHideIncomingRelations() {
        val i = index(page("a", text = "[[b]]"), page("b"), page("c"))
        assertEquals(listOf("c"), i.search("", filter = KnowledgeNavFilter.UNCONNECTED).map { it.page.id })
    }
    @Test fun unavailableBodyIsNeverParsedOrReportedAsAnOrphan() {
        val i = index(page("a", "Visible", "[[secret]] #private", body = false))
        assertTrue(i.links("a").isEmpty()); assertTrue(i.tagCounts.isEmpty()); assertTrue(i.search("private").isEmpty())
        assertEquals(1, i.bodyOmissions); assertTrue(i.search("", filter = KnowledgeNavFilter.UNCONNECTED).isEmpty())
    }
    @Test fun privacyPredicateRejectsEncryptedEvenWhenUnlocked() {
        for (doc in listOf(true, false)) for (deleted in listOf(true, false)) for (encrypted in listOf(true, false))
            assertEquals(doc && !deleted && !encrypted, knowledgeNavMayIndex(doc, deleted, encrypted))
    }
    @Test fun metadataTagsAreNormalizedAndAvailableWithoutBodyScan() {
        val i = index(page("a", body = false).copy(metadataTags = listOf("#Work", "work")))
        assertEquals(1, i.tagCounts["work"]); assertEquals(1, i.search("", tag = "work").size)
    }
    @Test fun unlinkedMentionsIgnoreCodeLinksSelfAndSubstringWords() {
        val i = index(page("a", "Alpha"), page("b", text = "Alpha is here"), page("c", text = "[[Alpha]]"),
            page("d", text = "Alphabet"), page("e", text = "`Alpha`"))
        assertEquals(listOf("b"), i.mentionsOf("a").map { it.page.id })
    }
    @Test fun chineseUnlinkedMentionsCanBeFoundInSentences() {
        val i = index(page("a", "写作"), page("b", text = "记录写作的经验"))
        assertEquals(listOf("b"), i.mentionsOf("a").map { it.page.id })
    }
    @Test fun copiedLinkRoundTripsThroughTheActualResolver() {
        for (folder in listOf("", "库", "上级/下级")) {
            val target = page("b", "标题", folder = folder)
            val link = index(target).wikiLink("b")!!
            assertEquals(listOf("b"), index(page("a", text = link), target).links("a").single().candidates)
        }
    }
    @Test fun reservedTitleOrAmbiguousRootTitleCannotGenerateMisleadingLink() {
        for (title in listOf("A#B", "A|B", "A[B]", "A/B", "A:B")) assertNull(index(page("a", title)).wikiLink("a"))
        assertNull(index(page("a", "Same"), page("b", "Same", folder = "nested")).wikiLink("a"))
    }
    @Test fun blankWorkspaceOrRepeatedIdentityIsRejected() {
        for (action in listOf<() -> Unit>({ buildKnowledgeNavIndex("", emptyList()) }, { index(page("a"), page("a")) }, { index(page("")) })) {
            var rejected = false; try { action() } catch (_: IllegalArgumentException) { rejected = true }; assertTrue(rejected)
        }
    }
    @Test fun cancelledIndexCannotBePublishedAsComplete() {
        var calls = 0; var stopped = false
        try { buildKnowledgeNavIndex("w", (0..50).map { page("$it") }) { if (++calls > 3) throw InterruptedException() } }
        catch (_: InterruptedException) { stopped = true }
        assertTrue(stopped)
    }
    @Test fun thousandsOfNotesRemainSearchableWithoutTruncatingResults() {
        val notes = (0 until 1200).map { page("n$it", "标题 $it", if (it > 0) "[[标题 ${it - 1}]]" else "") }
        val original = notes.toList(); val i = buildKnowledgeNavIndex("w", notes)
        assertEquals(1200, i.search("").size); assertEquals(1199, i.resolvedLinks); assertEquals(original, notes)
    }
    @Test fun separateWorkspacesNeverShareIndexOrResolverState() {
        val a = buildKnowledgeNavIndex("a", listOf(page("a", text = "[[b]]")))
        val b = buildKnowledgeNavIndex("b", listOf(page("b")))
        assertTrue(a.links("a").single().candidates.isEmpty()); assertTrue(b.incoming["b"].isNullOrEmpty())
    }
    @Test fun equalContentReplacementSnapshotsStillInvalidateTheEffectKey() {
        val a = mutableListOf("a"); val b = mutableListOf("a"); val folders = listOf("f")
        assertEquals(KnowledgeNavSourceKey(a, folders), KnowledgeNavSourceKey(a, folders))
        assertFalse(KnowledgeNavSourceKey(a, folders) == KnowledgeNavSourceKey(b, folders))
    }
    @Test fun multilineInlineCodeAndListFenceDoNotCreateRelations() {
        val i = index(page("a", text = "`code\n[[b]] #tag`\n- ```\n  [[b]]\n  ```\n[[c]]"), page("b"), page("c"))
        assertEquals(listOf("c"), i.links("a").map { it.candidates.single() })
        assertTrue(i.pages.getValue("a").tags.isEmpty())
    }
}
"####;

pub const SNAPSHOT_TEST_CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui

import org.junit.Assert.*
import org.junit.Test
import com.ofairyo.gridtimer.data.*
import kotlinx.serialization.json.*

class KnowledgeNavigationSnapshotTest {
    private fun note(id: String, text: String = "[[目标]] #public") = NoteEntry(
        id = id, title = id, kind = NoteEntryKind.DOCUMENT,
        document = NoteDocument(markdownEnabled = true, blocks = listOf(NoteBlock(type = NoteBlockType.TEXT, text = text)))
    )

    @Test fun adapterExcludesDeletedStickyLockedAndTemporarilyUnlockedNotes() {
        val plain = note("plain")
        val encrypted = note("secret").copy(encryption = NoteEncryptionEnvelope(keyId = "key"))
        val notes = listOf(plain, encrypted, encrypted.copy(id = "unlocked", encryptionUnlocked = true),
            note("deleted").copy(deletedAtEpochMillis = 1), note("sticky").copy(kind = NoteEntryKind.STICKY))
        val snapshot = buildKnowledgeNavSnapshot("w", notes, emptyList())
        assertSame(notes, snapshot.sourceNotes)
        assertEquals(setOf("plain"), snapshot.index.pages.keys)
        assertEquals(2, snapshot.excludedEncrypted)
        assertTrue(snapshot.index.search("secret").isEmpty())
    }

    @Test fun adapterUsesCurrentFolderAndBlocksWithoutChangingTheNote() {
        val folder = NoteFolder(id = "folder", name = "资料库")
        val source = note("source", "[[资料库/目标]]")
        val target = note("target", "").copy(title = "目标", folderId = folder.id)
        val snapshot = buildKnowledgeNavSnapshot("w", listOf(source, target), listOf(folder))
        assertEquals(listOf("target"), snapshot.index.pages.getValue("source").links.single().candidates)
        assertEquals("[[资料库/目标]]", source.document.blocks.single().text)
    }

    @Test fun richAndStructuredBodiesAreNotFlattenedIntoFalseLinks() {
        val rich = note("rich").copy(document = NoteDocument(richTextEnabled = true, richTextPlainText = "[[private]] #hidden"))
        val structured = note("structured").let { it.copy(document = it.document.copy(knowledge = buildJsonObject {
            put("tags", buildJsonArray { add("project") })
        })) }
        val index = buildKnowledgeNavSnapshot("w", listOf(rich, structured), emptyList()).index
        assertEquals(2, index.bodyOmissions)
        assertTrue(index.pages.values.all { it.links.isEmpty() })
        assertEquals(1, index.tagCounts["project"])
    }

    @Test fun largeBodiesStayNavigableAndCoverageIsExplicit() {
        val source = note("large", "x".repeat(512_001) + "[[目标]]")
        val index = buildKnowledgeNavSnapshot("w", listOf(source), emptyList()).index
        assertEquals(1, index.bodyOmissions)
        assertEquals(1, index.search("large").size)
        assertTrue(index.pages.getValue("large").links.isEmpty())
    }

    @Test fun encryptedMetadataCannotEnterTagNavigation() {
        val source = note("secret").let { it.copy(encryption = NoteEncryptionEnvelope(keyId = "key"),
            document = it.document.copy(knowledge = buildJsonObject { put("tags", buildJsonArray { add("secret-tag") }) })) }
        val index = buildKnowledgeNavSnapshot("w", listOf(source), emptyList()).index
        assertTrue(index.tagCounts.isEmpty())
    }
}
"####;


#[cfg(test)]
mod tests {
    use super::*;

    fn studio() -> &'static str {
        super::super::kotlin_sources::SOURCES.iter()
            .find(|item| item.path == SCREEN_PATH).expect("actual note studio template").contents
    }

    #[test]
    fn actual_collection_opens_navigation_and_reuses_the_existing_note_route() {
        let source = render(SCREEN_PATH, studio()).unwrap();
        assert_eq!(source.matches("KnowledgeNavigationDialog(").count(), 1);
        assert_eq!(source.matches("knowledge_open_navigation").count(), 1);
        assert!(source.contains("onOpenNote(note)"));
        assert!(source.contains("if (knowledgeNavigationVisible && !trashMode)"));
    }

    #[test]
    fn prior_collection_transforms_can_run_before_this_navigation_transform() {
        let source = super::super::android_note_list_performance::render(SCREEN_PATH, studio()).unwrap();
        let source = super::super::android_knowledge_filters::render(SCREEN_PATH, &source).unwrap();
        let source = super::super::android_ai_workflow::render(SCREEN_PATH, &source).unwrap();
        let source = render(SCREEN_PATH, &source).unwrap();
        assert!(source.contains("knowledge_open_navigation"));
        assert!(source.contains("KnowledgeAiDialog("));
    }

    #[test]
    fn missing_duplicate_and_reapplied_hooks_fail_closed() {
        assert!(render(SCREEN_PATH, "changed template").is_err());
        assert!(render(SCREEN_PATH, &format!("{}\n{STATE_ANCHOR}", studio())).is_err());
        let once = render(SCREEN_PATH, studio()).unwrap();
        assert!(render(SCREEN_PATH, &once).is_err());
    }

    #[test]
    fn transform_preserves_editor_save_and_sticky_source_bytes() {
        let original = studio();
        let mut restored = render(SCREEN_PATH, original).unwrap()
            .replacen(STATE, STATE_ANCHOR, 1).replacen(ENTRY, ENTRY_ANCHOR, 1);
        for name in ["androidx.compose.foundation.layout.heightIn", "androidx.compose.ui.platform.testTag"] {
            let import = format!("import {name}\n");
            if !original.contains(&import) { restored = restored.replacen(&import, "", 1); }
        }
        assert_eq!(restored, original);
    }

    #[test]
    fn privacy_guard_precedes_all_title_body_and_property_access() {
        let adapter = UI_CONTENTS.split("for (note in notes)").nth(1).unwrap();
        let guard = adapter.find("if (!knowledgeNavMayIndex(").unwrap();
        assert!(guard < adapter.find("note.resolvedDocument()").unwrap());
        assert!(guard < adapter.find("note.displayTitle()").unwrap());
        assert!(UI_CONTENTS.contains("sourceNotes === appData.notes"));
        assert!(UI_CONTENTS.contains("KnowledgeNavSourceKey(appData.notes, appData.noteFolders)"));
        assert!(UI_CONTENTS.contains("live.encryption != null"));
    }

    #[test]
    fn generated_navigation_cannot_mutate_notes_or_send_them_to_a_provider() {
        for forbidden in ["upsertNote", "deleteNote", "runLegalScan", "completeKnowledgeWithAi", "httpClient", "writeText("] {
            assert!(!UI_CONTENTS.contains(forbidden), "unexpected side effect {forbidden}");
            assert!(!INDEX_CONTENTS.contains(forbidden), "unexpected index side effect {forbidden}");
        }
        assert!(UI_CONTENTS.contains("withContext(Dispatchers.Default)"));
        assert!(UI_CONTENTS.contains("worker.ensureActive()"));
        assert!(UI_CONTENTS.contains("SecureFlagPolicy.SecureOn"));
    }

    #[test]
    fn generator_emits_both_production_files_and_both_real_jvm_test_suites() {
        let generator = include_str!("../bin/gridtimer_sourcegen.rs");
        for suffix in ["INDEX_PATH", "UI_PATH", "TEST_PATH", "SNAPSHOT_TEST_PATH"] {
            assert!(generator.contains(&format!("android_knowledge_navigation::{suffix}")));
        }
        let write = generator.split("fn write_source(").nth(1).unwrap();
        assert!(write.find("android_knowledge_navigation::render").unwrap() < write.find("fs::write").unwrap());
    }

    #[test]
    fn unrelated_kotlin_and_formatted_preview_are_unchanged() {
        for path in ["com/ofairyo/gridtimer/ui/NoteDocumentEditor.kt", "com/ofairyo/gridtimer/ui/AndroidRenderedMarkdown.kt", TEST_PATH] {
            assert_eq!(render(path, "unrelated source").unwrap(), "unrelated source");
        }
    }
}
