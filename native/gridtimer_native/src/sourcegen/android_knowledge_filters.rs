// v2.23.2 - Keep knowledge filters local, recover the full list, and show explicit scopes.
// UI, state, and JVM tests are authored in Rust and emitted by sourcegen.

const SCREEN: &str = "com/ofairyo/gridtimer/ui/NoteStudioSheet.kt";
pub const PATH: &str = "com/ofairyo/gridtimer/ui/KnowledgeFilterState.kt";
pub const TEST_PATH: &str = "com/ofairyo/gridtimer/ui/KnowledgeFilterStateTest.kt";

fn replace(source: &mut String, before: &str, after: &str) -> Result<(), String> {
    if source.matches(before).count() != 1 {
        return Err(format!("knowledge filter anchor is not unique: {before}"));
    }
    *source = source.replacen(before, after, 1);
    Ok(())
}

fn replace_range(source: &mut String, start: &str, end: &str, value: &str) -> Result<(), String> {
    let from = source
        .find(start)
        .ok_or_else(|| format!("missing knowledge filter start: {start}"))?;
    let to = source[from..]
        .find(end)
        .map(|offset| from + offset)
        .ok_or_else(|| format!("missing knowledge filter end: {end}"))?;
    source.replace_range(from..to, value);
    Ok(())
}

pub fn render(path: &str, source: &str) -> Result<String, String> {
    if path != SCREEN {
        return Ok(source.to_owned());
    }
    let mut result = source.to_owned();
    let from = result
        .find("internal fun NoteStudioSheet(")
        .ok_or("missing knowledge studio")?;
    let to = result[from..]
        .find("internal fun StickyNoteStudioSheet(")
        .map(|offset| from + offset)
        .ok_or("missing sticky studio boundary")?;
    let mut studio = result[from..to].to_owned();
    replace(&mut studio, "    val context = LocalContext.current", "    val context = LocalContext.current\n    val workspaceKey = LocalNoteMediaWorkspaceKey.current")?;
    replace(&mut studio,
        "    var query by rememberSaveable { mutableStateOf(\"\") }",
        "    var savedKnowledgeFilters by rememberSaveable(workspaceKey, stateSaver = KnowledgeFilterStateSaver) {\n        mutableStateOf(KnowledgeFilterState(workspaceKey))\n    }\n    val knowledgeFilters = savedKnowledgeFilters.forWorkspace(workspaceKey)\n    val query = knowledgeFilters.query")?;
    replace(
        &mut studio,
        "    var trashMode by rememberSaveable { mutableStateOf(false) }",
        "    val trashMode = knowledgeFilters.trashMode",
    )?;
    replace(
        &mut studio,
        "    var quickFilter by rememberSaveable { mutableStateOf(NoteQuickFilter.ALL) }",
        "    val quickFilter = knowledgeFilters.quickFilter",
    )?;
    replace(&mut studio, "    var collectionView by rememberSaveable { mutableStateOf(NoteCollectionView.RECENT) }", "    val collectionView = knowledgeFilters.collectionView\n    val selectedSortMode = knowledgeFilters.effectiveSortMode(appData.notePreferences.sortMode)")?;
    // Dialogs, editor selection, and filters must not leak from a previous account.
    studio = studio.replace(
        "by rememberSaveable {",
        "by rememberSaveable(workspaceKey) {",
    );
    studio = studio.replace("by remember {", "by remember(workspaceKey) {");
    replace(&mut studio,
        "    val selectedFolderId = appData.notePreferences.selectedFolderId?.takeIf { selectedId ->\n        appData.noteFolders.any { folder -> folder.id == selectedId }\n    }",
        "    val selectedFolderId = knowledgeFilters.resolvedFolderId(appData.noteFolders)")?;
    replace(&mut studio,
        "    val activeNotes = remember(appData.notes, appData.notePreferences, appData.noteFolders) {\n        appData.sortedActiveNotebookDocuments(selectedFolderId)\n    }",
        "    val activeNotes = remember(appData.notes, appData.notePreferences, appData.noteFolders, selectedFolderId, selectedSortMode) {\n        appData.copy(notePreferences = appData.notePreferences.copy(sortMode = selectedSortMode))\n            .sortedActiveNotebookDocuments(selectedFolderId)\n    }")?;
    replace_range(&mut studio, "    val quickFilterCounts = remember(activeNotes) {", "    val trashedNotes =", "    val quickFilterCounts = remember(appData.notes, appData.noteFolders, knowledgeFilters) {\n        knowledgeFilters.quickFilterCounts(appData.notes, appData.noteFolders)\n    }\n    val filterFolderCounts = remember(appData.notes, appData.noteFolders, knowledgeFilters) {\n        knowledgeFilters.folderCounts(appData.notes, appData.noteFolders)\n    }\n")?;
    replace_range(&mut studio, "    val quickFilteredActiveNotes = remember(activeNotes, quickFilter) {", "    val activeSections =", "    val filteredActiveNotes = remember(activeNotes, knowledgeFilters, appData.noteFolders) {\n        knowledgeFilters.visibleDocuments(activeNotes, appData.noteFolders)\n    }\n")?;
    replace(&mut studio, "                    selectedFolderId = selectedFolderId,", "                    selectedFolderId = selectedFolderId,\n                    sortMode = selectedSortMode,")?;
    // Folder-manager totals remain unfiltered; only the selector uses candidate counts.
    replace(
        &mut studio,
        "                    folderCounts = folderCounts,",
        "                    folderCounts = filterFolderCounts,",
    )?;
    replace(&mut studio, "                    onQueryChange = { query = it },", "                    onQueryChange = { savedKnowledgeFilters = savedKnowledgeFilters.forWorkspace(workspaceKey).copy(query = it) },")?;
    replace(&mut studio, "                    onToggleTrash = { trashMode = !trashMode },", "                    onToggleTrash = {\n                        val current = savedKnowledgeFilters.forWorkspace(workspaceKey)\n                        savedKnowledgeFilters = current.copy(trashMode = !current.trashMode)\n                    },")?;
    replace(&mut studio, "                    onSelectQuickFilter = { quickFilter = it },", "                    onSelectQuickFilter = { candidate ->\n                        val current = savedKnowledgeFilters.forWorkspace(workspaceKey)\n                        savedKnowledgeFilters = current.selectQuickFilter(candidate, current.quickFilterCounts(appData.notes, appData.noteFolders))\n                    },")?;
    replace(&mut studio, "                    onSelectCollectionView = { collectionView = it },", "                    onSelectCollectionView = { savedKnowledgeFilters = savedKnowledgeFilters.forWorkspace(workspaceKey).copy(collectionView = it) },")?;
    replace(&mut studio, "                    onSelectFolder = viewModel::selectNoteFolder,", "                    onSelectFolder = { candidate ->\n                        val current = savedKnowledgeFilters.forWorkspace(workspaceKey)\n                        savedKnowledgeFilters = current.selectFolder(candidate, current.folderCounts(appData.notes, appData.noteFolders), appData.noteFolders)\n                    },")?;
    replace(&mut studio, "                    onSelectSortMode = viewModel::setNoteSortMode,", "                    onSelectSortMode = { candidate ->\n                        val current = savedKnowledgeFilters.forWorkspace(workspaceKey)\n                        val next = current.selectSortMode(candidate, appData.notePreferences.sortMode)\n                        if (next !== current) {\n                            savedKnowledgeFilters = next\n                            viewModel.setNoteSortMode(candidate, workspaceKey)\n                        }\n                    },\n                    onShowAll = { savedKnowledgeFilters = savedKnowledgeFilters.forWorkspace(workspaceKey).showAll() },")?;
    replace(&mut studio, "                        query = \"\"\n                        quickFilter = NoteQuickFilter.ALL\n                        trashMode = false", "                        savedKnowledgeFilters = savedKnowledgeFilters.forWorkspace(workspaceKey).copy(\n                            query = \"\", quickFilter = NoteQuickFilter.ALL, trashMode = false\n                        )")?;
    result.replace_range(from..to, &studio);

    // Sticky-note local UI state also clears when the account changes.
    let from = result
        .find("internal fun StickyNoteStudioSheet(")
        .ok_or("missing sticky studio")?;
    let to = result[from..]
        .find("internal fun NoteEntry.attachmentPreviewProjection(")
        .map(|offset| from + offset)
        .ok_or("missing sticky studio end")?;
    let mut sticky = result[from..to].to_owned();
    replace(&mut sticky, "    val context = LocalContext.current", "    val context = LocalContext.current\n    val workspaceKey = LocalNoteMediaWorkspaceKey.current")?;
    sticky = sticky.replace(
        "by rememberSaveable {",
        "by rememberSaveable(workspaceKey) {",
    );
    sticky = sticky.replace("by remember {", "by remember(workspaceKey) {");
    result.replace_range(from..to, &sticky);

    let from = result
        .find("private fun NoteCollectionContent(")
        .ok_or("missing knowledge collection")?;
    let to = result[from..]
        .find("private fun flowusPageBackground()")
        .map(|offset| from + offset)
        .ok_or("missing knowledge collection end")?;
    let mut collection = result[from..to].to_owned();
    replace(
        &mut collection,
        "    selectedFolderId: String?,",
        "    selectedFolderId: String?,\n    sortMode: NoteSortMode,",
    )?;
    replace(
        &mut collection,
        "    onSelectSortMode: (NoteSortMode) -> Unit,",
        "    onSelectSortMode: (NoteSortMode) -> Unit,\n    onShowAll: () -> Unit,",
    )?;
    replace(&mut collection, "                    sortMode = appData.notePreferences.sortMode,", "                    sortMode = sortMode,\n                    query = query,\n                    resultCount = filteredActiveNotes.size,\n                    totalCount = notebookDocuments.size,")?;
    replace(&mut collection, "                    onSelectSortMode = onSelectSortMode\n", "                    onSelectSortMode = onSelectSortMode,\n                    onShowAll = onShowAll\n")?;
    replace_range(
        &mut collection,
        "        if (!trashMode && filteredActiveNotes.isEmpty()) {",
        "        if (trashMode && filteredTrashedNotes.isEmpty()) {",
        EMPTY_STATE,
    )?;
    result.replace_range(from..to, &collection);

    replace_range(
        &mut result,
        "private fun NoteCollectionControls(",
        "private data class NoteCollectionViewOption(",
        CONTROLS,
    )?;
    replace(
        &mut result,
        "private fun noteMatchesQuery(",
        "internal fun noteMatchesQuery(",
    )?;
    Ok(result)
}

pub const CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui

import androidx.compose.runtime.saveable.Saver
import com.ofairyo.gridtimer.data.*

// View state never writes or removes documents. The workspace is part of the
// saved value as well as rememberSaveable's key, including process restoration.
internal data class KnowledgeFilterState(
    val workspaceKey: String,
    val query: String = "",
    val folderId: String? = null,
    val quickFilter: NoteQuickFilter = NoteQuickFilter.ALL,
    val trashMode: Boolean = false,
    val collectionView: NoteCollectionView = NoteCollectionView.RECENT,
    val sortMode: NoteSortMode? = null
) {
    fun forWorkspace(currentWorkspaceKey: String): KnowledgeFilterState =
        if (workspaceKey == currentWorkspaceKey) this else KnowledgeFilterState(currentWorkspaceKey)

    fun resolvedFolderId(folders: List<NoteFolder>): String? =
        folderId?.takeIf { id -> folders.any { it.id == id } }

    fun effectiveSortMode(preference: NoteSortMode): NoteSortMode = sortMode ?: preference

    fun selectSortMode(candidate: NoteSortMode, preference: NoteSortMode): KnowledgeFilterState =
        if (effectiveSortMode(preference) == candidate) this else copy(sortMode = candidate)

    fun showAll(): KnowledgeFilterState = copy(
        query = "", folderId = null, quickFilter = NoteQuickFilter.ALL, trashMode = false
    )

    fun selectQuickFilter(candidate: NoteQuickFilter, counts: Map<NoteQuickFilter, Int>): KnowledgeFilterState {
        if (candidate == quickFilter) return this
        if (candidate != NoteQuickFilter.ALL && (counts[candidate] ?: 0) <= 0) return this
        return copy(quickFilter = candidate)
    }

    fun selectFolder(candidate: String?, counts: Map<String, Int>, folders: List<NoteFolder>): KnowledgeFilterState {
        if (candidate == folderId) return this
        if (candidate != null && (folders.none { it.id == candidate } || (counts[candidate] ?: 0) <= 0)) return this
        return copy(folderId = candidate)
    }

    fun visibleDocuments(notes: List<NoteEntry>, folders: List<NoteFolder>): List<NoteEntry> {
        val scope = resolvedFolderId(folders)
        return notes.filter { note ->
            !note.isDeleted() && note.kind == NoteEntryKind.DOCUMENT &&
                (scope == null || note.folderId == scope) && note.matchesQuickFilter(quickFilter) &&
                noteMatchesQuery(query, note, folders.firstOrNull { it.id == note.folderId })
        }
    }

    fun quickFilterCounts(notes: List<NoteEntry>, folders: List<NoteFolder>): Map<NoteQuickFilter, Int> =
        NoteQuickFilter.entries.associateWith { candidate -> copy(quickFilter = candidate).visibleDocuments(notes, folders).size }

    fun folderCounts(notes: List<NoteEntry>, folders: List<NoteFolder>): Map<String, Int> {
        val candidates = copy(folderId = null).visibleDocuments(notes, folders)
        return candidates.mapNotNull(NoteEntry::folderId).groupingBy { it }.eachCount() + ("" to candidates.size)
    }

    fun savedValues(): List<Any> = listOf(workspaceKey, query, folderId.orEmpty(), quickFilter.name, trashMode, collectionView.name, sortMode?.name.orEmpty())

    companion object {
        fun fromSavedValues(values: List<Any>): KnowledgeFilterState? = runCatching {
            require(values.size == 7)
            KnowledgeFilterState(
                workspaceKey = values[0] as String,
                query = values[1] as String,
                folderId = (values[2] as String).takeIf(String::isNotBlank),
                quickFilter = NoteQuickFilter.valueOf(values[3] as String),
                trashMode = values[4] as Boolean,
                collectionView = NoteCollectionView.valueOf(values[5] as String),
                sortMode = (values[6] as String).takeIf(String::isNotBlank)?.let(NoteSortMode::valueOf)
            )
        }.getOrNull()
    }
}

internal val KnowledgeFilterStateSaver = Saver<KnowledgeFilterState, List<Any>>(
    save = { it.savedValues() }, restore = KnowledgeFilterState::fromSavedValues
)
"####;

const EMPTY_STATE: &str = r####"        if (!trashMode && filteredActiveNotes.isEmpty()) {
            item {
                if (notebookDocuments.isNotEmpty()) {
                    FlowusPanel(accent = accent) {
                        Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                            Text("当前筛选无结果", style = MaterialTheme.typography.titleSmall)
                            Text("已有 ${notebookDocuments.size} 页", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            androidx.compose.material3.TextButton(onClick = onShowAll) { Text("显示全部") }
                        }
                    }
                } else {
                    NoteEmptyCard(title = "还没有文档", description = "点击新建，添加第一篇文档。", accent = accent)
                }
            }
        }

"####;

const CONTROLS: &str = r####"private fun NoteCollectionControls(
    quickFilter: NoteQuickFilter,
    quickFilterCounts: Map<NoteQuickFilter, Int>,
    selectedFolderId: String?,
    sortMode: NoteSortMode,
    query: String,
    resultCount: Int,
    totalCount: Int,
    folders: List<NoteFolder>,
    folderCounts: Map<String, Int>,
    onSelectQuickFilter: (NoteQuickFilter) -> Unit,
    onSelectFolder: (String?) -> Unit,
    onSelectSortMode: (NoteSortMode) -> Unit,
    onShowAll: () -> Unit
) {
    val workspaceKey = LocalNoteMediaWorkspaceKey.current
    var openSelector by rememberSaveable(workspaceKey) { mutableStateOf<String?>(null) }
    val selectedType = noteQuickFilterOptions().first { it.filter == quickFilter }
    val selectedFolder = folders.firstOrNull { it.id == selectedFolderId }
    val selectedSort = noteSortOptions().first { it.mode == sortMode }
    val hasFilters = query.isNotBlank() || quickFilter != NoteQuickFilter.ALL || selectedFolderId != null
    FlowusPanel(accent = notesAccentColor) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 14.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
                Text("显示 $resultCount / $totalCount 页", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                if (hasFilters) androidx.compose.material3.TextButton(onClick = onShowAll) { Text("显示全部") }
            }
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Text("文档库", Modifier.width(62.dp), style = MaterialTheme.typography.bodySmall)
                Box(Modifier.weight(1f)) {
                    androidx.compose.material3.TextButton(onClick = { openSelector = "folder" }, modifier = Modifier.fillMaxWidth()) {
                        Text(selectedFolder?.name ?: "全部文档", Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Text("▾")
                    }
                    androidx.compose.material3.DropdownMenu(expanded = openSelector == "folder", onDismissRequest = { openSelector = null }) {
                        androidx.compose.material3.DropdownMenuItem(text = { Text("全部文档 ${folderCounts[""] ?: totalCount}") }, onClick = { openSelector = null; onSelectFolder(null) })
                        folders.forEach { folder ->
                            val count = folderCounts[folder.id] ?: 0
                            androidx.compose.material3.DropdownMenuItem(
                                text = { Text("${folder.name} $count") },
                                enabled = folder.id == selectedFolderId || count > 0,
                                onClick = { openSelector = null; onSelectFolder(folder.id) }
                            )
                        }
                    }
                }
            }
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Text("类型", Modifier.width(62.dp), style = MaterialTheme.typography.bodySmall)
                Box(Modifier.weight(1f)) {
                    androidx.compose.material3.TextButton(onClick = { openSelector = "type" }, modifier = Modifier.fillMaxWidth()) {
                        Text(selectedType.label, Modifier.weight(1f))
                        Text("▾")
                    }
                    androidx.compose.material3.DropdownMenu(expanded = openSelector == "type", onDismissRequest = { openSelector = null }) {
                        noteQuickFilterOptions().forEach { option ->
                            val count = quickFilterCounts[option.filter] ?: 0
                            androidx.compose.material3.DropdownMenuItem(
                                text = { Text("${option.label} $count") },
                                enabled = option.filter == NoteQuickFilter.ALL || option.filter == quickFilter || count > 0,
                                onClick = { openSelector = null; onSelectQuickFilter(option.filter) }
                            )
                        }
                    }
                }
            }
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Text("排序", Modifier.width(62.dp), style = MaterialTheme.typography.bodySmall)
                Box(Modifier.weight(1f)) {
                    androidx.compose.material3.TextButton(onClick = { openSelector = "sort" }, modifier = Modifier.fillMaxWidth()) {
                        Text(selectedSort.label, Modifier.weight(1f))
                        Text("▾")
                    }
                    androidx.compose.material3.DropdownMenu(expanded = openSelector == "sort", onDismissRequest = { openSelector = null }) {
                        noteSortOptions().forEach { option ->
                            androidx.compose.material3.DropdownMenuItem(text = { Text(option.label) }, onClick = { openSelector = null; onSelectSortMode(option.mode) })
                        }
                    }
                }
            }
            if (hasFilters) {
                val summary = buildList {
                    selectedFolder?.let { add(it.name) }
                    if (quickFilter != NoteQuickFilter.ALL) add(selectedType.label)
                    if (query.isNotBlank()) add("搜索：$query")
                }.joinToString(" · ")
                Text(summary, Modifier.padding(bottom = 4.dp), style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}

"####;

pub const TEST_CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui

import com.ofairyo.gridtimer.data.*
import org.junit.Assert.*
import org.junit.Test

class KnowledgeFilterStateTest {
    private val folders = listOf(NoteFolder(id = "work", name = "Work"), NoteFolder(id = "empty", name = "Empty"))
    private val notes = listOf(
        NoteEntry(id = "filed", title = "Alpha", content = "original A", folderId = "work", kind = NoteEntryKind.DOCUMENT, pinned = true),
        NoteEntry(id = "unfiled", title = "Beta", content = "original B", kind = NoteEntryKind.DOCUMENT),
        NoteEntry(id = "removed", title = "Removed", kind = NoteEntryKind.DOCUMENT, deletedAtEpochMillis = 1L),
        NoteEntry(id = "sticky", title = "Sticky", kind = NoteEntryKind.STICKY)
    )

    @Test fun showAllAtomicallyRestoresExistingDocumentsAndKeepsSortChoice() {
        val selected = KnowledgeFilterState("account-a", query = "missing", folderId = "empty", quickFilter = NoteQuickFilter.IMAGE, trashMode = true, sortMode = NoteSortMode.TITLE_ASC)
        assertTrue(selected.visibleDocuments(notes, folders).isEmpty())
        val restored = selected.showAll()
        assertEquals(listOf("filed", "unfiled"), restored.visibleDocuments(notes, folders).map { it.id })
        assertFalse(restored.trashMode)
        assertEquals(NoteSortMode.TITLE_ASC, restored.effectiveSortMode(NoteSortMode.UPDATED_DESC))
        assertSame(notes[0], restored.visibleDocuments(notes, folders)[0])
    }

    @Test fun unavailableChoicesCannotHideTheListAndAZeroSelectionAlwaysHasAnEscape() {
        val state = KnowledgeFilterState("account-a")
        assertSame(state, state.selectQuickFilter(NoteQuickFilter.IMAGE, state.quickFilterCounts(notes, folders)))
        assertSame(state, state.selectFolder("empty", state.folderCounts(notes, folders), folders))
        assertSame(state, state.selectFolder("missing", mapOf("missing" to 9), folders))
        val pinned = state.selectQuickFilter(NoteQuickFilter.PINNED, state.quickFilterCounts(notes, folders))
        assertEquals(listOf("filed"), pinned.visibleDocuments(notes, folders).map { it.id })
        val zero = pinned.copy(query = "missing")
        assertEquals(NoteQuickFilter.ALL, zero.selectQuickFilter(NoteQuickFilter.ALL, zero.quickFilterCounts(notes, folders)).quickFilter)
        assertEquals(listOf("filed", "unfiled"), zero.showAll().visibleDocuments(notes, folders).map { it.id })
    }

    @Test fun savedStateCannotApplyAnotherWorkspacesSearchOrScope() {
        val state = KnowledgeFilterState("account-a", "missing", "work", NoteQuickFilter.PINNED, true, NoteCollectionView.BOARD, NoteSortMode.CREATED_ASC)
        val saved = KnowledgeFilterState.fromSavedValues(state.savedValues())!!
        assertEquals(state, saved)
        assertSame(saved, saved.forWorkspace("account-a"))
        val other = saved.forWorkspace("account-b")
        assertEquals("account-b", other.workspaceKey)
        assertEquals(listOf("filed", "unfiled"), other.visibleDocuments(notes, folders).map { it.id })
        assertFalse(other.trashMode)
        assertEquals(NoteCollectionView.RECENT, other.collectionView)
        assertNull(other.sortMode)
        assertNull(KnowledgeFilterState.fromSavedValues(listOf("broken")))
        assertEquals(2, state.copy(folderId = "deleted-folder", query = "", quickFilter = NoteQuickFilter.ALL).visibleDocuments(notes, folders).size)
    }

    @Test fun candidateCountsReflectSearchAndScopeAndRepeatedSortChoicesAreNoOps() {
        val state = KnowledgeFilterState("account-a", query = "Alpha")
        assertEquals(1, state.quickFilterCounts(notes, folders)[NoteQuickFilter.ALL])
        assertEquals(1, state.folderCounts(notes, folders)["work"])
        assertEquals(1, state.folderCounts(notes, folders)[""])
        val data = AppData(notes = notes, noteFolders = folders)
        var current = KnowledgeFilterState("account-a")
        repeat(25) {
            for (mode in NoteSortMode.entries) {
                current = current.selectSortMode(mode, data.notePreferences.sortMode)
                assertSame(current, current.selectSortMode(mode, data.notePreferences.sortMode))
                val sorted = data.copy(notePreferences = data.notePreferences.copy(sortMode = current.effectiveSortMode(data.notePreferences.sortMode))).sortedActiveNotebookDocuments()
                assertEquals(setOf("filed", "unfiled"), current.showAll().visibleDocuments(sorted, folders).map { it.id }.toSet())
                assertEquals(notes, data.notes)
            }
        }
    }
}
"####;
