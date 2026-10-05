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
    let from = source
        .find(start)
        .ok_or("missing workspace header helper")?;
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
    fn refuses_missing_duplicate_or_unreconciled_template_anchors() {
        let original = base_screen();
        assert!(render(SCREEN, original).is_ok());
        assert!(render(SCREEN, &original.replacen(HEADER_ITEM, "", 1)).is_err());
        assert!(render(SCREEN, &format!("{original}{HEADER_ITEM}")).is_err());
        assert!(render(
            SCREEN,
            &format!("{original}\nFlowusWorkspaceHeader(title = extraTitle)\n")
        )
        .is_err());

        let summary_call = "                noteCount = notebookDocuments.size,\n";
        assert!(render(SCREEN, &original.replacen(summary_call, "", 1)).is_err());
        assert!(render(SCREEN, &format!("{original}{summary_call}")).is_err());
    }

    #[test]
    fn refuses_ambiguous_helper_removal_boundaries() {
        let original = base_screen();
        let start = "@Composable\nprivate fun FlowusWorkspaceHeader(";
        let end = "@Composable\nprivate fun FlowusPropertyBadge(";
        // Keep the same helper usage count: the removal boundary itself must fail closed.
        assert!(render(
            SCREEN,
            &original.replacen(
                start,
                "@Composable /* moved */\nprivate fun FlowusWorkspaceHeader(",
                1
            )
        )
        .is_err());
        assert!(render(
            SCREEN,
            &original.replacen(end, "private fun FlowusPropertyBadge(", 1)
        )
        .is_err());
        assert!(render(SCREEN, &format!("{original}{end}")).is_err());
    }
}
