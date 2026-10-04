// v2.22.43 - Keep pause controls and timer geometry stable across state changes.
const SCREEN: &str = "com/ofairyo/gridtimer/ui/GridTimerScreen.kt";

fn replace(source: &mut String, old: &str, new: &str) -> Result<(), String> {
    if source.matches(old).count() != 1 {
        return Err(format!("timer layout anchor changed: {old}"));
    }
    *source = source.replacen(old, new, 1);
    Ok(())
}

fn section(
    source: &mut String,
    start: &str,
    end: &str,
    edit: impl FnOnce(&mut String) -> Result<(), String>,
) -> Result<(), String> {
    let begin = source.find(start).ok_or("timer layout section missing")?;
    let finish = source[begin..]
        .find(end)
        .map(|offset| begin + offset)
        .ok_or("timer layout section end missing")?;
    let mut part = source[begin..finish].to_owned();
    edit(&mut part)?;
    source.replace_range(begin..finish, &part);
    Ok(())
}

pub fn render(path: &str, base: &str) -> Result<String, String> {
    if path != SCREEN {
        return Ok(base.to_owned());
    }
    let mut source = base.to_owned();
    replace(
        &mut source,
        "val sweepAngle = if (isRunning) secondFraction * 360f else 208f",
        "// Pausing freezes the actual progress instead of snapping to a decorative angle.\n    val sweepAngle = secondFraction * 360f",
    )?;
    replace(
        &mut source,
        "modifier = Modifier.animateItemPlacement(animationSpec = timerTilePlacementAnimation),",
        "// Placement motion belongs to an explicit reorder, not a timer state update.\n                    modifier = if (dragSession != null || pendingCommittedSlotOrder != null) {\n                        Modifier.animateItemPlacement(animationSpec = timerTilePlacementAnimation)\n                    } else {\n                        Modifier\n                    },",
    )?;
    section(
        &mut source,
        "private fun TimerTile(\n",
        "@Composable\nprivate fun TimerTileBodyText(",
        |part| {
            let begin = part
                .find("                        if (slotStatusLabel.isNotBlank()) {")
                .ok_or("timer status layout missing")?;
            let finish = part[begin..]
                .find("\n                    }\n\n                }")
                .map(|offset| begin + offset)
                .ok_or("timer footer layout missing")?;
            part.replace_range(begin..finish, TILE_STATUS);
            Ok(())
        },
    )?;
    section(
        &mut source,
        "private fun DetailActionDock(\n",
        "@Composable\ninternal fun SectionCard(",
        |part| {
            replace(part,
                "text = if (isRunning) \"点击即可暂停并保存本次记录\" else \"点击即可开始，让这个格子进入计时\",",
                "text = if (isRunning) \"正在计时\" else if (canArchive) \"已暂停\" else \"待开始\",")?;
            replace(
                part,
                r####"            if (canArchive) {
                PhysicalButton(
                    label = "归档",
                    icon = Icons.Rounded.Archive,
                    accent = MaterialTheme.colorScheme.secondary,
                    filled = false,
                    modifier = Modifier.fillMaxWidth(),
                    onClick = onArchive
                )
            }"####,
                r####"            // Keep the dock height and primary hit target stable when pausing.
            PhysicalButton(
                label = "归档",
                icon = Icons.Rounded.Archive,
                accent = MaterialTheme.colorScheme.secondary,
                filled = false,
                modifier = Modifier.fillMaxWidth(),
                enabled = canArchive,
                onClick = onArchive
            )"####,
            )
        },
    )?;
    section(
        &mut source,
        "private fun DetailHeroCard(\n",
        "@Composable\nprivate fun DetailActionDock(",
        |part| {
            // Both responsive layouts reserve the same two-line status area.
            if part.matches("text = statusLine,").count() != 2 {
                return Err("detail status anchors changed".to_owned());
            }
            *part = part.replace("text = statusLine,", "text = statusLine,\n                        minLines = 2,\n                        maxLines = 2,\n                        overflow = TextOverflow.Ellipsis,");
            replace(
                part,
                r####"                if (canArchive) {
                    MiniChromeBadge(
                        label = "可归档",
                        accent = MaterialTheme.colorScheme.secondary
                    )
                }
"####,
                "",
            )
        },
    )?;
    // Overview expansion has its own AnimatedVisibility. Do not spring the entire
    // header while the background statistics and running state arrive separately.
    replace(
        &mut source,
        "                    .animateContentSize(animationSpec = headerResizeSpring)\n",
        "",
    )?;
    replace(
        &mut source,
        "            text = heroDescription,\n            maxLines = if (compactLayout) 2 else Int.MAX_VALUE,",
        "            text = heroDescription,\n            minLines = if (compactLayout) 2 else 1,\n            maxLines = if (compactLayout) 2 else Int.MAX_VALUE,",
    )?;
    section(
        &mut source,
        "internal fun MiniChromeBadge(",
        "@Composable\ninternal fun MiniChromeIconBadge(",
        |part| {
            replace(part,
            "                fontWeight = FontWeight.Medium\n",
            "                fontWeight = FontWeight.Medium,\n                fontFeatureSettings = \"tnum\"\n")
        },
    )?;
    Ok(source)
}

const TILE_STATUS: &str = r####"                        // Reserve both lines even when the async statistics are not ready.
                        // A pause must not resize the card or displace the next grid row.
                        Text(
                            text = slotStatusLabel,
                            minLines = 2,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis,
                            style = MaterialTheme.typography.bodySmall.copy(
                                color = signalAccent.copy(alpha = if (isRunning) 0.90f else 0.74f),
                                lineHeight = 20.sp
                            )
                        )
                        Text(
                            text = footerLabel,
                            minLines = 2,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis,
                            style = MaterialTheme.typography.bodySmall.copy(
                                color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.58f),
                                lineHeight = 20.sp
                            )
                        )"####;
