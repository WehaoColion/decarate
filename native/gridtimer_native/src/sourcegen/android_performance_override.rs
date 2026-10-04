// v2.22.30 - Remove UI-thread data work and preserve unchanged UI domains.
// Keep the v2.22.25 Android behavior while moving only hot work
// out of composition and bounding pathological micro-break catch-up.

#[path = "android_data_responsiveness.rs"]
mod android_data_responsiveness;

pub const SCREEN_PATH: &str = "com/ofairyo/gridtimer/ui/GridTimerScreen.kt";
pub const REPOSITORY_PATH: &str = "com/ofairyo/gridtimer/data/TimerRepository.kt";
pub const MICRO_BREAKS_PATH: &str = "com/ofairyo/gridtimer/data/MicroBreaks.kt";
pub const HOME_TRACE_PATH: &str = "com/ofairyo/gridtimer/ui/HomePerformanceTrace.kt";
pub const SYNC_ROUTE_PATH: &str = "com/ofairyo/gridtimer/data/SyncNetworkRoute.kt";

pub fn render(path: &str, base: &str) -> Result<String, String> {
    let rendered = match path {
        SCREEN_PATH => render_screen(base),
        REPOSITORY_PATH => render_repository(base),
        MICRO_BREAKS_PATH => render_micro_breaks(base),
        HOME_TRACE_PATH => render_home_trace(base),
        SYNC_ROUTE_PATH => render_sync_route(base),
        _ => Ok(base.to_string()),
    }?;
    android_data_responsiveness::render(path, &rendered)
}

fn replace_once(target: &mut String, old: &str, new: &str, label: &str) -> Result<(), String> {
    let matches = target.match_indices(old).count();
    if matches != 1 {
        return Err(format!(
            "expected exactly one {label} fragment, found {matches}"
        ));
    }
    target.replace_range(
        target
            .find(old)
            .ok_or_else(|| format!("missing {label} fragment"))?
            ..target.find(old).unwrap() + old.len(),
        new,
    );
    Ok(())
}

fn replace_section(
    target: &mut String,
    start: &str,
    end: &str,
    replacement: &str,
    label: &str,
) -> Result<(), String> {
    let start_matches = target.match_indices(start).count();
    let end_matches = target.match_indices(end).count();
    if start_matches != 1 || end_matches != 1 {
        return Err(format!(
            "expected one {label} section boundary, found start={start_matches}, end={end_matches}"
        ));
    }
    let start_index = target
        .find(start)
        .ok_or_else(|| format!("missing {label} start"))?;
    let end_index = target[start_index..]
        .find(end)
        .map(|offset| start_index + offset)
        .ok_or_else(|| format!("missing {label} end"))?;
    target.replace_range(start_index..end_index, replacement);
    Ok(())
}

fn render_screen(base: &str) -> Result<String, String> {
    let mut rendered = base.to_string();

    replace_once(
        &mut rendered,
        "import androidx.compose.runtime.SideEffect\nimport androidx.compose.runtime.collectAsState",
        "import androidx.compose.runtime.SideEffect\nimport androidx.compose.runtime.State\nimport androidx.compose.runtime.collectAsState",
        "running emphasis state import",
    )?;

    replace_section(
        &mut rendered,
        "@Composable\nprivate fun AmbientBackdrop(",
        "\n\n/*\nprivate fun MyAccountScreen(",
        r####"@Composable
private fun AmbientBackdrop(hasRunningSlots: Boolean) {
    val lightBackdrop = MaterialTheme.colorScheme.background.red > 0.5f
    val runningAccent = MaterialTheme.colorScheme.primary
    val runningPulse = rememberRunningEmphasis(enabled = hasRunningSlots)

    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(
                Brush.radialGradient(
                    colors = listOf(
                        Color.White.copy(alpha = if (lightBackdrop) 0.18f else 0.06f),
                        Color.Transparent
                    ),
                    center = Offset(180f, 96f),
                    radius = 560f
                )
            )
    )
    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(
                Brush.radialGradient(
                    colors = listOf(
                        MaterialTheme.colorScheme.depthShadow.copy(alpha = if (lightBackdrop) 0.08f else 0.20f),
                        Color.Transparent
                    ),
                    center = Offset(960f, 1_680f),
                    radius = 880f
                )
            )
    )
    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(
                Brush.verticalGradient(
                    colors = listOf(
                        MaterialTheme.colorScheme.topHighlight.copy(alpha = if (lightBackdrop) 0.18f else 0.10f),
                        Color.Transparent,
                        MaterialTheme.colorScheme.depthShadow.copy(alpha = if (lightBackdrop) 0.02f else 0.08f)
                    ),
                    endY = 2_200f
                )
            )
    )
    Box(
        modifier = Modifier
            .fillMaxSize()
            .drawBehind {
                val pulse = runningPulse.value
                if (hasRunningSlots || pulse > 0f) {
                    drawRect(
                        brush = Brush.radialGradient(
                            colors = listOf(
                                runningAccent.copy(
                                    alpha = if (lightBackdrop) {
                                        0.035f + pulse * 0.030f
                                    } else {
                                        0.060f + pulse * 0.045f
                                    }
                                ),
                                Color.Transparent
                            ),
                            center = Offset(540f, 1_640f),
                            radius = 420f + pulse * 180f
                        )
                    )
                }
            }
    )
}
"####,
        "draw-phase ambient pulse",
    )?;

    replace_once(
        &mut rendered,
        "    val runningPulse = rememberRunningEmphasis(enabled = activeCount > 0)\n    val dockSurfaceColor = MaterialTheme.colorScheme.panel\n    val selectedIndicatorColor = selectedAccent.mixOverDock(dockSurfaceColor, 0.16f + runningPulse * 0.04f)",
        "    val runningPulse = rememberRunningEmphasis(enabled = activeCount > 0)\n    val dockSurfaceColor = MaterialTheme.colorScheme.panel\n    val selectedIndicatorColor = selectedAccent.mixOverDock(dockSurfaceColor, 0.16f)",
        "static dock indicator color",
    )?;

    replace_once(
        &mut rendered,
        "    livePulse: Float = 0f,",
        "    livePulse: State<Float>? = null,",
        "stable dock pulse state",
    )?;

    replace_section(
        &mut rendered,
        "                modifier = Modifier.size(28.dp),\n                contentAlignment = Alignment.Center\n            ) {\n                if (livePulse > 0f && testTag == \"home_nav_board\") {",
        "                BalancedIcon(\n                    imageVector = icon,\n                    tint = iconColor,",
        r####"                modifier = Modifier
                    .size(28.dp)
                    .drawBehind {
                        val pulse = livePulse?.value ?: 0f
                        if (pulse > 0f && testTag == "home_nav_board") {
                            drawCircle(
                                color = accent.copy(
                                    alpha = if (selected) 0.24f else 0.16f + pulse * 0.10f
                                ),
                                radius = (12.5f + pulse * 5f).dp.toPx() / 2f
                            )
                        }
                    },
                contentAlignment = Alignment.Center
            ) {
"####,
        "draw-phase dock pulse",
    )?;

    replace_section(
        &mut rendered,
        "                if (isRunning) {\n                    Box(\n                        modifier = Modifier\n                            .fillMaxWidth()\n                            .height(3.dp)\n                            .background(\n                                Brush.horizontalGradient(\n                                    colors = listOf(\n                                        Color.Transparent,\n                                        categoryAccent.copy(alpha = 0.16f + runningHighlight * 0.24f),\n                                        Color.Transparent\n                                    )\n                                )\n                            )\n                    )\n                }",
        "\n                Column(\n                    modifier = Modifier\n                        .fillMaxWidth()\n                        .padding(16.dp),\n                    verticalArrangement = Arrangement.spacedBy(14.dp)\n                ) {",
        r####"                if (isRunning) {
                    Box(
                        modifier = Modifier
                            .fillMaxWidth()
                            .height(3.dp)
                            .drawBehind {
                                val pulse = runningHighlight.value
                                drawRect(
                                    brush = Brush.horizontalGradient(
                                        colors = listOf(
                                            Color.Transparent,
                                            categoryAccent.copy(alpha = 0.16f + pulse * 0.24f),
                                            Color.Transparent
                                        )
                                    )
                                )
                            }
                    )
                }
"####,
        "draw-phase tile pulse",
    )?;

    replace_once(
        &mut rendered,
        "private fun StatusLight(accent: Color, emphasis: Float) {\n    Box(\n        modifier = Modifier\n            .size((7.5f + emphasis * 2.5f).dp)\n            .clip(CircleShape)\n            .background(accent.copy(alpha = 0.78f + emphasis * 0.16f))\n    )\n}",
        r####"private fun StatusLight(accent: Color, emphasis: State<Float>) {
    Box(
        modifier = Modifier
            .size(10.dp)
            .drawBehind {
                val pulse = emphasis.value
                drawCircle(
                    color = accent.copy(alpha = 0.78f + pulse * 0.16f),
                    radius = (7.5f + pulse * 2.5f).dp.toPx() / 2f
                )
            }
    )
}"####,
        "fixed-layout status light",
    )?;

    replace_once(
        &mut rendered,
        "private fun rememberRunningEmphasis(enabled: Boolean): Float {",
        "private fun rememberRunningEmphasis(enabled: Boolean): State<Float> {",
        "stable running emphasis return type",
    )?;
    replace_once(
        &mut rendered,
        "    return emphasis.value\n}\n\n@Composable\nprivate fun rememberNowState(",
        "    return emphasis.asState()\n}\n\n@Composable\nprivate fun rememberNowState(",
        "draw-phase running emphasis state",
    )?;

    Ok(rendered)
}

fn render_repository(base: &str) -> Result<String, String> {
    let mut rendered = base.to_string();
    replace_once(
        &mut rendered,
        r####"            val updated = runCatching {
                stampChangedFieldRevisions(
                    previous = current,
                    next = transform(current),
                    mutationAt = now()
                ).sanitized()
            }.getOrElse { throwable ->"####,
        r####"            val updated = runCatching {
                withContext(Dispatchers.Default) {
                    stampChangedFieldRevisions(
                        previous = current,
                        next = transform(current),
                        mutationAt = now()
                    ).sanitized()
                }
            }.getOrElse { throwable ->"####,
        "repository transform off main",
    )?;
    replace_once(
        &mut rendered,
        "            transformFailure?.let { failure ->\n                attempt = failure\n                return@withLock\n            }\n            if (updated == current) {",
        "            transformFailure?.let { failure ->\n                attempt = failure\n                return@withLock\n            }\n            val updatedChanged = withContext(Dispatchers.Default) { updated != current }\n            if (!updatedChanged) {",
        "repository equality off main",
    )?;
    Ok(rendered)
}

fn render_sync_route(base: &str) -> Result<String, String> {
    let mut rendered = base.to_string();
    replace_once(
        &mut rendered,
        "            for (candidate in SyncRemoteRendezvous.candidates(log)) {",
        "            for (candidate in SyncRemoteRendezvous.candidates(context, log)) {",
        "context-bound signed route candidates",
    )?;
    replace_once(
        &mut rendered,
        "                ) ?: continue\n                return Resolution(serverUrl = target, rememberedServerUrl = target)",
        "                ) ?: continue\n                if (!SyncRemoteRendezvous.acceptCandidate(context, candidate, log)) {\n                    continue\n                }\n                return Resolution(serverUrl = target, rememberedServerUrl = target)",
        "durable signed route acceptance",
    )?;
    Ok(rendered)
}

fn render_micro_breaks(base: &str) -> Result<String, String> {
    let mut rendered = base.to_string();
    replace_once(
        &mut rendered,
        "private const val NATIVE_MICRO_BREAK_TRANSITION_FOCUS_RESUMED = 1\n",
        "private const val NATIVE_MICRO_BREAK_TRANSITION_FOCUS_RESUMED = 1\nprivate const val MICRO_BREAK_RESOLUTION_MAX_PHASE_STEPS = 128\n",
        "bounded micro-break fallback constant",
    )?;
    replace_once(
        &mut rendered,
        "    var latestTransitionAt: Long? = null\n\n    while (true) {",
        "    var latestTransitionAt: Long? = null\n    var phaseSteps = 0\n\n    while (true) {",
        "bounded micro-break fallback counter",
    )?;
    replace_once(
        &mut rendered,
        "                latestTransitionAt = activeSegmentStart\n            }\n            continue\n",
        "                latestTransitionAt = activeSegmentStart\n                phaseSteps += 1\n                if (phaseSteps >= MICRO_BREAK_RESOLUTION_MAX_PHASE_STEPS) {\n                    break\n                }\n            }\n            continue\n",
        "bounded micro-break zero-length transition",
    )?;
    replace_once(
        &mut rendered,
        "        latestTransitionAt = transitionAt\n        phaseProgress = 0L\n        activeSegmentStart = transitionAt\n        remainingElapsed -= remainingInPhase\n",
        "        latestTransitionAt = transitionAt\n        phaseProgress = 0L\n        activeSegmentStart = transitionAt\n        remainingElapsed -= remainingInPhase\n        phaseSteps += 1\n        if (phaseSteps >= MICRO_BREAK_RESOLUTION_MAX_PHASE_STEPS) {\n            break\n        }\n",
        "bounded micro-break phase transition",
    )?;
    Ok(rendered)
}

fn render_home_trace(base: &str) -> Result<String, String> {
    let mut rendered = base.to_string();
    replace_once(
        &mut rendered,
        "import android.os.SystemClock\n",
        "import android.os.SystemClock\nimport com.ofairyo.gridtimer.BuildConfig\n",
        "release trace build import",
    )?;
    replace_once(
        &mut rendered,
        "internal object HomePerformanceTrace {\n",
        "internal object HomePerformanceTrace {\n    internal val enabled: Boolean get() = BuildConfig.DEBUG\n",
        "release trace gate",
    )?;
    replace_once(
        &mut rendered,
        "    fun shouldSampleUiIndexComparison(): Boolean {\n        return synchronized(lock) {",
        "    fun shouldSampleUiIndexComparison(): Boolean {\n        if (!enabled) return false\n        return synchronized(lock) {",
        "release index trace gate",
    )?;
    replace_once(
        &mut rendered,
        "    ) {\n        synchronized(lock) {\n            if (uiIndexComparisonSampleCount >= MAX_UI_INDEX_COMPARISON_SAMPLES) {",
        "    ) {\n        if (!enabled) return\n        synchronized(lock) {\n            if (uiIndexComparisonSampleCount >= MAX_UI_INDEX_COMPARISON_SAMPLES) {",
        "release index trace sink gate",
    )?;
    replace_once(
        &mut rendered,
        "    ) {\n        val nowMillis = SystemClock.elapsedRealtime()\n        val comparison = synchronized(lock) {",
        "    ) {\n        if (!enabled) return\n        val nowMillis = SystemClock.elapsedRealtime()\n        val comparison = synchronized(lock) {",
        "release recompose trace sink gate",
    )?;
    Ok(rendered)
}
