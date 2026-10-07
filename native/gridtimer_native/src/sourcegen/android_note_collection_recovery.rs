// v2.23.2.21 - Make empty sticky folder views recoverable and test filter resets.
// v2.23.2 - Recover complete knowledge lists when native grouping is incomplete.
// Android implementation and its JVM state tests are authored in this Rust generator.

const BRIDGE_PATH: &str = "com/ofairyo/gridtimer/core/NativeOptimizerBridge.kt";

pub fn render(path: &str, source: &str) -> Result<String, String> {
    if path != BRIDGE_PATH {
        return Ok(source.to_owned());
    }
    let mut rendered = source.to_owned();
    replace_once(
        &mut rendered,
        "            ?.let(::parseNativeNoteCollectionSections)",
        "            .let { values -> recoverNoteCollectionPartition(values, updatedEpochDays, pinnedFlags) }",
    )?;
    let old_order_guard =
        "            ?.takeIf { indices -> indices.size == count && indices.all { it in 0 until count } }";
    if rendered.matches(old_order_guard).count() != 2 {
        return Err("expected active and trash note order guards".to_owned());
    }
    rendered = rendered.replace(
        old_order_guard,
        "            ?.takeIf { indices -> isCompleteNoteOrder(indices, count) }",
    );
    let parser_start = rendered
        .find("    private fun parseNativeNoteCollectionSections(")
        .ok_or("missing old note collection parser")?;
    let parser_end = rendered[parser_start..]
        .find("    private fun Long.toValidSessionIndex(")
        .map(|offset| parser_start + offset)
        .ok_or("missing old note collection parser boundary")?;
    rendered.replace_range(parser_start..parser_end, "");
    Ok(rendered)
}

fn replace_once(source: &mut String, old: &str, new: &str) -> Result<(), String> {
    if source.matches(old).count() != 1 {
        return Err(format!(
            "note collection recovery anchor is not unique: {old}"
        ));
    }
    *source = source.replacen(old, new, 1);
    Ok(())
}

pub const HELPER_PATH: &str = "com/ofairyo/gridtimer/core/NoteCollectionPartition.kt";
pub const HELPER_CONTENTS: &str = r####"package com.ofairyo.gridtimer.core

// An optimization may reorder a list, but cannot remove or duplicate its records.
internal fun isCompleteNoteOrder(indices: IntArray, count: Int): Boolean {
    if (count < 0 || indices.size != count) return false
    val seen = BooleanArray(count)
    for (index in indices) {
        if (index !in 0 until count || seen[index]) return false
        seen[index] = true
    }
    return true
}

// Decode against the exact source arrays. Counts are checked before allocation or
// integer conversion; every source index must occur once in its correct group.
internal fun decodeNoteCollectionPartition(
    values: LongArray,
    updatedEpochDays: LongArray,
    pinnedFlags: BooleanArray
): List<NativeNoteCollectionSection>? {
    val count = updatedEpochDays.size
    if (pinnedFlags.size != count || values.isEmpty()) return null
    if (count == 0) return emptyList<NativeNoteCollectionSection>().takeIf {
        values.size == 1 && values[0] == 0L
    }
    val sectionCount = values[0]
    if (sectionCount !in 0L..count.toLong()) return null
    val headersSize = 1L + 3L * sectionCount
    if (values.size.toLong() !in headersSize..(headersSize + count.toLong())) return null
    val minimumDay = java.time.LocalDate.MIN.toEpochDay()
    val maximumDay = java.time.LocalDate.MAX.toEpochDay()
    if (updatedEpochDays.any { it !in minimumDay..maximumDay }) return null

    val sections = ArrayList<NativeNoteCollectionSection>(minOf(sectionCount.toInt(), 64))
    val seen = BooleanArray(count)
    var offset = 1
    var previousDay: Long? = null
    var covered = 0
    repeat(sectionCount.toInt()) { groupIndex ->
        if (values.size - offset < 3) return null
        val kind = values[offset]
        val day = values[offset + 1]
        val groupCount = values[offset + 2]
        offset += 3
        if (groupCount !in 1L..count.toLong() || groupCount > (values.size - offset).toLong()) return null
        when (kind) {
            0L -> if (groupIndex != 0 || day != Long.MIN_VALUE) return null
            1L -> {
                if (day !in minimumDay..maximumDay || previousDay?.let { day >= it } == true) return null
                previousDay = day
            }
            else -> return null
        }
        val indices = IntArray(groupCount.toInt())
        var previousIndex = -1
        for (position in indices.indices) {
            val encodedIndex = values[offset + position]
            if (encodedIndex !in 0L until count.toLong()) return null
            val index = encodedIndex.toInt()
            if (seen[index] || index <= previousIndex) return null
            if (kind == 0L) {
                if (!pinnedFlags[index]) return null
            } else if (pinnedFlags[index] || updatedEpochDays[index] != day) return null
            seen[index] = true
            covered++
            indices[position] = index
            previousIndex = index
        }
        offset += indices.size
        sections += NativeNoteCollectionSection(kind.toInt(), day, indices)
    }
    return sections.takeIf { offset == values.size && covered == count }
}

// Native allocation/read failures can return no groups. Rebuild from the same
// immutable inputs in that case, so a failed optimization cannot hide pages.
internal fun recoverNoteCollectionPartition(
    values: LongArray?,
    updatedEpochDays: LongArray,
    pinnedFlags: BooleanArray
): List<NativeNoteCollectionSection>? {
    if (updatedEpochDays.size != pinnedFlags.size) return null
    val minimumDay = java.time.LocalDate.MIN.toEpochDay()
    val maximumDay = java.time.LocalDate.MAX.toEpochDay()
    if (updatedEpochDays.any { it !in minimumDay..maximumDay }) return null
    values?.let { decodeNoteCollectionPartition(it, updatedEpochDays, pinnedFlags) }?.let { return it }
    val pinned = ArrayList<Int>()
    val byDay = linkedMapOf<Long, MutableList<Int>>()
    for (index in updatedEpochDays.indices) {
        if (pinnedFlags[index]) pinned.add(index)
        else byDay.getOrPut(updatedEpochDays[index]) { ArrayList() }.add(index)
    }
    return buildList {
        if (pinned.isNotEmpty()) add(NativeNoteCollectionSection(0, Long.MIN_VALUE, pinned.toIntArray()))
        byDay.entries.sortedByDescending { it.key }.forEach { (day, indices) ->
            add(NativeNoteCollectionSection(1, day, indices.toIntArray()))
        }
    }
}

internal data class NoteCollectionFilterState(
    val query: String,
    val allQuickFilter: Boolean,
    val selectedFolderId: String?
) {
    val hasActiveFilters: Boolean
        get() = query.isNotBlank() || !allQuickFilter || selectedFolderId != null

    fun showAll(): NoteCollectionFilterState = copy(
        query = "",
        allQuickFilter = true,
        selectedFolderId = null
    )
}
"####;

pub const TEST_PATH: &str = "com/ofairyo/gridtimer/core/NoteCollectionRecoveryTest.kt";
pub const TEST_CONTENTS: &str = r####"package com.ofairyo.gridtimer.core

import org.junit.Assert.*
import org.junit.Test

class NoteCollectionRecoveryTest {
    private val days = longArrayOf(30L, 20L, 30L, 10L)
    private val pinned = booleanArrayOf(false, true, false, false)
    private val complete = longArrayOf(3L, 0L, Long.MIN_VALUE, 1L, 1L, 1L, 30L, 2L, 0L, 2L, 1L, 10L, 1L, 3L)

    private fun indices(groups: List<NativeNoteCollectionSection>): List<Int> =
        groups.flatMap { it.noteIndices.toList() }

    private fun assertFullRecovery(raw: LongArray?) {
        val recovered = recoverNoteCollectionPartition(raw, days, pinned)!!
        assertEquals(listOf(1, 0, 2, 3), indices(recovered))
        assertEquals(listOf(0, 1, 1), recovered.map { it.kindCode })
        assertEquals(listOf(Long.MIN_VALUE, 30L, 10L), recovered.map { it.epochDay })
    }

    @Test fun validNativeGroupsRetainPinnedPriorityDayOrderAndEveryPage() {
        val groups = decodeNoteCollectionPartition(complete, days, pinned)!!
        assertEquals(listOf(1, 0, 2, 3), indices(groups))
        assertEquals(listOf(Long.MIN_VALUE, 30L, 10L), groups.map { it.epochDay })
        assertFullRecovery(complete)
        assertEquals(emptyList<NativeNoteCollectionSection>(), decodeNoteCollectionPartition(longArrayOf(0L), longArrayOf(), booleanArrayOf()))
    }

    @Test fun emptyFailedOrPartialNativeResultsCannotHideSourcePages() {
        for (raw in listOf<LongArray?>(null, longArrayOf(), longArrayOf(0L), complete.copyOf(complete.size - 1), longArrayOf(1L, 1L, 30L, 1L, 0L))) {
            if (raw != null) assertNull(decodeNoteCollectionPartition(raw, days, pinned))
            assertFullRecovery(raw)
        }
    }

    @Test fun duplicateMissingForeignOrOverflowIndicesTriggerCompleteRecovery() {
        for (index in listOf(0L, 4L, -1L, Long.MAX_VALUE, Int.MAX_VALUE.toLong() + 1L)) {
            val damaged = complete.copyOf().also { it[9] = index }
            assertNull(decodeNoteCollectionPartition(damaged, days, pinned))
            assertFullRecovery(damaged)
        }
        val extra = complete + 0L
        assertNull(decodeNoteCollectionPartition(extra, days, pinned))
        assertFullRecovery(extra)
    }

    @Test fun invalidCountsAndEpochDaysCannotAllocateFromUntrustedMetadata() {
        for (count in listOf(-1L, 0L, 5L, Long.MAX_VALUE, Int.MAX_VALUE.toLong() + 1L)) {
            val wrongHeader = complete.copyOf().also { it[0] = count }
            assertNull(decodeNoteCollectionPartition(wrongHeader, days, pinned))
            assertFullRecovery(wrongHeader)
            val wrongGroup = complete.copyOf().also { it[3] = count }
            assertNull(decodeNoteCollectionPartition(wrongGroup, days, pinned))
            assertFullRecovery(wrongGroup)
        }
        val wrongDay = complete.copyOf().also { it[6] = Long.MAX_VALUE }
        assertNull(decodeNoteCollectionPartition(wrongDay, days, pinned))
        assertFullRecovery(wrongDay)
        assertNull(recoverNoteCollectionPartition(null, longArrayOf(Long.MAX_VALUE), booleanArrayOf(false)))
        assertNull(recoverNoteCollectionPartition(null, days, booleanArrayOf(false)))
    }

    @Test fun incorrectPinnedMembershipKindOrOrderingFallsBackToTheSourceContract() {
        val corruptions = listOf(
            complete.copyOf().also { it[1] = 2L },
            complete.copyOf().also { it[4] = 0L },
            complete.copyOf().also { it[8] = 2L; it[9] = 0L },
            complete.copyOf().also { it[11] = 30L },
            complete.copyOf().also { it[2] = 20L }
        )
        for (damaged in corruptions) {
            assertNull(decodeNoteCollectionPartition(damaged, days, pinned))
            assertFullRecovery(damaged)
        }
    }

    @Test fun sortMustBeAPermutationAndCannotDuplicateOrLosePages() {
        assertTrue(isCompleteNoteOrder(intArrayOf(2, 0, 3, 1), 4))
        assertTrue(isCompleteNoteOrder(intArrayOf(), 0))
        assertFalse(isCompleteNoteOrder(intArrayOf(), 4))
        assertFalse(isCompleteNoteOrder(intArrayOf(0, 1, 2), 4))
        assertFalse(isCompleteNoteOrder(intArrayOf(0, 1, 1, 3), 4))
        assertFalse(isCompleteNoteOrder(intArrayOf(0, 1, 2, 4), 4))
        assertFalse(isCompleteNoteOrder(intArrayOf(-1, 1, 2, 3), 4))
        assertFalse(isCompleteNoteOrder(intArrayOf(), -1))
    }

    @Test fun selectedFolderEmptyStateCanReturnToAllStickyNotes() {
        val scoped = NoteCollectionFilterState("", true, "AiProbe07136f6b")
        assertTrue(scoped.hasActiveFilters)
        assertEquals(NoteCollectionFilterState("", true, null), scoped.showAll())
    }

    @Test fun searchAndQuickFilterEmptyStatesAlsoExposeRecovery() {
        assertTrue(NoteCollectionFilterState("old note", true, null).hasActiveFilters)
        assertTrue(NoteCollectionFilterState("", false, null).hasActiveFilters)
        assertFalse(NoteCollectionFilterState("", true, null).hasActiveFilters)
    }
}
"####;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_renderer_targets_only_the_bridge() {
        let bridge = super::super::kotlin_sources::SOURCES
            .iter()
            .find(|source| source.path == BRIDGE_PATH)
            .unwrap();
        let rendered = render(bridge.path, bridge.contents).unwrap();
        assert!(rendered
            .contains("recoverNoteCollectionPartition(values, updatedEpochDays, pinnedFlags)"));
        assert_eq!(
            rendered
                .matches("?.takeIf { indices -> isCompleteNoteOrder(indices, count) }")
                .count(),
            2
        );
        assert!(!rendered.contains("parseNativeNoteCollectionSections("));
        assert_eq!(render("unrelated.kt", "sentinel").unwrap(), "sentinel");
    }
}
