// v2.22.30 - Stop perpetual decorative frames and isolate the dial's static face.

pub fn render(path: &str, base: &str) -> Result<String, String> {
    if path != "com/ofairyo/gridtimer/ui/GridTimerScreen.kt" {
        return Ok(base.to_owned());
    }
    let mut source = base.to_owned();
    replace_section(
        &mut source,
        "@Composable\nprivate fun rememberRunningEmphasis(",
        "@Composable\nprivate fun rememberNowState(",
        RUNNING_INDICATOR,
    )?;
    replace_section(
        &mut source,
        "@Composable\nprivate fun rememberNowState(",
        "@Composable\ninternal fun rememberCurrentTimeMillis(",
        FOREGROUND_CLOCK,
    )?;
    replace_section(
        &mut source,
        "@Composable\nprivate fun AmbientBackdrop(",
        "\n\n/*\nprivate fun MyAccountScreen(",
        BACKDROP,
    )?;
    replace_once(&mut source, "import androidx.lifecycle.LifecycleEventObserver\n",
        "import androidx.lifecycle.LifecycleEventObserver\nimport androidx.lifecycle.repeatOnLifecycle\n")?;
    replace_once(&mut source, "import androidx.compose.ui.draw.drawBehind\n",
        "import androidx.compose.ui.draw.drawBehind\nimport androidx.compose.ui.draw.drawWithCache\n")?;
    replace_once(
        &mut source,
        "import androidx.compose.ui.graphics.Brush\n",
        "import androidx.compose.ui.graphics.Brush\nimport androidx.compose.ui.graphics.Path\n",
    )?;
    replace_once(
        &mut source,
        "    val focusSuggestion = appData.homeFocusSuggestion()",
        "    val focusSuggestion = remember(appData.slots) { appData.homeFocusSuggestion() }",
    )?;
    replace_once(&mut source, "import androidx.compose.runtime.Composable\n",
        "import androidx.compose.runtime.Composable\nimport androidx.compose.runtime.produceState\n")?;
    replace_section(&mut source, "    val todaySummary = remember(appData.sessions, currentDayWindow)",
        "    val latestSession = uiIndex.latestSession",
        "    val todayStats = rememberHomeDayStats(appData.sessions, currentDayWindow)\n    val todaySummary = todayStats.summary\n    val todayTotal = todaySummary.totalDurationMillis\n    val todayDurationBySlotId = todayStats.durationBySlotId\n")?;
    source.push_str(HOME_DAY_STATS);

    // Keep elapsed time out of the cached face's parameters. A new drawWithCache
    // lambda capturing elapsedMillis would invalidate its cache every second.
    replace_section(
        &mut source,
        "@Composable\nprivate fun TimerDial(",
        "@Composable\nprivate fun rememberRunningEmphasis(",
        TIMER_DIAL,
    )?;
    Ok(source)
}

const HOME_DAY_STATS: &str = r####"

private class HomeSessionsKey(private val sessions: List<TimerSession>) {
    override fun equals(other: Any?): Boolean = other is HomeSessionsKey && sessions === other.sessions
    override fun hashCode(): Int = System.identityHashCode(sessions)
}

private data class HomeDayStats(
    val request: Any?,
    val summary: TimerSessionWindowSummary,
    val durationBySlotId: Map<Int, Long>
)

private val EmptyHomeDayStats = HomeDayStats(null, TimerSessionWindowSummary(0L, 0, null, 0L, null, null), emptyMap())

@Composable
private fun rememberHomeDayStats(sessions: List<TimerSession>, window: Pair<Long, Long>): HomeDayStats {
    val request = remember(HomeSessionsKey(sessions), window) { Any() }
    val result = produceState(initialValue = EmptyHomeDayStats, key1 = request) {
        value = withContext(Dispatchers.Default) {
            val totals = mutableMapOf<Int, Long>()
            for (session in sessions) {
                if (session.endedAtEpochMillis in window.first until window.second) {
                    totals[session.slotId] = com.ofairyo.gridtimer.data.saturatedDurationAdd(
                        totals[session.slotId] ?: 0L, session.durationMillis
                    )
                }
            }
            HomeDayStats(request, timerSessionWindowSummary(sessions, window.first, window.second), totals)
        }
    }.value
    return if (result.request === request) result else EmptyHomeDayStats
}
"####;

fn replace_once(source: &mut String, old: &str, new: &str) -> Result<(), String> {
    if source.matches(old).count() != 1 {
        return Err(format!(
            "responsiveness override expected exactly one {old:?}"
        ));
    }
    *source = source.replacen(old, new, 1);
    Ok(())
}

fn replace_section(source: &mut String, start: &str, end: &str, new: &str) -> Result<(), String> {
    if source.matches(start).count() != 1 || source.matches(end).count() != 1 {
        return Err(format!(
            "responsiveness override has ambiguous section {start:?}"
        ));
    }
    let from = source.find(start).unwrap();
    let to = source[from..]
        .find(end)
        .ok_or_else(|| format!("missing {end:?} after {start:?}"))?
        + from;
    source.replace_range(from..to, new);
    Ok(())
}

const RUNNING_INDICATOR: &str = r####"@Composable
private fun rememberRunningEmphasis(enabled: Boolean): State<Float> {
    // Running is a state, not an endless animation. The clock still updates each second.
    return rememberUpdatedState(if (enabled) 0.5f else 0f)
}

"####;

const FOREGROUND_CLOCK: &str = r####"@Composable
private fun rememberNowState(
    tickMillis: Long,
    enabled: Boolean = true
): MutableLongState {
    val nowState = remember { mutableLongStateOf(System.currentTimeMillis()) }
    val lifecycleOwner = LocalLifecycleOwner.current
    LaunchedEffect(tickMillis, enabled, lifecycleOwner) {
        nowState.longValue = System.currentTimeMillis()
        if (!enabled) return@LaunchedEffect
        val interval = tickMillis.coerceAtLeast(1L)
        lifecycleOwner.lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (isActive) {
                val now = System.currentTimeMillis()
                nowState.longValue = now
                // Align visible clocks; returning to the app reads the real timestamp.
                delay(interval - Math.floorMod(now, interval))
            }
        }
    }
    return nowState
}

"####;

const BACKDROP: &str = r####"@Composable
private fun AmbientBackdrop(hasRunningSlots: Boolean) {
    val colors = MaterialTheme.colorScheme
    val light = colors.background.red > 0.5f
    val accent = colors.primary
    val highlight = colors.topHighlight
    val shadow = colors.depthShadow
    Spacer(
        modifier = Modifier.fillMaxSize().drawWithCache {
            val surface = Brush.verticalGradient(
                colors = listOf(
                    highlight.copy(alpha = if (light) 0.16f else 0.08f),
                    Color.Transparent,
                    shadow.copy(alpha = if (light) 0.035f else 0.08f)
                )
            )
            val runningGlow = Brush.radialGradient(
                colors = listOf(accent.copy(alpha = if (light) 0.045f else 0.075f), Color.Transparent),
                center = Offset(size.width * 0.5f, size.height * 0.82f),
                radius = (size.minDimension * 0.7f).coerceAtLeast(1f)
            )
            onDrawBehind {
                drawRect(surface)
                if (hasRunningSlots) drawRect(runningGlow)
            }
        }
    )
}

"####;

const TIMER_DIAL: &str = r####"@Composable
private fun TimerDial(
    accent: Color,
    elapsedMillis: Long,
    isRunning: Boolean,
    modifier: Modifier = Modifier
) {
    val dialTick = MaterialTheme.colorScheme.onSurface.copy(alpha = if (isRunning) 0.36f else 0.24f)
    val dialBright = MaterialTheme.colorScheme.surfaceBright
    val dialHighlight = MaterialTheme.colorScheme.topHighlight
    val secondFraction = ((elapsedMillis % 60_000L).toFloat() / 60_000f).coerceIn(0f, 1f)
    val sweepAngle = if (isRunning) secondFraction * 360f else 208f
    Box(modifier = modifier) {
        TimerDialFace(accent, isRunning, Modifier.matchParentSize())
        Canvas(modifier = Modifier.matchParentSize()) {
            val radius = min(size.width, size.height) / 2f
            val arcRadius = radius * 0.73f
            drawArc(
                color = accent.copy(alpha = if (isRunning) 0.62f else 0.16f),
                startAngle = -90f, sweepAngle = sweepAngle, useCenter = false,
                topLeft = Offset(center.x - arcRadius, center.y - arcRadius),
                size = Size(arcRadius * 2f, arcRadius * 2f),
                style = Stroke(width = radius * 0.082f, cap = StrokeCap.Round)
            )
            val orbitRadians = (-90f + sweepAngle) / 180f * PI.toFloat()
            val orbitCenter = Offset(center.x + cos(orbitRadians) * arcRadius, center.y + sin(orbitRadians) * arcRadius)
            drawCircle(accent.copy(alpha = if (isRunning) 0.90f else 0.42f), radius * 0.044f, orbitCenter)
            drawCircle(dialBright.copy(alpha = 0.82f), radius * 0.018f, orbitCenter)
            rotate(degrees = -90f + secondFraction * 360f, pivot = center) {
                drawLine(if (isRunning) accent else dialTick.copy(alpha = 0.56f), center,
                    Offset(center.x, center.y - radius * 0.50f), radius * 0.046f, StrokeCap.Round)
                drawLine(dialTick.copy(alpha = 0.28f), center,
                    Offset(center.x, center.y + radius * 0.22f), radius * 0.022f, StrokeCap.Round)
                drawLine(dialHighlight.copy(alpha = 0.46f), Offset(center.x, center.y - radius * 0.08f),
                    Offset(center.x, center.y - radius * 0.46f), radius * 0.012f, StrokeCap.Round)
            }
            drawCircle(accent.copy(alpha = 0.20f), radius * 0.17f)
            drawCircle(dialBright.copy(alpha = 0.62f), radius * 0.11f)
            drawCircle(if (isRunning) accent else dialTick.copy(alpha = 0.45f), radius * 0.075f)
        }
    }
}

@Composable
private fun TimerDialFace(accent: Color, isRunning: Boolean, modifier: Modifier) {
    val colors = MaterialTheme.colorScheme
    val outer = if (isRunning) accent.copy(alpha = 0.18f) else colors.outline.copy(alpha = 0.10f)
    val tickColor = colors.onSurface.copy(alpha = if (isRunning) 0.36f else 0.24f)
    val surface = colors.surface.copy(alpha = 0.97f)
    val bright = colors.surfaceBright
    val inner = colors.surfaceContainerHigh
    val highlight = colors.topHighlight
    val chrome = colors.chrome
    val shadow = colors.depthShadow.copy(alpha = if (isRunning) 0.16f else 0.10f)
    Spacer(modifier = modifier.drawWithCache {
        val radius = size.minDimension / 2f
        val center = Offset(size.width / 2f, size.height / 2f)
        val bezel = Brush.radialGradient(
            listOf(bright, surface, chrome.copy(alpha = 0.98f), shadow.copy(alpha = 0.78f)),
            Offset(center.x - radius * 0.18f, center.y - radius * 0.22f),
            radius.coerceAtLeast(1f)
        )
        val paths = List(4) { Path() }
        for (tick in 0 until 60) {
            val angle = tick / 60f * 2f * PI.toFloat() - PI.toFloat() / 2f
            val tier = when { tick % 15 == 0 -> 0; tick % 5 == 0 -> 1; tick % 2 == 0 -> 2; else -> 3 }
            val outside = when (tier) { 0 -> 0.76f; 1 -> 0.75f; else -> 0.735f }
            val inside = when (tier) { 0 -> 0.42f; 1 -> 0.52f; else -> 0.61f }
            paths[tier].moveTo(center.x + cos(angle) * radius * inside, center.y + sin(angle) * radius * inside)
            paths[tier].lineTo(center.x + cos(angle) * radius * outside, center.y + sin(angle) * radius * outside)
        }
        val strokes = listOf(0.034f, 0.023f, 0.010f, 0.010f).map { Stroke(radius * it, cap = StrokeCap.Round) }
        val tickColors = listOf(
            if (isRunning) accent.copy(alpha = 0.46f) else tickColor.copy(alpha = 0.34f),
            tickColor.copy(alpha = if (isRunning) 0.34f else 0.25f),
            tickColor.copy(alpha = 0.16f), tickColor.copy(alpha = 0.11f)
        )
        onDrawBehind {
            drawCircle(bezel, radius * 0.94f)
            drawCircle(highlight.copy(alpha = 0.32f), radius * 0.94f, style = Stroke(radius * 0.02f))
            drawCircle(outer, radius * 0.82f, style = Stroke(radius * 0.11f))
            drawCircle(surface.copy(alpha = 0.98f), radius * 0.62f)
            drawCircle(inner.copy(alpha = 0.92f), radius * 0.62f * 0.82f)
            drawCircle(shadow.copy(alpha = 0.16f), radius * 0.62f, style = Stroke(radius * 0.012f))
            for (tier in paths.indices) drawPath(paths[tier], tickColors[tier], style = strokes[tier])
            drawCircle(tickColor.copy(alpha = 0.08f), radius * 0.73f, style = Stroke(radius * 0.07f))
        }
    })
}

"####;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_screen_stops_idle_frames_without_removing_live_time() {
        let base = crate::kotlin_sources::SOURCES
            .iter()
            .find(|s| s.path.ends_with("/GridTimerScreen.kt"))
            .unwrap();
        let previous =
            crate::android_performance_override::render(base.path, base.contents).unwrap();
        let screen = render(base.path, &previous).unwrap();
        assert!(!screen.contains("emphasis.animateTo("));
        assert!(!screen.contains("emphasis.snapTo("));
        assert!(screen.contains("repeatOnLifecycle(Lifecycle.State.STARTED)"));
        assert!(
            screen.contains("rememberCurrentTimeMillis(tickMillis = 1_000L, enabled = isRunning)")
        );
        let face = screen
            .split("private fun TimerDialFace(")
            .nth(1)
            .unwrap()
            .split("@Composable")
            .next()
            .unwrap();
        assert!(!face.contains("elapsedMillis"));
        assert!(
            face.find("for (tick in 0 until 60)").unwrap() < face.find("onDrawBehind").unwrap()
        );
        assert!(screen.contains("testTag(\"slot_timer_toggle_${slot.id}\")"));
    }

    #[test]
    fn stale_or_double_applied_screen_override_is_rejected() {
        assert!(render(
            "com/ofairyo/gridtimer/ui/GridTimerScreen.kt",
            "changed template"
        )
        .is_err());
        assert_eq!(render("unrelated.kt", "original").unwrap(), "original");
    }
}
