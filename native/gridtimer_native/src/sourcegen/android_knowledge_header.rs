//! Merge the document workspace heading and summary into one adaptive panel.
//! This runs with the existing note-summary pass, before UI localization.

const SCREEN: &str = "com/ofairyo/gridtimer/ui/NoteStudioSheet.kt";

pub fn render(path: &str, source: &str) -> Result<String, String> {
    if path != SCREEN {
        return Ok(source.to_owned());
    }
    // The helper has exactly one call and one declaration in the original screen.
    // A future extra caller must be reconciled instead of silently losing its UI.
    if source.matches("FlowusWorkspaceHeader(").count() != 2 {
        return Err("unexpected document workspace header usage".into());
    }
    let mut result = source.to_owned();
    replace_once(&mut result, HEADER_ITEM, "")?;
    replace_once(
        &mut result,
        "                noteCount = notebookDocuments.size,\n",
        "                noteCount = notebookDocuments.size,\n                visibleCount = if (trashMode) filteredTrashedNotes.size else filteredActiveNotes.size,\n",
    )?;
    replace_once(&mut result, OLD_SUMMARY, COMPACT_SUMMARY)?;
    remove_unused_header(&mut result)?;
    for import in [
        "androidx.compose.foundation.layout.heightIn",
        "androidx.compose.ui.platform.testTag",
        "androidx.compose.ui.semantics.contentDescription",
        "androidx.compose.ui.semantics.semantics",
    ] {
        let statement = format!("import {import}\n");
        if !result.contains(&statement) {
            replace_once(
                &mut result,
                "package com.ofairyo.gridtimer.ui\n",
                &format!("package com.ofairyo.gridtimer.ui\n{statement}"),
            )?;
        }
    }
    Ok(result)
}

fn replace_once(source: &mut String, before: &str, after: &str) -> Result<(), String> {
    if source.matches(before).count() != 1 {
        return Err(format!(
            "compact knowledge header anchor is missing or duplicated: {}",
            before.lines().find(|line| !line.is_empty()).unwrap_or("")
        ));
    }
    *source = source.replacen(before, after, 1);
    Ok(())
}

fn remove_unused_header(source: &mut String) -> Result<(), String> {
    let start = "@Composable\nprivate fun FlowusWorkspaceHeader(";
    let end = "@Composable\nprivate fun FlowusPropertyBadge(";
    if source.matches(start).count() != 1 || source.matches(end).count() != 1 {
        return Err("compact knowledge header helper boundaries changed".into());
    }
    let from = source.find(start).ok_or("missing workspace header helper")?;
    let to = source[from..]
        .find(end)
        .map(|offset| from + offset)
        .ok_or("missing property badge helper")?;
    source.replace_range(from..to, "");
    Ok(())
}

const HEADER_ITEM: &str = r####"        item {
            FlowusWorkspaceHeader(
                title = if (trashMode) "回收站" else "文档",
                subtitle = if (trashMode) "已删除页面" else "本地知识页",
                pageCount = if (trashMode) filteredTrashedNotes.size else filteredActiveNotes.size,
                accent = accent
            )
        }

"####;

const OLD_SUMMARY: &str = r####"@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun NoteSummaryCard(
    accent: Color,
    noteCount: Int,
    trashCount: Int,
    folderCount: Int,
    pinnedCount: Int,
    pendingChecklistCount: Int,
    imageNoteCount: Int,
    latestNote: NoteEntry?,
    now: Long,
    trashMode: Boolean,
    onCreateNote: () -> Unit,
    onToggleTrash: () -> Unit,
    onManageFolders: () -> Unit
) {
    FlowusPanel(accent = accent) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Column(
                    modifier = Modifier.weight(1f),
                    verticalArrangement = Arrangement.spacedBy(6.dp)
                ) {
                    Text(
                        text = if (trashMode) "先恢复，再清理" else "页面先成形，再沉淀",
                        style = MaterialTheme.typography.titleMedium.copy(fontWeight = FontWeight.SemiBold)
                    )
                    Text(
                        text = latestNote?.let { "${noteUpdatedLabel(it, now)}。${it.previewBody()}" }
                            ?: if (trashMode) "恢复后回到文档库。" else "便签可转为知识页。",
                        style = MaterialTheme.typography.bodyMedium.copy(
                            color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.72f)
                        )
                    )
                }
                Spacer(modifier = Modifier.width(12.dp))
                Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    FlowusIconAction("新建页面", Icons.Rounded.Add, accent, onClick = onCreateNote)
                    FlowusIconAction(
                        if (trashMode) "回到文档" else "回收站",
                        if (trashMode) Icons.Rounded.FolderOpen else Icons.Rounded.RestoreFromTrash,
                        MaterialTheme.colorScheme.secondary,
                        onClick = onToggleTrash
                    )
                }
            }
            FlowRow(
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                FlowusPropertyBadge(label = "页面 $noteCount", accent = accent)
                FlowusPropertyBadge(label = "置顶 $pinnedCount", accent = accentFor("amber"))
                FlowusPropertyBadge(label = "任务块 $pendingChecklistCount", accent = accentFor("green"))
                FlowusPropertyBadge(label = "图片 $imageNoteCount", accent = accentFor("teal"))
                FlowusPropertyBadge(label = "文档库 $folderCount", accent = MaterialTheme.colorScheme.secondary)
                FlowusPropertyBadge(label = "回收站 $trashCount", accent = MaterialTheme.colorScheme.primary)
            }
            FlowusIconAction("管理文档库", Icons.Rounded.Folder, accent, onClick = onManageFolders)
        }
    }
}

"####;

const COMPACT_SUMMARY: &str = r####"@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun NoteSummaryCard(
    accent: Color,
    noteCount: Int,
    visibleCount: Int,
    trashCount: Int,
    folderCount: Int,
    pinnedCount: Int,
    pendingChecklistCount: Int,
    imageNoteCount: Int,
    latestNote: NoteEntry?,
    now: Long,
    trashMode: Boolean,
    onCreateNote: () -> Unit,
    onToggleTrash: () -> Unit,
    onManageFolders: () -> Unit
) {
    val totalCount = if (trashMode) trashCount else noteCount
    FlowusPanel(modifier = Modifier.testTag("knowledge_compact_header"), accent = accent) {
        Column(
            modifier = Modifier.fillMaxWidth().padding(14.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            // FlowRow also accommodates large fonts and long filtered counts.
            FlowRow(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalArrangement = Arrangement.spacedBy(4.dp)
            ) {
                Text(
                    text = if (trashMode) "回收站" else "文档",
                    style = MaterialTheme.typography.titleLarge.copy(fontWeight = FontWeight.SemiBold)
                )
                FlowusPropertyBadge(
                    label = if (visibleCount == totalCount) "$totalCount 页"
                        else "显示 $visibleCount / 共 $totalCount 页",
                    accent = accent
                )
            }
            FlowRow(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp)
            ) {
                FlowusIconAction(
                    "新建页面", Icons.Rounded.Add, accent,
                    modifier = Modifier.heightIn(min = 48.dp).testTag("knowledge_header_create"),
                    onClick = onCreateNote
                )
                FlowusIconAction(
                    if (trashMode) "回到文档" else "回收站 $trashCount",
                    if (trashMode) Icons.Rounded.FolderOpen else Icons.Rounded.RestoreFromTrash,
                    MaterialTheme.colorScheme.secondary,
                    modifier = Modifier.heightIn(min = 48.dp).testTag("knowledge_header_trash"),
                    onClick = onToggleTrash
                )
                FlowusIconAction(
                    "文档库 $folderCount", Icons.Rounded.Folder, accent,
                    modifier = Modifier.heightIn(min = 48.dp).testTag("knowledge_header_folders")
                        .semantics { contentDescription = "管理文档库，共 $folderCount 个" },
                    onClick = onManageFolders
                )
            }
            FlowRow(
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp)
            ) {
                FlowusPropertyBadge(label = "置顶 $pinnedCount", accent = accentFor("amber"))
                FlowusPropertyBadge(label = "任务块 $pendingChecklistCount", accent = accentFor("green"))
                FlowusPropertyBadge(label = "图片 $imageNoteCount", accent = accentFor("teal"))
            }
            // Keep the useful recent-page preview, but never let it expand the header.
            // Reuse the original privacy-aware preview accessor, not raw note content.
            latestNote?.let { note ->
                Text(
                    text = "${noteUpdatedLabel(note, now)}。${note.previewBody()}",
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    style = MaterialTheme.typography.bodySmall.copy(
                        color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.62f)
                    )
                )
            }
        }
    }
}

"####;

#[cfg(test)]
mod tests {
    use super::*;

    fn base_screen() -> &'static str {
        crate::kotlin_sources::SOURCES
            .iter()
            .find(|source| source.path == SCREEN)
            .expect("real NoteStudioSheet template")
            .contents
    }

    #[test]
    fn merges_the_real_two_panel_template_without_changing_callbacks() {
        let result = render(SCREEN, base_screen()).unwrap();
        assert!(!result.contains("FlowusWorkspaceHeader("));
        assert!(!result.contains("页面先成形，再沉淀"));
        assert!(!result.contains("label = \"页面 $noteCount\""));
        assert_eq!(result.matches("NoteSummaryCard(").count(), 2);
        for tag in [
            "knowledge_compact_header",
            "knowledge_header_create",
            "knowledge_header_trash",
            "knowledge_header_folders",
        ] {
            assert_eq!(result.matches(&format!("testTag(\"{tag}\")")).count(), 1);
        }
        for callback in ["onCreateNote", "onToggleTrash", "onManageFolders"] {
            assert!(COMPACT_SUMMARY.contains(&format!("onClick = {callback}")));
        }
    }

    #[test]
    fn cached_summary_pass_still_feeds_the_single_header() {
        let result = crate::android_note_list_performance::render(SCREEN, base_screen()).unwrap();
        assert!(result.contains("trashCount = summaryTrashCount,"));
        assert!(result.contains("pinnedCount = summaryPinnedCount,"));
        assert!(result.contains("pendingChecklistCount = summaryPendingCount,"));
        assert!(result.contains("imageNoteCount = summaryImageCount,"));
        assert!(result.contains("testTag(\"knowledge_compact_header\")"));
    }

    #[test]
    fn visible_and_total_counts_keep_search_and_trash_scopes() {
        let result = render(SCREEN, base_screen()).unwrap();
        assert!(result.contains(
            "visibleCount = if (trashMode) filteredTrashedNotes.size else filteredActiveNotes.size,"
        ));
        assert!(COMPACT_SUMMARY.contains("val totalCount = if (trashMode) trashCount else noteCount"));
        assert!(COMPACT_SUMMARY.contains("else \"显示 $visibleCount / 共 $totalCount 页\""));
    }

    #[test]
    fn retains_actions_stats_and_bounded_privacy_aware_preview() {
        assert_eq!(COMPACT_SUMMARY.matches("FlowusIconAction(").count(), 3);
        assert_eq!(COMPACT_SUMMARY.matches("heightIn(min = 48.dp)").count(), 3);
        assert_eq!(COMPACT_SUMMARY.matches("FlowRow(").count(), 3);
        assert!(COMPACT_SUMMARY.contains("note.previewBody()"));
        assert!(COMPACT_SUMMARY.contains("maxLines = 1"));
        assert!(!COMPACT_SUMMARY.contains("note.content"));
        for stat in [
            "$pinnedCount",
            "$pendingChecklistCount",
            "$imageNoteCount",
            "$folderCount",
            "$trashCount",
        ] {
            assert!(COMPACT_SUMMARY.contains(stat));
        }
    }

    #[test]
    fn preserves_shared_actions_ai_entry_editor_and_sticky_note_ui() {
        let original = base_screen();
        let rendered = render(SCREEN, original).unwrap();
        let tail = "@Composable\nprivate fun FlowusIconAction(";
        assert_eq!(
            original.split_once(tail).unwrap().1,
            rendered.split_once(tail).unwrap().1
        );
        let call = "                onCreateNote = { onCreateNote(NoteDraftPreset.BLANK) },";
        assert_eq!(
            original.matches(call).count(),
            rendered.matches(call).count()
        );
    }

    #[test]
    fn refuses_missing_duplicate_or_edited_anchors() {
        let original = base_screen();
        assert!(render(SCREEN, &original.replacen(HEADER_ITEM, "", 1)).is_err());
        assert!(render(SCREEN, &format!("{original}{HEADER_ITEM}")).is_err());
        assert!(render(
            SCREEN,
            &original.replacen("页面先成形，再沉淀", "changed upstream", 1)
        )
        .is_err());
    }

    #[test]
    fn unrelated_templates_are_byte_identical() {
        let source = "a screen that is not the notebook";
        assert_eq!(
            render("com/ofairyo/gridtimer/ui/GridTimerScreen.kt", source).unwrap(),
            source
        );
    }
}
