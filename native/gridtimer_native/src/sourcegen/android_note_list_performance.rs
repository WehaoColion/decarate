// v2.22.49.5 Android - Filter notes without account serialization or unused title parsing.
pub fn render(path: &str, source: &str) -> Result<String, String> {
    let mut result = source.to_owned();
    if path.ends_with("/Models.kt") {
        replace_section(
            &mut result,
            "fun AppData.activeNotes():",
            "internal fun AppData.allNoteAttachments()",
            VISIBILITY,
        )?;
        replace_section(
            &mut result,
            "fun AppData.sortedActiveNotes(",
            "fun AppData.sortedTrashedNotes()",
            SORTING,
        )?;
    } else if path.ends_with("/NoteStudioSheet.kt") {
        let anchor = "    val notebookDocuments = remember(appData.notes) { appData.activeNotebookDocuments() }";
        if !result.contains(anchor) {
            return Err("note summary anchor missing".into());
        }
        result = result.replacen(anchor, SUMMARY, 1);
        for (before, after) in [
            ("trashCount = appData.sortedTrashedNotebookDocuments().size,", "trashCount = summaryTrashCount,"),
            ("pinnedCount = notebookDocuments.count(NoteEntry::pinned),", "pinnedCount = summaryPinnedCount,"),
            ("pendingChecklistCount = notebookDocuments.sumOf { note -> note.checklistStats()?.remaining ?: 0 },", "pendingChecklistCount = summaryPendingCount,"),
            ("imageNoteCount = notebookDocuments.count { it.attachmentCount(NoteAttachmentKind.IMAGE) > 0 },", "imageNoteCount = summaryImageCount,"),
        ] {
            if !result.contains(before) { return Err(format!("note summary field missing: {before}")); }
            result = result.replacen(before, after, 1);
        }
    }
    Ok(result)
}

fn replace_section(
    source: &mut String,
    start: &str,
    end: &str,
    replacement: &str,
) -> Result<(), String> {
    let from = source
        .find(start)
        .ok_or_else(|| format!("note list anchor missing: {start}"))?;
    let to = source[from..]
        .find(end)
        .map(|offset| from + offset)
        .ok_or_else(|| format!("note list end missing: {end}"))?;
    source.replace_range(from..to, replacement);
    Ok(())
}

const VISIBILITY: &str = r###"fun AppData.activeNotes(): List<NoteEntry> = notes.filterNot(NoteEntry::isDeleted)

fun AppData.trashedNotes(): List<NoteEntry> = notes.filter(NoteEntry::isDeleted)

fun AppData.activeStickyNotes(): List<NoteEntry> =
    notes.filter { !it.isDeleted() && it.isStickyNote() }

fun AppData.activeNotebookDocuments(): List<NoteEntry> =
    notes.filter { !it.isDeleted() && it.isNotebookDocument() }

fun AppData.trashedNotebookDocuments(): List<NoteEntry> =
    notes.filter { it.isDeleted() && it.isNotebookDocument() }

"###;

const SORTING: &str = r###"// CREATED_ASC also uses titles to break equal creation timestamps.
internal fun noteSortTitles(notes: List<NoteEntry>, mode: NoteSortMode, titleOf: (NoteEntry) -> String = NoteEntry::displayTitle): Array<String> =
    if (mode == NoteSortMode.CREATED_ASC || mode == NoteSortMode.TITLE_ASC)
        Array(notes.size) { titleOf(notes[it]) }
    else Array(notes.size) { "" }

private fun AppData.sortedActiveNotesOfKind(folderId: String?, kind: NoteEntryKind?): List<NoteEntry> {
    val filtered = notes.filter { note ->
        !note.isDeleted() && (kind == null || note.kind == kind) && (folderId == null || note.folderId == folderId)
    }
    if (filtered.size < 2) return filtered
    NativeOptimizerBridge.sortNoteIndices(
        sortModeCode = notePreferences.sortMode.ordinal,
        pinnedFlags = BooleanArray(filtered.size) { filtered[it].pinned },
        createdAtEpochMillis = LongArray(filtered.size) { filtered[it].createdAtEpochMillis },
        updatedAtEpochMillis = LongArray(filtered.size) { filtered[it].updatedAtEpochMillis },
        titles = noteSortTitles(filtered, notePreferences.sortMode)
    )?.let { sortedIndices -> return sortedIndices.map(filtered::get) }
    return filtered.sortedWith(noteComparator(notePreferences.sortMode))
}

fun AppData.sortedActiveNotes(folderId: String? = null): List<NoteEntry> =
    sortedActiveNotesOfKind(folderId, null)

fun AppData.sortedActiveStickyNotes(folderId: String? = null): List<NoteEntry> =
    sortedActiveNotesOfKind(folderId, NoteEntryKind.STICKY)

fun AppData.sortedActiveNotebookDocuments(folderId: String? = null): List<NoteEntry> =
    sortedActiveNotesOfKind(folderId, NoteEntryKind.DOCUMENT)

"###;

const SUMMARY: &str = r###"    val notebookDocuments = remember(appData.notes) { appData.activeNotebookDocuments() }
    val summaryTrashCount = remember(appData.notes) { appData.notes.count { it.isDeleted() && it.kind == NoteEntryKind.DOCUMENT } }
    val summaryPinnedCount = remember(notebookDocuments) { notebookDocuments.count(NoteEntry::pinned) }
    val summaryPendingCount = remember(notebookDocuments) { notebookDocuments.sumOf { it.checklistStats()?.remaining ?: 0 } }
    val summaryImageCount = remember(notebookDocuments) { notebookDocuments.count { it.attachmentCount(NoteAttachmentKind.IMAGE) > 0 } }"###;

pub const TEST_PATH: &str = "com/ofairyo/gridtimer/ui/NoteListPerformanceTest.kt";
pub const TEST_CONTENTS: &str = r###"package com.ofairyo.gridtimer.ui

import com.ofairyo.gridtimer.data.*
import org.junit.Assert.*
import org.junit.Test

class NoteListPerformanceTest {
    private fun note(id: String, kind: NoteEntryKind = NoteEntryKind.DOCUMENT, folder: String = "a", deleted: Long? = null, created: Long = 10, updated: Long = 10, pinned: Boolean = false, title: String = id) =
        NoteEntry(id=id, kind=kind, folderId=folder, deletedAtEpochMillis=deleted, createdAtEpochMillis=created, updatedAtEpochMillis=updated, pinned=pinned, title=title)

    @Test fun activeAndTrashCollectionsKeepTheirDeletionAndKindBoundaries() {
        val live=note("live"); val sticky=note("sticky",NoteEntryKind.STICKY)
        val removed=note("removed",deleted=0); val removedSticky=note("removedSticky",NoteEntryKind.STICKY,deleted=20)
        val data=AppData(notes=listOf(live,sticky,removed,removedSticky))
        assertEquals(listOf(live,sticky),data.activeNotes()); assertEquals(listOf(removed,removedSticky),data.trashedNotes())
        assertEquals(listOf(live),data.activeNotebookDocuments()); assertEquals(listOf(sticky),data.activeStickyNotes())
        assertEquals(listOf(removed),data.trashedNotebookDocuments()); assertSame(live,data.activeNotes().first())
    }
    @Test fun folderAndKindFiltersRunWithoutAdmittingDeletedOrForeignNotes() {
        val doc=note("doc"); val sticky=note("sticky",NoteEntryKind.STICKY)
        val data=AppData(notes=listOf(note("foreign",folder="b"),note("removed",deleted=1),doc,sticky))
        assertEquals(listOf(doc),data.sortedActiveNotebookDocuments("a"))
        assertEquals(listOf(sticky),data.sortedActiveStickyNotes("a"))
        assertTrue(data.sortedActiveNotes("absent").isEmpty())
        assertEquals(3,data.sortedActiveNotes().size)
    }
    @Test fun allSortModesPreservePinnedPriorityAndTimestampAndTitleTieBreaks() {
        val notes=listOf(note("b",created=20,updated=30),note("a",created=20,updated=40),note("c",created=10,updated=40),note("p",created=1,updated=1,pinned=true))
        val expected=mapOf(NoteSortMode.UPDATED_DESC to listOf("p","a","c","b"), NoteSortMode.CREATED_DESC to listOf("p","a","b","c"), NoteSortMode.CREATED_ASC to listOf("p","c","a","b"), NoteSortMode.TITLE_ASC to listOf("p","a","b","c"))
        for ((mode, ids) in expected) {
            val data=AppData(notes=notes).let { it.copy(notePreferences=it.notePreferences.copy(sortMode=mode)) }
            assertEquals(mode.name,ids,data.sortedActiveNotebookDocuments().map { it.id })
        }
        val ties=listOf(note("first",title="same"),note("second",title="SAME"))
        for(mode in NoteSortMode.values()) {
            val data=AppData(notes=ties).let { it.copy(notePreferences=it.notePreferences.copy(sortMode=mode)) }
            assertEquals(listOf("first","second"),data.sortedActiveNotes().map { it.id })
        }
    }
    @Test fun titleSortingUsesOnlyTheLockedPreviewUntilTheNoteIsUnlocked() {
        val sealed=note("sealed",title="private secret").copy(encryption=NoteEncryptionEnvelope(keyId="key"),encryptionUnlocked=false)
        val titles=noteSortTitles(listOf(sealed),NoteSortMode.TITLE_ASC)
        assertFalse(titles.single().contains("secret")); assertEquals(sealed.displayTitle(),titles.single())
        val removed=sealed.copy(deletedAtEpochMillis=1)
        assertTrue(AppData(notes=listOf(removed)).sortedActiveNotes().isEmpty())
    }
}
"###;
