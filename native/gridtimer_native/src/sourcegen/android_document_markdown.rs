//! Keep independent canvas paragraphs distinct in the read-only Markdown preview.
//! Neither stored blocks nor the shared AI-answer renderer are changed.

const EDITOR_PATH: &str = "com/ofairyo/gridtimer/ui/NoteDocumentEditor.kt";
pub const TEST_PATH: &str = "com/ofairyo/gridtimer/ui/DocumentMarkdownBlocksTest.kt";

pub fn render(path: &str, source: &str) -> Result<String, String> {
    if path != EDITOR_PATH {
        return Ok(source.to_owned());
    }
    let count = source.matches(OLD_JOIN).count();
    if count != 1 || source.contains("internal fun documentMarkdownFromBlocks(") {
        return Err(format!(
            "document Markdown paragraph projection: expected one original join, found {count}"
        ));
    }
    Ok(format!("{}\n{HELPERS}", source.replacen(OLD_JOIN, NEW_JOIN, 1)))
}

const OLD_JOIN: &str = r####"    return draftBlocks.joinToString("\n") { block ->
        textFieldStates[block.id]?.text ?: block.text
    }.takeIf(String::isNotBlank)"####;

const NEW_JOIN: &str = r####"    return documentMarkdownFromBlocks(draftBlocks.map { block ->
        textFieldStates[block.id]?.text ?: block.text
    }).takeIf(String::isNotBlank)"####;

const HELPERS: &str = r####"

/** Preview-only projection. Never normalizes or writes the editor's block text. */
internal fun documentMarkdownFromBlocks(blocks: List<String>): String {
    if (blocks.size < 2) return blocks.firstOrNull().orEmpty()
    val context = DocumentMarkdownContext()
    return buildString {
        blocks.forEachIndexed { index, text ->
            if (index > 0) {
                // A canvas paragraph is not a CommonMark soft line break. Keep a
                // single newline only while continuing an explicit Markdown structure.
                append(if (context.continuesWith(text)) "\n" else "\n\n")
            }
            append(text)
            context.read(text)
        }
    }
}

private class DocumentMarkdownContext {
    private var fence: Pair<Char, Int>? = null
    private var mathEnd: String? = null
    private var previousLine = ""
    private var table = false

    fun continuesWith(nextBlock: String): Boolean {
        if (fence != null || mathEnd != null) return true
        val next = nextBlock.lineSequence().firstOrNull().orEmpty()
        if (previousLine.isBlank() || next.isBlank()) return false
        val leftCells = tableCells(previousLine)
        val rightCells = tableCells(next)
        if (leftCells != null && rightCells != null &&
            rightCells.size == leftCells.size && rightCells.all { TABLE_DELIMITER.matches(it.trim()) }) return true
        if (table && rightCells != null) return true
        // Retain tight lists and indented continuation lines, but never make a
        // following independent prose block a lazy list/quote continuation.
        if (LIST_ITEM.containsMatchIn(previousLine) &&
            (LIST_ITEM.containsMatchIn(next) || next.startsWith("  ") || next.startsWith("\t"))) return true
        if (previousLine.trimStart().startsWith(">") && next.trimStart().startsWith(">")) return true
        if (isIndented(previousLine) && isIndented(next)) return true
        return SETEXT.matches(next) && !previousLine.trimStart().startsWith("#")
    }

    fun read(text: String) {
        text.lineSequence().forEach { line ->
            val activeFence = fence
            if (activeFence != null) {
                val marker = fenceMarker(line)
                if (marker != null && marker.first == activeFence.first && marker.second >= activeFence.second &&
                    line.trimStart().drop(marker.second).isBlank()) fence = null
                previousLine = line
                table = false
                return@forEach
            }
            val marker = if (mathEnd == null) fenceMarker(line) else null
            if (marker != null && (marker.first != '`' || !line.trimStart().drop(marker.second).contains('`'))) {
                fence = marker
                previousLine = line
                table = false
                return@forEach
            }
            readMath(line)
            val cells = tableCells(line)
            table = when {
                line.isBlank() || cells == null -> false
                cells.all { TABLE_DELIMITER.matches(it.trim()) } -> {
                    val header = tableCells(previousLine)
                    header != null && header.size == cells.size
                }
                else -> table
            }
            previousLine = line
        }
    }

    private fun readMath(line: String) {
        var index = 0
        var codeTicks = 0
        while (index < line.length) {
            if (mathEnd == null && line[index] == '`' && !escaped(line, index)) {
                val length = line.drop(index).takeWhile { it == '`' }.length
                if (codeTicks == 0) codeTicks = length else if (codeTicks == length) codeTicks = 0
                index += length
                continue
            }
            val end = mathEnd
            if (codeTicks == 0 && !escaped(line, index)) {
                if (end != null && line.startsWith(end, index)) {
                    mathEnd = null
                    index += end.length
                    continue
                }
                if (end == null) {
                    val close = when {
                        line.startsWith("\$\$", index) -> "\$\$"
                        line.startsWith("\\[", index) -> "\\]"
                        line.startsWith("\\(", index) -> "\\)"
                        else -> null
                    }
                    if (close != null) {
                        mathEnd = close
                        index += 2
                        continue
                    }
                }
            }
            index++
        }
    }

    private fun fenceMarker(line: String): Pair<Char, Int>? {
        val indent = line.takeWhile { it == ' ' }.length
        if (indent > 3) return null
        val text = line.drop(indent)
        val character = text.firstOrNull() ?: return null
        if (character != '`' && character != '~') return null
        val count = text.takeWhile { it == character }.length
        return if (count >= 3) character to count else null
    }

    private fun tableCells(line: String): List<String>? {
        val separators = line.indices.filter { line[it] == '|' && !escaped(line, it) }
        if (separators.isEmpty()) return null
        val cells = mutableListOf<String>()
        var start = 0
        separators.forEach { index -> cells += line.substring(start, index); start = index + 1 }
        cells += line.substring(start)
        if (cells.first().isBlank()) cells.removeAt(0)
        if (cells.isNotEmpty() && cells.last().isBlank()) cells.removeAt(cells.lastIndex)
        return cells.takeIf { it.isNotEmpty() }
    }

    private fun escaped(text: String, index: Int): Boolean {
        var backslashes = 0
        var cursor = index - 1
        while (cursor >= 0 && text[cursor] == '\\') { backslashes++; cursor-- }
        return backslashes % 2 != 0
    }

    private fun isIndented(line: String): Boolean = line.startsWith("    ") || line.startsWith("\t")

    companion object {
        private val TABLE_DELIMITER = Regex(":?-+:?")
        private val LIST_ITEM = Regex("^ {0,3}(?:[-+*]|[0-9]{1,9}[.)])[ \\t]+")
        private val SETEXT = Regex(" {0,3}(?:=+|-+)[ \\t]*")
    }
}
"####;

pub const TEST_CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui

import org.junit.Assert.*
import org.junit.Test
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import com.ofairyo.gridtimer.data.NoteBlock
import com.ofairyo.gridtimer.data.NoteBlockType
import com.ofairyo.gridtimer.data.NoteDocument

class DocumentMarkdownBlocksTest {
    @Test fun independentParagraphsHaveBlankLineBoundaries() {
        assertEquals("第一段。\n\n第二段。\n\n第三段。", documentMarkdownFromBlocks(listOf("第一段。", "第二段。", "第三段。")))
    }
    @Test fun screenshotShapeKeepsHeadingsAndFiveSeparateParagraphs() {
        val blocks = listOf("# 第一节", "段落甲。", "段落乙。", "段落丙。", "段落丁。", "段落戊。", "# 第二节", "后文。")
        assertEquals(blocks.joinToString("\n\n"), documentMarkdownFromBlocks(blocks))
    }
    @Test fun singleImportedMarkdownBlockIsByteForByteUnchanged() {
        val source = "# 标题\r\n\r\n一行\r\n另一行  \r\n\r\n```text\r\na\r\n\r\nb\r\n```\r\n\r\n\$\$x^2\$\$\r\n"
        assertEquals(source, documentMarkdownFromBlocks(listOf(source)))
    }
    @Test fun internalSoftAndHardBreaksAreNotGloballyRewritten() {
        assertEquals("第一行\n第二行  \n第三行\n\n下一段", documentMarkdownFromBlocks(listOf("第一行\n第二行  \n第三行", "下一段")))
    }
    @Test fun emptyInputAndExplicitBlankBlocksAreSafe() {
        assertEquals("", documentMarkdownFromBlocks(emptyList()))
        assertEquals("", documentMarkdownFromBlocks(listOf("")))
        assertEquals("甲\n\n\n\n乙", documentMarkdownFromBlocks(listOf("甲", "", "乙")))
    }
    @Test fun unicodeAndTrailingWhitespaceArePreserved() {
        assertEquals("🙂 e\u0301 中文  \n\n𠮷\t", documentMarkdownFromBlocks(listOf("🙂 e\u0301 中文  ", "𠮷\t")))
    }
    @Test fun existingBlankLinesInsideBlocksStayUnchanged() {
        assertEquals("甲\n\n乙\n\n丙\n\n丁", documentMarkdownFromBlocks(listOf("甲\n\n乙", "丙\n\n丁")))
    }
    @Test fun splitBacktickFencePreservesCodeLinesAndBlankLines() {
        val code = listOf("```kotlin", "val a = 1", "", "println(a)", "```")
        assertEquals(code.joinToString("\n") + "\n\n后文", documentMarkdownFromBlocks(code + "后文"))
    }
    @Test fun shorterFenceDoesNotPrematurelyCloseCode() {
        val code = listOf("````text", "```", "甲", "````")
        assertEquals(code.joinToString("\n") + "\n\n乙", documentMarkdownFromBlocks(code + "乙"))
    }
    @Test fun tildeFenceAndMismatchedClosingCharacterArePreserved() {
        val code = listOf("~~~text", "```", "x", "~~~~")
        assertEquals(code.joinToString("\n"), documentMarkdownFromBlocks(code))
    }
    @Test fun completeFenceInsideOneBlockDoesNotCaptureFollowingParagraph() {
        assertEquals("```\nx\n```\n\n甲\n\n乙", documentMarkdownFromBlocks(listOf("```\nx\n```", "甲", "乙")))
    }
    @Test fun fencedCodeCanContainMathDelimitersLiterally() {
        assertEquals("```\n\$\$\n\\[\n```\n\n甲\n\n乙", documentMarkdownFromBlocks(listOf("```", "\$\$", "\\[", "```", "甲", "乙")))
    }
    @Test fun dollarMathAcrossBlocksDoesNotGainBlankLines() {
        assertEquals("\$\$\nx^2 +\ny^2\n\$\$\n\n甲\n\n乙", documentMarkdownFromBlocks(listOf("\$\$", "x^2 +", "y^2", "\$\$", "甲", "乙")))
    }
    @Test fun bracketAndParenthesisMathContinueUntilTheirMatchingEnd() {
        assertEquals("\\[\nx+y\n\\]\n\n甲", documentMarkdownFromBlocks(listOf("\\[", "x+y", "\\]", "甲")))
        assertEquals("\\(x+\ny\\)\n\n甲", documentMarkdownFromBlocks(listOf("\\(x+", "y\\)", "甲")))
    }
    @Test fun closedInlineMathDoesNotJoinIndependentParagraphs() {
        assertEquals("公式 \$\$x^2\$\$\n\n甲\n\n乙", documentMarkdownFromBlocks(listOf("公式 \$\$x^2\$\$", "甲", "乙")))
    }
    @Test fun escapedOrInlineCodeMathMarkersDoNotOpenMath() {
        assertEquals("字面 \\\$(not math)\n\n甲", documentMarkdownFromBlocks(listOf("字面 \\\$(not math)", "甲")))
        assertEquals("`\$\$`\n\n甲\n\n乙", documentMarkdownFromBlocks(listOf("`\$\$`", "甲", "乙")))
        assertEquals("\\\$\$\n\n甲", documentMarkdownFromBlocks(listOf("\\\$\$", "甲")))
    }
    @Test fun tableHeaderDelimiterAndRowsStayContiguous() {
        val rows = listOf("| 名称 | 数值 |", "| :--- | ---: |", "| 甲 | 1 |", "| 乙 | 2 |")
        assertEquals(rows.joinToString("\n") + "\n\n说明", documentMarkdownFromBlocks(rows + "说明"))
    }
    @Test fun tableWithoutOuterPipesAndEscapedPipeWork() {
        val rows = listOf("名称 | 数值", "--- | ---", "甲\\|乙 | 1")
        assertEquals(rows.joinToString("\n"), documentMarkdownFromBlocks(rows))
    }
    @Test fun ordinaryPipeTextAndMismatchedTableDelimiterRemainParagraphs() {
        assertEquals("甲 | 乙\n\n丙 | 丁", documentMarkdownFromBlocks(listOf("甲 | 乙", "丙 | 丁")))
        assertEquals("甲 | 乙\n\n| --- |", documentMarkdownFromBlocks(listOf("甲 | 乙", "| --- |")))
    }
    @Test fun orderedUnorderedAndTaskListsKeepTheirMarkers() {
        for (items in listOf(listOf("1. 甲", "2. 乙"), listOf("- 甲", "- 乙"), listOf("- [ ] 甲", "- [x] 乙"))) {
            assertEquals(items.joinToString("\n") + "\n\n正文", documentMarkdownFromBlocks(items + "正文"))
        }
    }
    @Test fun quoteAndIndentedCodeStayContiguousButEndBeforeProse() {
        assertEquals("> 甲\n> 乙\n\n正文", documentMarkdownFromBlocks(listOf("> 甲", "> 乙", "正文")))
        assertEquals("    a\n    b\n\n正文", documentMarkdownFromBlocks(listOf("    a", "    b", "正文")))
    }
    @Test fun setextHeadingBoundaryIsNotBroken() {
        assertEquals("标题\n===\n\n正文", documentMarkdownFromBlocks(listOf("标题", "===", "正文")))
    }
    @Test fun eachProjectionStartsWithFreshSyntaxState() {
        documentMarkdownFromBlocks(listOf("```", "unclosed"))
        documentMarkdownFromBlocks(listOf("\$\$", "unclosed"))
        assertEquals("甲\n\n乙", documentMarkdownFromBlocks(listOf("甲", "乙")))
    }
    @Test fun manyParagraphsAreNotTruncatedOrMutated() {
        val blocks = (0 until 500).map { "段落 $it" }.toMutableList()
        val original = blocks.toList()
        assertEquals(blocks.joinToString("\n\n"), documentMarkdownFromBlocks(blocks))
        assertEquals(original, blocks)
    }
    @Test fun actualPreviewUsesLiveDraftAndKeepsOriginalBlocks() {
        val first = NoteBlock(id = "one", type = NoteBlockType.TEXT, text = "已存正文")
        val second = NoteBlock(id = "two", type = NoteBlockType.TEXT, text = "下一段")
        val blocks = listOf(first, second)
        val document = NoteDocument(markdownEnabled = true, blocks = blocks)
        val live = TextFieldValue("未提交正文", composition = TextRange(0, 2))
        assertEquals("未提交正文\n\n下一段", documentMarkdownPreviewText(document, blocks, mapOf(first.id to live)))
        assertEquals("已存正文", document.blocks.first().text)
        assertEquals(listOf("one", "two"), document.blocks.map { it.id })
        assertEquals(TextRange(0, 2), live.composition)
    }
    @Test fun actualPreviewRetainsEligibilityAndEmptyDraftRules() {
        val blocks = listOf(NoteBlock(id = "one", type = NoteBlockType.TEXT, text = "正文"))
        val document = NoteDocument(markdownEnabled = true, blocks = blocks)
        assertNull(documentMarkdownPreviewText(document.copy(markdownEnabled = false), blocks))
        assertNull(documentMarkdownPreviewText(document.copy(richTextEnabled = true), blocks))
        assertNull(documentMarkdownPreviewText(document, listOf(NoteBlock(type = NoteBlockType.IMAGE))))
        assertNull(documentMarkdownPreviewText(document, emptyList()))
        assertNull(documentMarkdownPreviewText(document, listOf(NoteBlock(type = NoteBlockType.TEXT, text = " \n "))))
    }
}
"####;

#[cfg(test)]
mod tests {
    use super::*;

    fn editor() -> &'static str {
        super::super::kotlin_sources::SOURCES
            .iter()
            .find(|item| item.path == EDITOR_PATH)
            .expect("real document editor template")
            .contents
    }

    #[test]
    fn real_editor_uses_projection_once_and_preserves_other_bytes() {
        let original = editor();
        let rendered = render(EDITOR_PATH, original).unwrap();
        assert_eq!(rendered.matches(NEW_JOIN).count(), 1);
        assert!(!rendered.contains(OLD_JOIN));
        let restored = rendered.strip_suffix(&format!("\n{HELPERS}")).unwrap();
        assert_eq!(restored.replacen(NEW_JOIN, OLD_JOIN, 1), original);
    }

    #[test]
    fn projection_keeps_live_draft_and_original_eligibility_guards() {
        let rendered = render(EDITOR_PATH, editor()).unwrap();
        for guard in [
            "!sourceDocument.markdownEnabled",
            "sourceDocument.richTextEnabled",
            "draftBlocks.any { it.type != NoteBlockType.TEXT }",
            "textFieldStates[block.id]?.text ?: block.text",
            ".takeIf(String::isNotBlank)",
        ] {
            assert!(rendered.contains(guard), "missing {guard}");
        }
    }

    #[test]
    fn unrelated_renderers_and_sticky_editor_are_unchanged() {
        for path in [
            "com/ofairyo/gridtimer/ui/AndroidRenderedMarkdown.kt",
            "com/ofairyo/gridtimer/ui/SmartisanNoteUi.kt",
            TEST_PATH,
        ] {
            assert_eq!(render(path, OLD_JOIN).unwrap(), OLD_JOIN);
        }
    }

    #[test]
    fn drift_and_duplicate_projection_fail_closed() {
        assert!(render(EDITOR_PATH, "changed template").is_err());
        assert!(render(EDITOR_PATH, &format!("{OLD_JOIN}{OLD_JOIN}")).is_err());
        let rendered = render(EDITOR_PATH, editor()).unwrap();
        assert!(render(EDITOR_PATH, &rendered).is_err());
    }

    #[test]
    fn generator_registers_runtime_transform_and_jvm_regression_tests() {
        let generator = include_str!("../bin/gridtimer_sourcegen.rs");
        assert!(generator.contains("mod android_document_markdown;"));
        assert!(generator.contains("android_document_markdown::TEST_PATH"));
        assert!(generator.contains("android_document_markdown::TEST_CONTENTS"));
        let write = generator.split("fn write_source(").nth(1).unwrap();
        let transform = write.find("android_document_markdown::render(relative_path, &contents)").unwrap();
        assert!(transform < write.find("fs::write(destination, contents.trim_start())").unwrap());
    }
}
