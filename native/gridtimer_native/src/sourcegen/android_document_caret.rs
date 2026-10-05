// Keep the block-editor caret inside the viewport after newlines and IME/layout changes.
// Kotlin remains generated from Rust-owned production and regression-test sources.
pub const SCREEN: &str = "com/ofairyo/gridtimer/ui/NoteDocumentEditor.kt";
pub const POLICY_PATH: &str = "com/ofairyo/gridtimer/ui/DocumentCaretPolicy.kt";
pub const UI_PATH: &str = "com/ofairyo/gridtimer/ui/DocumentCaretVisibility.kt";
pub const TEST_PATH: &str = "com/ofairyo/gridtimer/ui/DocumentCaretPolicyTest.kt";

fn replace(source: &mut String, before: &str, after: &str) -> Result<(), String> {
    let count = source.matches(before).count();
    if count != 1 {
        return Err(format!("document caret anchor count {count}: {before}"));
    }
    *source = source.replacen(before, after, 1);
    Ok(())
}

pub fn render(path: &str, source: &str) -> Result<String, String> {
    if path != SCREEN {
        return Ok(source.to_owned());
    }
    let mut source = source.to_owned();
    for import in [
        "androidx.compose.foundation.lazy.LazyListState",
        "androidx.compose.foundation.relocation.bringIntoViewRequester",
        "androidx.compose.runtime.withFrameNanos",
        "androidx.compose.ui.layout.onSizeChanged",
    ] {
        let line = format!("import {import}\n");
        if !source.contains(&line) {
            replace(
                &mut source,
                "package com.ofairyo.gridtimer.ui\n",
                &format!("package com.ofairyo.gridtimer.ui\n{line}"),
            )?;
        }
    }
    replace(
        &mut source,
        "    val showingMarkdownPreview = markdownPreview && markdownPreviewText != null",
        LIST_FOCUS,
    )?;
    replace(
        &mut source,
        "        LazyColumn(\n            modifier = Modifier.weight(1f),\n            contentPadding = PaddingValues(start = 20.dp, top = 4.dp, end = 20.dp, bottom = 12.dp),",
        "        LazyColumn(\n            state = editorListState,\n            modifier = Modifier.weight(1f).onSizeChanged { editorViewportHeightPx = it.height },\n            contentPadding = PaddingValues(start = 20.dp, top = 4.dp, end = 20.dp, bottom = 12.dp),",
    )?;
    // Named entries keep the lazy-list prefix and focus index mapping explicit.
    replace(
        &mut source,
        "        item {\n            DocumentEditorTopBar(",
        "        item(key = \"document-editor-topbar\") {\n            DocumentEditorTopBar(",
    )?;
    replace(
        &mut source,
        "        item {\n            if (showingMarkdownPreview) {",
        "        item(key = \"document-editor-title\") {\n            if (showingMarkdownPreview) {",
    )?;
    replace(
        &mut source,
        "        item {\n            DocumentPropertyStrip(",
        "        item(key = \"document-editor-properties\") {\n            DocumentPropertyStrip(",
    )?;
    replace(
        &mut source,
        "        if (markdownPreviewText != null) {\n            item {",
        "        if (markdownPreviewText != null) {\n            item(key = \"document-editor-preview-switch\") {",
    )?;
    replace(
        &mut source,
        "                textFieldState = textFieldStates[block.id] ?: TextFieldValue(block.text),",
        "                textFieldState = textFieldStates[block.id] ?: TextFieldValue(block.text),\n                viewportHeightPx = editorViewportHeightPx,",
    )?;
    replace(
        &mut source,
        "    textFieldState: TextFieldValue,\n    active: Boolean,",
        "    textFieldState: TextFieldValue,\n    viewportHeightPx: Int,\n    active: Boolean,",
    )?;
    replace(
        &mut source,
        "    val focusRequester = remember(block.id) { FocusRequester() }",
        "    val focusWorkspaceKey = LocalNoteMediaWorkspaceKey.current\n    val focusRequester = remember(focusWorkspaceKey, note.id, block.id) { FocusRequester() }\n    val caretVisibility = rememberDocumentCaretVisibility(note.id, block.id, textFieldState, viewportHeightPx)",
    )?;
    replace(
        &mut source,
        "    LaunchedEffect(requestFocus) {\n        if (requestFocus && block.type == NoteBlockType.TEXT) {\n            focusRequester.requestFocus()",
        "    LaunchedEffect(focusWorkspaceKey, note.id, block.id, requestFocus) {\n        if (requestFocus && block.type == NoteBlockType.TEXT) {\n            withFrameNanos { }\n            focusRequester.requestFocus()",
    )?;
    replace(
        &mut source,
        "                            value = textFieldState,\n                            onValueChange = onTextChanged,",
        "                            value = textFieldState,\n                            onValueChange = onTextChanged,\n                            onTextLayout = { caretVisibility.layout = it },",
    )?;
    replace(
        &mut source,
        "                                .onFocusChanged { focusState ->\n                                    if (focusState.isFocused) {",
        "                                .onFocusChanged { focusState ->\n                                    caretVisibility.focused = focusState.isFocused\n                                    if (focusState.isFocused) {",
    )?;
    replace(
        &mut source,
        "                                Box(modifier = Modifier.fillMaxWidth()) {\n                                    if (textFieldState.text.isBlank()) {",
        "                                Box(modifier = Modifier.fillMaxWidth().bringIntoViewRequester(caretVisibility.requester)) {\n                                    if (textFieldState.text.isBlank()) {",
    )?;
    Ok(source)
}

const LIST_FOCUS: &str = r####"    val showingMarkdownPreview = markdownPreview && markdownPreviewText != null
    val editorListState = rememberSaveable(note.id, workspaceKey, saver = LazyListState.Saver) { LazyListState() }
    var editorViewportHeightPx by remember(note.id, workspaceKey) { mutableStateOf(0) }
    val pendingBlockIndex = if (pendingFocusBlockId == null) -1 else blocks.indexOfFirst {
        it.id == pendingFocusBlockId && it.type == NoteBlockType.TEXT
    }
    LaunchedEffect(note.id, workspaceKey, pendingFocusBlockId, pendingBlockIndex,
        showingMarkdownPreview, markdownPreviewText != null) {
        val targetId = pendingFocusBlockId ?: return@LaunchedEffect
        val targetIndex = documentFocusItemIndex(
            pendingBlockIndex, markdownPreviewText != null, showingMarkdownPreview,
            editorListState.layoutInfo.visibleItemsInfo.any { it.key == targetId }
        ) ?: return@LaunchedEffect
        // A newly split block may not be composed at all. Materialize it first;
        // its own focus/layout effect then reveals the precise caret rectangle.
        // Already visible blocks must NOT jump back to their top when long.
        editorListState.scrollToItem(targetIndex)
    }"####;

pub const POLICY_CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui

// Local layout policy only. It never changes text, selection, composition or stored notes.
internal data class DocumentCaretBounds(
    val left: Float,
    val top: Float,
    val right: Float,
    val bottom: Float
)

internal fun documentCaretOffset(
    text: String,
    layoutText: String,
    selectionEnd: Int,
    focused: Boolean,
    windowFocused: Boolean,
    viewportHeightPx: Int
): Int? {
    if (!focused || !windowFocused || viewportHeightPx <= 0 || text != layoutText) return null
    // Compose offsets use UTF-16, including surrogate pairs. Do not count code points here.
    return selectionEnd.coerceIn(0, text.length)
}

internal fun documentCaretRevealBounds(
    caret: DocumentCaretBounds,
    viewportHeightPx: Int,
    clearancePx: Float
): DocumentCaretBounds? {
    if (viewportHeightPx <= 0 || !clearancePx.isFinite() ||
        !caret.left.isFinite() || !caret.top.isFinite() ||
        !caret.right.isFinite() || !caret.bottom.isFinite() ||
        caret.top < 0f || caret.bottom <= caret.top || caret.right < caret.left
    ) return null
    val viewport = viewportHeightPx.toFloat()
    val lineHeight = caret.bottom - caret.top
    val clearance = clearancePx.coerceIn(0f, ((viewport - lineHeight) / 2f).coerceAtLeast(0f))
    val bottom = caret.bottom + clearance
    // A very large font can exceed a small landscape viewport. Never request a
    // rectangle taller than the scroll viewport; align the active line's bottom instead.
    val top = maxOf(0f, caret.top - clearance, bottom - viewport)
    return DocumentCaretBounds(caret.left, top, maxOf(caret.right, caret.left + 1f), bottom)
}

// Top bar, title and property strip precede the blocks. The preview switch is optional.
// Keep this mapping aligned with the named lazy-list prefix entries in the editor.
internal fun documentFocusItemIndex(
    blockIndex: Int,
    hasPreviewSwitch: Boolean,
    showingPreview: Boolean,
    alreadyVisible: Boolean
): Int? = if (blockIndex < 0 || showingPreview || alreadyVisible) null
    else 3 + (if (hasPreviewSwitch) 1 else 0) + blockIndex
"####;

pub const UI_CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.relocation.BringIntoViewRequester
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp

@OptIn(ExperimentalFoundationApi::class)
internal class DocumentCaretVisibility {
    val requester = BringIntoViewRequester()
    var focused by mutableStateOf(false)
    var layout by mutableStateOf<TextLayoutResult?>(null)
}

@OptIn(ExperimentalFoundationApi::class, ExperimentalLayoutApi::class)
@Composable
internal fun rememberDocumentCaretVisibility(
    noteId: String,
    blockId: String,
    value: TextFieldValue,
    viewportHeightPx: Int
): DocumentCaretVisibility {
    val workspaceKey = LocalNoteMediaWorkspaceKey.current
    val state = remember(workspaceKey, noteId, blockId) { DocumentCaretVisibility() }
    val density = LocalDensity.current
    val imeBottomPx = WindowInsets.ime.getBottom(density)
    val window = LocalWindowInfo.current
    val latestValue by rememberUpdatedState(value)
    val latestViewportHeight by rememberUpdatedState(viewportHeightPx)
    val clearancePx = with(density) { 8.dp.toPx() }

    // Do not key this to list scroll offset or global position: manual scrolling
    // must not snap back to a focused paragraph. New edits/caret moves, new text
    // layout, focus and viewport/IME changes are the only relocation triggers.
    LaunchedEffect(
        state, state.focused, window.isWindowFocused, value, state.layout,
        viewportHeightPx, imeBottomPx, clearancePx
    ) {
        if (!state.focused || !window.isWindowFocused) return@LaunchedEffect
        // A newline may create a new lazy item, or grow an existing field. Let
        // measurement AND placement complete before reading its local cursor rect.
        withFrameNanos { }
        val layout = state.layout ?: return@LaunchedEffect
        val current = latestValue
        val offset = documentCaretOffset(
            current.text, layout.layoutInput.text.text, current.selection.end,
            state.focused, window.isWindowFocused, latestViewportHeight
        ) ?: return@LaunchedEffect
        val caret = layout.getCursorRect(offset)
        val target = documentCaretRevealBounds(
            DocumentCaretBounds(caret.left, caret.top, caret.right, caret.bottom),
            latestViewportHeight, clearancePx
        ) ?: return@LaunchedEffect
        // The requester is on the unpadded decoration box, in text-layout
        // coordinates. Reveal the caret, not the whole potentially huge block.
        // LaunchedEffect propagates cancellation when a newer edit/focus replaces
        // this request; no delayed jobs can scroll an old block back into view.
        state.requester.bringIntoView(Rect(target.left, target.top, target.right, target.bottom))
    }
    return state
}
"####;

pub const TEST_CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui

import org.junit.Assert.*
import org.junit.Test

class DocumentCaretPolicyTest {
    private fun offset(
        text: String = "段落\n", layout: String = text, end: Int = text.length,
        focused: Boolean = true, window: Boolean = true, height: Int = 280
    ) = documentCaretOffset(text, layout, end, focused, window, height)

    @Test fun newlineWaitsForTheMatchingTextLayout() {
        assertNull(offset(layout = "段落"))
        assertEquals(3, offset())
    }
    @Test fun equalLengthReplacementDoesNotUseAnOldLayout() {
        assertNull(offset(text = "甲乙", layout = "丙丁"))
        assertEquals(2, offset(text = "甲乙"))
    }
    @Test fun emptyNewBlockCanRevealItsFirstCaret() {
        assertEquals(0, offset(text = "", end = 0))
        assertNotNull(documentCaretRevealBounds(DocumentCaretBounds(0f, 0f, 0f, 26f), 280, 8f))
    }
    @Test fun unfocusedOrCoveredWindowCannotMoveTheDocument() {
        assertNull(offset(focused = false))
        assertNull(offset(window = false))
    }
    @Test fun layoutMustHaveAUsableViewport() {
        assertNull(offset(height = 0))
        assertNull(offset(height = -1))
        assertNull(documentCaretRevealBounds(DocumentCaretBounds(0f, 0f, 2f, 26f), 0, 8f))
    }
    @Test fun selectionUsesUtf16AndTheMovingSelectionEnd() {
        assertEquals(3, offset(text = "中\uD83D\uDE00文", end = 3))
        assertEquals(1, offset(text = "中文段落", end = 1))
        assertEquals(0, offset(end = -1))
        assertEquals(3, offset(end = 999))
    }
    @Test fun longBlockRevealsOnlyTheActiveLine() {
        val rect = documentCaretRevealBounds(DocumentCaretBounds(12f, 2000f, 14f, 2026f), 280, 8f)!!
        assertEquals(1992f, rect.top, 0.001f)
        assertEquals(2034f, rect.bottom, 0.001f)
        assertEquals(42f, rect.bottom - rect.top, 0.001f)
    }
    @Test fun clearanceUsesPixelsFromTheCurrentDensity() {
        val caret = DocumentCaretBounds(0f, 100f, 2f, 126f)
        assertEquals(134f, documentCaretRevealBounds(caret, 280, 8f)!!.bottom, 0.001f)
        assertEquals(150f, documentCaretRevealBounds(caret, 840, 24f)!!.bottom, 0.001f)
    }
    @Test fun firstLineNeverRequestsNegativeTop() {
        assertEquals(0f, documentCaretRevealBounds(DocumentCaretBounds(0f, 0f, 2f, 26f), 280, 8f)!!.top, 0.001f)
    }
    @Test fun largeFontAndSmallLandscapeViewportHaveBoundedTargets() {
        val caret = DocumentCaretBounds(0f, 100f, 2f, 220f)
        val rect = documentCaretRevealBounds(caret, 64, 24f)!!
        assertEquals(64f, rect.bottom - rect.top, 0.001f)
        assertEquals(220f, rect.bottom, 0.001f)
        assertEquals(0f, documentCaretRevealBounds(DocumentCaretBounds(0f, 0f, 2f, 26f), 30, -8f)!!.top, 0.001f)
    }
    @Test fun invalidCoordinatesCannotStartAScroll() {
        for (caret in listOf(
            DocumentCaretBounds(Float.NaN, 0f, 2f, 26f),
            DocumentCaretBounds(0f, Float.POSITIVE_INFINITY, 2f, 26f),
            DocumentCaretBounds(0f, -1f, 2f, 26f),
            DocumentCaretBounds(0f, 26f, 2f, 26f),
            DocumentCaretBounds(2f, 0f, 0f, 26f)
        )) assertNull(documentCaretRevealBounds(caret, 280, 8f))
        assertNull(documentCaretRevealBounds(DocumentCaretBounds(0f, 0f, 2f, 26f), 280, Float.NaN))
    }
    @Test fun offscreenNewlineAndInsertionIncludeAllPrecedingBlocks() {
        assertEquals(3, documentFocusItemIndex(0, false, false, false))
        assertEquals(4, documentFocusItemIndex(0, true, false, false))
        assertEquals(24, documentFocusItemIndex(20, true, false, false))
    }
    @Test fun visibleBlockNeverJumpsToItsTop() {
        assertNull(documentFocusItemIndex(20, true, false, true))
    }
    @Test fun removedBlockOrPreviewModeCannotStealFocus() {
        assertNull(documentFocusItemIndex(-1, true, false, false))
        assertNull(documentFocusItemIndex(2, true, true, false))
    }
    @Test fun consecutiveLinesStayWithinTheAvailableHeight() {
        for (line in 0..200) for (height in listOf(32, 64, 180, 280, 640)) {
            val bottom = line * 26f + 26f
            val rect = documentCaretRevealBounds(DocumentCaretBounds(0f, line * 26f, 2f, bottom), height, 8f)!!
            assertTrue(rect.top >= 0f)
            assertTrue(rect.bottom >= bottom)
            assertTrue(rect.bottom - rect.top <= height.toFloat())
        }
    }
}
"####;

#[cfg(test)]
mod tests {
    use super::*;

    fn editor() -> &'static str {
        super::super::kotlin_sources::SOURCES
            .iter()
            .find(|source| source.path == SCREEN)
            .expect("actual document editor template")
            .contents
    }

    #[test]
    fn drift_or_duplicate_hooks_fail_instead_of_silently_skipping_fix() {
        let missing_hook =
            editor().replace("onValueChange = onTextChanged,", "onValueChange = changed,");
        assert!(render(SCREEN, &missing_hook).is_err());
        let duplicate = format!(
            "{}\n    val showingMarkdownPreview = markdownPreview && markdownPreviewText != null",
            editor()
        );
        assert!(render(SCREEN, &duplicate).is_err());
        assert!(render(SCREEN, &render(SCREEN, editor()).unwrap()).is_err());
    }
}
