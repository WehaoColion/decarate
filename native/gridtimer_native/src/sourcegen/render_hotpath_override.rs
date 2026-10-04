// v2.22.27 - Move timer CPU work off the UI thread, bound exact catch-up, and cache static dial geometry.

pub const SCREEN_PATH: &str = "com/ofairyo/gridtimer/ui/GridTimerScreen.kt";
pub const REPOSITORY_PATH: &str = "com/ofairyo/gridtimer/data/TimerRepository.kt";
pub const DIAGNOSTIC_LOG_PATH: &str = "com/ofairyo/gridtimer/diagnostics/DiagnosticLogStore.kt";
pub const HOME_TRACE_PATH: &str = "com/ofairyo/gridtimer/ui/HomePerformanceTrace.kt";
pub const MICRO_BREAKS_PATH: &str = "com/ofairyo/gridtimer/data/MicroBreaks.kt";
pub const MODELS_PATH: &str = "com/ofairyo/gridtimer/data/Models.kt";
pub const APP_LANGUAGE_PATH: &str = "com/ofairyo/gridtimer/ui/AppLanguageSupport.kt";
pub const VIEW_MODEL_PATH: &str = "com/ofairyo/gridtimer/ui/TimerViewModel.kt";
pub const BRIDGE_PATH_PREFIX: &str = "com/ofairyo/gridtimer/core/";
#[cfg(test)]
pub const BRIDGE_PATH: &str = "com/ofairyo/gridtimer/core/NativeOptimizerBridge.kt";
pub const UI_INDEXES_PATH: &str = "com/ofairyo/gridtimer/ui/UiIndexes.kt";

pub fn render(path: &str, base: &str) -> Result<String, String> {
    match path {
        SCREEN_PATH => render_screen(base),
        REPOSITORY_PATH => render_repository(base),
        DIAGNOSTIC_LOG_PATH => render_diagnostic_log(base),
        HOME_TRACE_PATH => render_home_trace(base),
        MICRO_BREAKS_PATH => render_micro_breaks(base),
        MODELS_PATH => render_models(base),
        APP_LANGUAGE_PATH => render_app_language(base),
        VIEW_MODEL_PATH => render_view_model(base),
        path if path.starts_with(BRIDGE_PATH_PREFIX) &&
            path.ends_with("NativeOptimizerBridge.kt") => render_native_bridge(base),
        UI_INDEXES_PATH => render_ui_indexes(base),
        _ => Ok(base.to_string()),
    }
}

fn replace_exactly_once(
    target: &mut String,
    old: &str,
    new: &str,
    label: &str,
) -> Result<(), String> {
    let matches = target.match_indices(old).count();
    if matches != 1 {
        return Err(format!(
            "expected exactly one {label} fragment in render-hotpath source, found {matches}"
        ));
    }
    *target = target.replacen(old, new, 1);
    Ok(())
}

fn replace_section_exactly_once(
    target: &mut String,
    start: &str,
    end: &str,
    replacement: &str,
    label: &str,
) -> Result<(), String> {
    let start_matches = target.match_indices(start).count();
    if start_matches != 1 {
        return Err(format!(
            "expected exactly one {label} start fragment, found {start_matches}"
        ));
    }
    let end_matches = target.match_indices(end).count();
    if end_matches != 1 {
        return Err(format!(
            "expected exactly one {label} end fragment, found {end_matches}"
        ));
    }
    let start_index = target
        .find(start)
        .ok_or_else(|| format!("missing {label} start fragment"))?;
    let end_index = target[start_index..]
        .find(end)
        .map(|offset| start_index + offset)
        .ok_or_else(|| format!("missing {label} end fragment after start"))?;
    if end_index <= start_index {
        return Err(format!("invalid {label} section bounds"));
    }
    target.replace_range(start_index..end_index, replacement);
    Ok(())
}

fn render_models(base: &str) -> Result<String, String> {
    let mut rendered = base.to_string();

    replace_exactly_once(
        &mut rendered,
        r####"enum class ThemeMode {
    SYSTEM,
    LIGHT,
    DARK
}"####,
        r####"enum class ThemeMode {
    SYSTEM,
    LIGHT,
    DARK,
    OLED
}"####,
        "OLED theme mode",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"    val themeMode: ThemeMode = ThemeMode.SYSTEM
) {
    companion object {"####,
        r####"    val themeMode: ThemeMode = ThemeMode.SYSTEM
) {
    val resolvedThemeMode: ThemeMode
        get() = themeMode

    companion object {"####,
        "resolved theme mode",
    )?;

    Ok(rendered)
}

fn render_app_language(base: &str) -> Result<String, String> {
    let mut rendered = base.to_string();
    replace_exactly_once(
        &mut rendered,
        "        ThemeMode.DARK -> strings.themeDarkLabel\n",
        "        ThemeMode.DARK -> strings.themeDarkLabel\n        ThemeMode.OLED -> \"OLED 纯黑\"\n",
        "OLED theme label",
    )?;
    Ok(rendered)
}

fn render_view_model(base: &str) -> Result<String, String> {
    let mut rendered = base.to_string();
    replace_exactly_once(
        &mut rendered,
        "            ThemeMode.DARK -> ThemeMode.SYSTEM\n",
        "            ThemeMode.DARK -> ThemeMode.OLED\n            ThemeMode.OLED -> ThemeMode.SYSTEM\n",
        "OLED theme cycle",
    )?;
    Ok(rendered)
}

fn render_screen(base: &str) -> Result<String, String> {
    let mut rendered = base.to_string();

    replace_exactly_once(
        &mut rendered,
        "import androidx.compose.runtime.SideEffect\nimport androidx.compose.runtime.collectAsState",
        "import androidx.compose.runtime.SideEffect\nimport androidx.compose.runtime.State\nimport androidx.compose.runtime.collectAsState",
        "running emphasis State import",
    )?;


    replace_exactly_once(
        &mut rendered,
        "import androidx.compose.ui.draw.clip\nimport androidx.compose.ui.draw.drawBehind",
        "import androidx.compose.ui.draw.clip\nimport androidx.compose.ui.draw.drawBehind\nimport androidx.compose.ui.draw.drawWithCache",
        "drawWithCache import",
    )?;

    replace_exactly_once(
        &mut rendered,
        "import androidx.compose.ui.graphics.Brush\nimport androidx.compose.ui.graphics.Color",
        "import androidx.compose.ui.graphics.Brush\nimport androidx.compose.ui.graphics.Color\nimport androidx.compose.ui.graphics.Path",
        "Path import",
    )?;

    replace_exactly_once(
        &mut rendered,
        "    val compositionStartNanos = SystemClock.elapsedRealtimeNanos()",
        "    val compositionStartNanos = if (HomePerformanceTrace.enabled) {\n        SystemClock.elapsedRealtimeNanos()\n    } else {\n        0L\n    }",
        "release-gated composition timestamp",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"    SideEffect {
        HomePerformanceTrace.recordHomeRecompose(
            context = context.applicationContext,
            compositionStartNanos = compositionStartNanos,
            appData = appData,
            overlayDepth = overlayDepth,
            selectedSlotVisible = detailOverlayVisible
        )
    }
"####,
        r####"    if (HomePerformanceTrace.enabled) {
        SideEffect {
            HomePerformanceTrace.recordHomeRecompose(
                context = context.applicationContext,
                compositionStartNanos = compositionStartNanos,
                appData = appData,
                overlayDepth = overlayDepth,
                selectedSlotVisible = detailOverlayVisible
            )
        }
    }
"####,
        "release-gated home recompose trace",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"    val lightBackdrop = MaterialTheme.colorScheme.background.red > 0.5f
    val runningPulse = rememberRunningEmphasis(enabled = hasRunningSlots)
"####,
        r####"    val lightBackdrop = MaterialTheme.colorScheme.background.red > 0.5f
    val runningAccent = MaterialTheme.colorScheme.primary
    val runningPulse = rememberRunningEmphasis(enabled = hasRunningSlots)
"####,
        "ambient running accent capture",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"    if (hasRunningSlots || runningPulse > 0f) {
        Box(
            modifier = Modifier
                .fillMaxSize()
                .background(
                    Brush.radialGradient(
                        colors = listOf(
                            MaterialTheme.colorScheme.primary.copy(
                                alpha = if (lightBackdrop) {
                                    0.035f + runningPulse * 0.030f
                                } else {
                                    0.060f + runningPulse * 0.045f
                                }
                            ),
                            Color.Transparent
                        ),
                        center = Offset(540f, 1_640f),
                        radius = 420f + runningPulse * 180f
                    )
                )
        )
    }
"####,
        r####"    Box(
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
"####,
        "draw-phase ambient running glow",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"    val runningPulse = rememberRunningEmphasis(enabled = activeCount > 0)
    val dockSurfaceColor = MaterialTheme.colorScheme.panel
    val selectedIndicatorColor = selectedAccent.mixOverDock(dockSurfaceColor, 0.16f + runningPulse * 0.04f)
"####,
        r####"    val runningPulse = rememberRunningEmphasis(enabled = activeCount > 0)
    val dockSurfaceColor = MaterialTheme.colorScheme.panel
    val selectedIndicatorBaseColor = selectedAccent.mixOverDock(dockSurfaceColor, 0.16f)
"####,
        "static dock indicator base color",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"                            color = selectedIndicatorColor,
                            shadowElevation = indicatorLift,
                            border = BorderStroke(1.dp, selectedAccent.copy(alpha = 0.20f))
                        ) {
                            Box(
                                modifier = Modifier
                                    .fillMaxSize()
                                    .clip(RoundedCornerShape(24.dp))
                            )
                        }
"####,
        r####"                            color = selectedIndicatorBaseColor,
                            shadowElevation = indicatorLift,
                            border = BorderStroke(1.dp, selectedAccent.copy(alpha = 0.20f))
                        ) {
                            Box(
                                modifier = Modifier
                                    .fillMaxSize()
                                    .clip(RoundedCornerShape(24.dp))
                                    .graphicsLayer {
                                        alpha = (runningPulse.value / 21f).coerceIn(0f, 1f)
                                    }
                                    .background(selectedAccent)
                            )
                        }
"####,
        "layer-phase dock indicator pulse",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"            Box(
                modifier = Modifier
                    .matchParentSize()
                    .background(
                        Brush.verticalGradient(
                            colors = listOf(
                                Color.White.copy(alpha = 0.10f),
                                selectedAccent.copy(alpha = 0.055f + runningPulse * 0.020f),
                                Color.Transparent
                            )
                        )
                    )
            )
"####,
        r####"            Box(
                modifier = Modifier
                    .matchParentSize()
                    .drawBehind {
                        val pulse = runningPulse.value
                        drawRect(
                            brush = Brush.verticalGradient(
                                colors = listOf(
                                    Color.White.copy(alpha = 0.10f),
                                    selectedAccent.copy(alpha = 0.055f + pulse * 0.020f),
                                    Color.Transparent
                                )
                            )
                        )
                    }
            )
"####,
        "draw-phase dock vertical glow",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"            Box(
                modifier = Modifier
                    .matchParentSize()
                    .background(
                        Brush.radialGradient(
                            colors = listOf(
                                selectedAccent.copy(alpha = 0.07f + runningPulse * 0.03f),
                                Color.Transparent
                            ),
                            center = Offset(180f, 30f),
                            radius = 420f + runningPulse * 140f
                        )
                    )
            )
"####,
        r####"            Box(
                modifier = Modifier
                    .matchParentSize()
                    .drawBehind {
                        val pulse = runningPulse.value
                        drawRect(
                            brush = Brush.radialGradient(
                                colors = listOf(
                                    selectedAccent.copy(alpha = 0.07f + pulse * 0.03f),
                                    Color.Transparent
                                ),
                                center = Offset(180f, 30f),
                                radius = 420f + pulse * 140f
                            )
                        )
                    }
            )
"####,
        "draw-phase dock radial glow",
    )?;

    replace_exactly_once(
        &mut rendered,
        "    livePulse: Float = 0f,",
        "    livePulse: State<Float>? = null,",
        "stable dock live pulse state",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"                if (livePulse > 0f && testTag == "home_nav_board") {
                    Box(
                        modifier = Modifier
                            .size((12.5f + livePulse * 5f).dp)
                            .clip(CircleShape)
                            .background(accent.copy(alpha = if (selected) 0.24f else 0.16f + livePulse * 0.10f))
                    )
                }
"####,
        r####"                if (testTag == "home_nav_board") {
                    livePulse?.let { emphasis ->
                        Box(
                            modifier = Modifier
                                .size(17.5.dp)
                                .graphicsLayer {
                                    val pulse = emphasis.value
                                    val diameter = 12.5f + pulse * 5f
                                    scaleX = diameter / 17.5f
                                    scaleY = scaleX
                                    alpha = when {
                                        pulse <= 0f -> 0f
                                        selected -> 0.24f
                                        else -> 0.16f + pulse * 0.10f
                                    }
                                }
                                .clip(CircleShape)
                                .background(accent)
                        )
                    }
                }
"####,
        "fixed-layout dock running marker",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"                    Box(
                        modifier = Modifier
                            .fillMaxWidth()
                            .height(3.dp)
                            .background(
                                Brush.horizontalGradient(
                                    colors = listOf(
                                        Color.Transparent,
                                        categoryAccent.copy(alpha = 0.16f + runningHighlight * 0.24f),
                                        Color.Transparent
                                    )
                                )
                            )
                    )
"####,
        r####"                    Box(
                        modifier = Modifier
                            .fillMaxWidth()
                            .height(3.dp)
                            .graphicsLayer {
                                alpha = 0.16f + runningHighlight.value * 0.24f
                            }
                            .background(
                                Brush.horizontalGradient(
                                    colors = listOf(
                                        Color.Transparent,
                                        categoryAccent,
                                        Color.Transparent
                                    )
                                )
                            )
                    )
"####,
        "layer-phase timer tile running highlight",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"private fun StatusLight(accent: Color, emphasis: Float) {
    Box(
        modifier = Modifier
            .size((7.5f + emphasis * 2.5f).dp)
            .clip(CircleShape)
            .background(accent.copy(alpha = 0.78f + emphasis * 0.16f))
    )
}
"####,
        r####"private fun StatusLight(accent: Color, emphasis: State<Float>) {
    Box(
        modifier = Modifier
            .size(10.dp)
            .graphicsLayer {
                val pulse = emphasis.value
                val diameter = 7.5f + pulse * 2.5f
                scaleX = diameter / 10f
                scaleY = scaleX
                alpha = 0.78f + pulse * 0.16f
            }
            .clip(CircleShape)
            .background(accent)
    )
}
"####,
        "fixed-layout timer status light",
    )?;

    replace_exactly_once(
        &mut rendered,
        "private fun rememberRunningEmphasis(enabled: Boolean): Float {",
        "private fun rememberRunningEmphasis(enabled: Boolean): State<Float> {",
        "stable running emphasis return type",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"    return emphasis.value
}

@Composable
private fun rememberNowState(
"####,
        r####"    return emphasis.asState()
}

@Composable
private fun rememberNowState(
"####,
        "stable running emphasis state",
    )?;


    replace_section_exactly_once(
        &mut rendered,
        "@Composable\nprivate fun TimerDial(",
        "\n\n@Composable\nprivate fun rememberRunningEmphasis",
        r####"@Composable
private fun TimerDial(
    accent: Color,
    elapsedMillis: Long,
    isRunning: Boolean,
    modifier: Modifier = Modifier
) {
    val dialOuter = if (isRunning) accent.copy(alpha = 0.18f) else MaterialTheme.colorScheme.outline.copy(alpha = 0.10f)
    val dialTick = MaterialTheme.colorScheme.onSurface.copy(alpha = if (isRunning) 0.36f else 0.24f)
    val dialSurface = MaterialTheme.colorScheme.surface.copy(alpha = 0.97f)
    val dialBright = MaterialTheme.colorScheme.surfaceBright
    val dialFaceInner = MaterialTheme.colorScheme.surfaceContainerHigh
    val dialHighlight = MaterialTheme.colorScheme.topHighlight
    val dialChrome = MaterialTheme.colorScheme.chrome
    val dialShadow = MaterialTheme.colorScheme.depthShadow.copy(alpha = if (isRunning) 0.16f else 0.10f)
    val sweepAlpha = if (isRunning) 0.62f else 0.16f
    val secondFraction = ((elapsedMillis % 60_000L).toFloat() / 60_000f).coerceIn(0f, 1f)
    val sweepAngle = if (isRunning) secondFraction * 360f else 208f

    Canvas(
        modifier = modifier.drawWithCache {
            val radius = min(size.width, size.height) / 2f
            val outerRadius = radius * 0.94f
            val bezelRadius = radius * 0.82f
            val dialFaceRadius = radius * 0.62f
            val arcRadius = radius * 0.73f
            val dialCenter = Offset(size.width / 2f, size.height / 2f)
            val outerBrush = Brush.radialGradient(
                colors = listOf(
                    dialBright,
                    dialSurface,
                    dialChrome.copy(alpha = 0.98f),
                    dialShadow.copy(alpha = 0.78f)
                ),
                center = Offset(
                    dialCenter.x - radius * 0.18f,
                    dialCenter.y - radius * 0.22f
                ),
                radius = radius
            )
            val majorTicks = Path()
            val mediumTicks = Path()
            val minorEvenTicks = Path()
            val minorOddTicks = Path()
            for (tick in 0 until 60) {
                val angle = (tick / 60f) * (2f * PI.toFloat()) - (PI.toFloat() / 2f)
                val tickTier = when {
                    tick % 15 == 0 -> 0
                    tick % 5 == 0 -> 1
                    else -> 2
                }
                val outerScale = when (tickTier) {
                    0 -> 0.76f
                    1 -> 0.75f
                    else -> 0.735f
                }
                val outer = Offset(
                    x = dialCenter.x + cos(angle) * radius * outerScale,
                    y = dialCenter.y + sin(angle) * radius * outerScale
                )
                val innerScale = when (tickTier) {
                    0 -> 0.42f
                    1 -> 0.52f
                    else -> 0.61f
                }
                val inner = Offset(
                    x = dialCenter.x + cos(angle) * radius * innerScale,
                    y = dialCenter.y + sin(angle) * radius * innerScale
                )
                val path = when (tickTier) {
                    0 -> majorTicks
                    1 -> mediumTicks
                    else -> if (tick % 2 == 0) minorEvenTicks else minorOddTicks
                }
                path.moveTo(inner.x, inner.y)
                path.lineTo(outer.x, outer.y)
            }
            val arcTopLeft = Offset(dialCenter.x - arcRadius, dialCenter.y - arcRadius)
            val arcSize = Size(arcRadius * 2f, arcRadius * 2f)

            onDrawBehind {
                drawCircle(
                    brush = outerBrush,
                    radius = outerRadius
                )
                drawCircle(
                    color = dialHighlight.copy(alpha = 0.32f),
                    radius = outerRadius,
                    style = Stroke(width = radius * 0.02f)
                )
                drawCircle(
                    color = dialOuter,
                    radius = bezelRadius,
                    style = Stroke(width = radius * 0.11f)
                )
                drawCircle(
                    color = dialSurface.copy(alpha = 0.98f),
                    radius = dialFaceRadius
                )
                drawCircle(
                    color = dialFaceInner.copy(alpha = 0.92f),
                    radius = dialFaceRadius * 0.82f
                )
                drawCircle(
                    color = dialShadow.copy(alpha = 0.16f),
                    radius = dialFaceRadius,
                    style = Stroke(width = radius * 0.012f)
                )

                drawPath(
                    path = majorTicks,
                    color = if (isRunning) accent.copy(alpha = 0.46f) else dialTick.copy(alpha = 0.34f),
                    style = Stroke(width = radius * 0.034f, cap = StrokeCap.Round)
                )
                drawPath(
                    path = mediumTicks,
                    color = dialTick.copy(alpha = if (isRunning) 0.34f else 0.25f),
                    style = Stroke(width = radius * 0.023f, cap = StrokeCap.Round)
                )
                drawPath(
                    path = minorEvenTicks,
                    color = dialTick.copy(alpha = 0.16f),
                    style = Stroke(width = radius * 0.010f, cap = StrokeCap.Round)
                )
                drawPath(
                    path = minorOddTicks,
                    color = dialTick.copy(alpha = 0.11f),
                    style = Stroke(width = radius * 0.010f, cap = StrokeCap.Round)
                )

                drawArc(
                    color = dialTick.copy(alpha = 0.08f),
                    startAngle = -90f,
                    sweepAngle = 360f,
                    useCenter = false,
                    topLeft = arcTopLeft,
                    size = arcSize,
                    style = Stroke(width = radius * 0.07f, cap = StrokeCap.Round)
                )
                drawArc(
                    color = accent.copy(alpha = sweepAlpha),
                    startAngle = -90f,
                    sweepAngle = sweepAngle,
                    useCenter = false,
                    topLeft = arcTopLeft,
                    size = arcSize,
                    style = Stroke(width = radius * 0.082f, cap = StrokeCap.Round)
                )

                val orbitRadians = (-90f + sweepAngle) / 180f * PI.toFloat()
                val orbitCenter = Offset(
                    x = dialCenter.x + cos(orbitRadians) * arcRadius,
                    y = dialCenter.y + sin(orbitRadians) * arcRadius
                )
                drawCircle(
                    color = accent.copy(alpha = if (isRunning) 0.90f else 0.42f),
                    center = orbitCenter,
                    radius = radius * 0.044f
                )
                drawCircle(
                    color = dialBright.copy(alpha = 0.82f),
                    center = orbitCenter,
                    radius = radius * 0.018f
                )

                rotate(degrees = -90f + secondFraction * 360f, pivot = dialCenter) {
                    drawLine(
                        color = if (isRunning) accent else dialTick.copy(alpha = 0.56f),
                        start = dialCenter,
                        end = Offset(dialCenter.x, dialCenter.y - radius * 0.50f),
                        strokeWidth = radius * 0.046f,
                        cap = StrokeCap.Round
                    )
                    drawLine(
                        color = dialTick.copy(alpha = 0.28f),
                        start = dialCenter,
                        end = Offset(dialCenter.x, dialCenter.y + radius * 0.22f),
                        strokeWidth = radius * 0.022f,
                        cap = StrokeCap.Round
                    )
                    drawLine(
                        color = dialHighlight.copy(alpha = 0.46f),
                        start = Offset(dialCenter.x, dialCenter.y - radius * 0.08f),
                        end = Offset(dialCenter.x, dialCenter.y - radius * 0.46f),
                        strokeWidth = radius * 0.012f,
                        cap = StrokeCap.Round
                    )
                }

                drawCircle(color = accent.copy(alpha = 0.20f), radius = radius * 0.17f)
                drawCircle(color = dialBright.copy(alpha = 0.62f), radius = radius * 0.11f)
                drawCircle(
                    color = if (isRunning) accent else dialTick.copy(alpha = 0.45f),
                    radius = radius * 0.075f
                )
            }
        }
    ) {}
}"####,
        "TimerDial draw cache",
    )?;

    replace_exactly_once(
        &mut rendered,
        "        ThemeMode.DARK -> true\n",
        "        ThemeMode.DARK, ThemeMode.OLED -> true\n",
        "OLED dark theme branch",
    )?;

    let dark_accent_branch = "        ThemeMode.DARK -> Color(0xFF58A3DA)\n";
    if rendered.match_indices(dark_accent_branch).count() != 2 {
        return Err("expected two theme accent branches in GridTimerScreen source".to_string());
    }
    rendered = rendered.replace(
        dark_accent_branch,
        "        ThemeMode.DARK -> Color(0xFF58A3DA)\n        ThemeMode.OLED -> Color(0xFF58A3DA)\n",
    );

    replace_exactly_once(
        &mut rendered,
        "        ThemeMode.DARK -> Icons.Rounded.DarkMode\n",
        "        ThemeMode.DARK -> Icons.Rounded.DarkMode\n        ThemeMode.OLED -> Icons.Rounded.DarkMode\n",
        "OLED theme icon",
    )?;

    replace_exactly_once(
        &mut rendered,
        "            listOf(ThemeMode.SYSTEM, ThemeMode.LIGHT, ThemeMode.DARK).forEach { mode ->\n",
        "            listOf(ThemeMode.SYSTEM, ThemeMode.LIGHT, ThemeMode.DARK, ThemeMode.OLED).forEach { mode ->\n",
        "OLED theme menu option",
    )?;
    Ok(rendered)
}

fn render_home_trace(base: &str) -> Result<String, String> {
    let mut rendered = base.to_string();

    replace_exactly_once(
        &mut rendered,
        "import android.os.SystemClock\nimport com.ofairyo.gridtimer.data.AppData",
        "import android.os.SystemClock\nimport com.ofairyo.gridtimer.BuildConfig\nimport com.ofairyo.gridtimer.data.AppData",
        "HomePerformanceTrace BuildConfig import",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"internal object HomePerformanceTrace {
    private const val MAX_RECOMPOSE_SAMPLES = 18
"####,
        r####"internal object HomePerformanceTrace {
    val enabled: Boolean
        get() = BuildConfig.DEBUG

    private const val MAX_RECOMPOSE_SAMPLES = 18
"####,
        "HomePerformanceTrace release gate",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"    fun shouldSampleUiIndexComparison(): Boolean {
        return synchronized(lock) {
"####,
        r####"    fun shouldSampleUiIndexComparison(): Boolean {
        if (!enabled) {
            return false
        }
        return synchronized(lock) {
"####,
        "release-gated UI index comparison",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"    fun recordUiIndexComparison(
        nativeElapsedNanos: Long,
        kotlinBaselineElapsedNanos: Long,
        slotCount: Int,
        sessionCount: Int,
        archivedTaskCount: Int
    ) {
        synchronized(lock) {
"####,
        r####"    fun recordUiIndexComparison(
        nativeElapsedNanos: Long,
        kotlinBaselineElapsedNanos: Long,
        slotCount: Int,
        sessionCount: Int,
        archivedTaskCount: Int
    ) {
        if (!enabled) {
            return
        }
        synchronized(lock) {
"####,
        "release-gated UI index comparison record",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"    fun recordHomeRecompose(
        context: Context,
        compositionStartNanos: Long,
        appData: AppData,
        overlayDepth: Int,
        selectedSlotVisible: Boolean
    ) {
        val nowMillis = SystemClock.elapsedRealtime()
"####,
        r####"    fun recordHomeRecompose(
        context: Context,
        compositionStartNanos: Long,
        appData: AppData,
        overlayDepth: Int,
        selectedSlotVisible: Boolean
    ) {
        if (!enabled) {
            return
        }
        val nowMillis = SystemClock.elapsedRealtime()
"####,
        "release-gated home trace record",
    )?;

    Ok(rendered)
}

fn render_repository(base: &str) -> Result<String, String> {
    let mut rendered = base.to_string();

    replace_exactly_once(
        &mut rendered,
        r####"    private suspend fun monitorMicroBreaks() {
        _appData.collectLatest { snapshot ->
            var current = snapshot
            while (true) {
                val nextDelay = current.nextMicroBreakDelayMillis(now()) ?: break
                if (nextDelay > 0L) {
                    delay(nextDelay)
                }
                advanceMicroBreaks(now())
                current = _appData.value
            }
        }
    }
"####,
        r####"    private suspend fun monitorMicroBreaks() {
        _appData.collectLatest { snapshot ->
            var current = snapshot
            var stalledRetryMillis = 1_000L
            var catchUpCutoff: Long? = null
            while (true) {
                val evaluationNow = catchUpCutoff ?: now()
                val nextDelay = current.nextMicroBreakDelayMillis(evaluationNow) ?: break
                if (nextDelay > 0L) {
                    delay(nextDelay)
                    current = _appData.value
                    catchUpCutoff = null
                    stalledRetryMillis = 1_000L
                    continue
                }
                catchUpCutoff = evaluationNow
                val progressed = advanceMicroBreaks(evaluationNow)
                current = _appData.value
                if (progressed) {
                    stalledRetryMillis = 1_000L
                } else {
                    delay(stalledRetryMillis)
                    stalledRetryMillis = (stalledRetryMillis * 2L).coerceAtMost(4_000L)
                    catchUpCutoff = null
                }
            }
        }
    }
"####,
        "fixed cutoff micro-break monitor",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"    private suspend fun advanceMicroBreaks(
        now: Long,
        alertFreshnessMillis: Long = MICRO_BREAK_ALERT_FRESHNESS_MILLIS,
        latestTransitionOnly: Boolean = false,
        awaitBell: Boolean = false,
        source: String = "monitor"
    ) {
        var freshTransitions = emptyList<MicroBreakTransition>()
        updateData { data ->
            val resolution = data.resolveMicroBreaks(now)
            val candidates = resolution.transitions.filter { transition ->
                (now - transition.occurredAtEpochMillis) in 0L..alertFreshnessMillis
            }
            freshTransitions = if (latestTransitionOnly) candidates.takeLast(1) else candidates
            resolution.data
        }
        freshTransitions.forEach { transition ->
            logDiagnosticEvent(
                category = "slot.micro_break",
                message =
                    "source=$source " +
                    "Transition=${transition.type} " +
                        "slotId=${transition.slotId} " +
                        "occurredAt=${transition.occurredAtEpochMillis}"
            )
            if (awaitBell) {
                MicroBreakReminderNotifier.notifyTransitionAndAwaitBell(appContext, transition)
            } else {
                MicroBreakReminderNotifier.notifyTransition(appContext, transition)
            }
        }
        syncTimerLiveUpdate()
    }
"####,
        r####"    private suspend fun advanceMicroBreaks(
        now: Long,
        alertFreshnessMillis: Long = MICRO_BREAK_ALERT_FRESHNESS_MILLIS,
        latestTransitionOnly: Boolean = false,
        awaitBell: Boolean = false,
        source: String = "monitor"
    ): Boolean {
        var freshTransitions = emptyList<MicroBreakTransition>()
        var resolutionChanged = false
        updateData { data ->
            val resolution = data.resolveMicroBreaks(now)
            resolutionChanged = resolution.data !== data
            val candidates = resolution.transitions.filter { transition ->
                (now - transition.occurredAtEpochMillis) in 0L..alertFreshnessMillis
            }
            freshTransitions = if (latestTransitionOnly) candidates.takeLast(1) else candidates
            resolution.data
        }
        if (!resolutionChanged) {
            return false
        }
        freshTransitions.forEach { transition ->
            logDiagnosticEvent(
                category = "slot.micro_break",
                message =
                    "source=$source " +
                    "Transition=${transition.type} " +
                         "slotId=${transition.slotId} " +
                         "occurredAt=${transition.occurredAtEpochMillis}"
            )
            if (awaitBell) {
                MicroBreakReminderNotifier.notifyTransitionAndAwaitBell(appContext, transition)
            } else {
                MicroBreakReminderNotifier.notifyTransition(appContext, transition)
            }
        }
        syncTimerLiveUpdate()
        return true
    }
"####,
        "bounded micro-break progress result",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"            val updated = runCatching {
                transform(current).sanitized()
            }.getOrElse { throwable ->
"####,
        r####"            val updated = runCatching {
                withContext(Dispatchers.Default) {
                    transform(current).sanitized()
                }
            }.getOrElse { throwable ->
"####,
        "repository default CPU dispatcher",
    )?;

    replace_exactly_once(
        &mut rendered,
        "            if (updated == current) {\n",
        "            val changed = withContext(Dispatchers.Default) {\n                updated != current\n            }\n            if (!changed) {\n",
        "repository background equality",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"        val safeTitle = NativeOptimizerBridge.normalizeRepositoryText(
            value = title,
            maxLength = 24,
            trimStartOnly = true,
            compactWhitespace = false
        ) ?: title.trimStart().take(24)
"####,
        r####"        val safeTitle = withContext(Dispatchers.Default) {
            NativeOptimizerBridge.normalizeRepositoryText(
                value = title,
                maxLength = 24,
                trimStartOnly = true,
                compactWhitespace = false
            ) ?: title.trimStart().take(24)
        }
"####,
        "title normalization dispatcher",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"        val safeNote = NativeOptimizerBridge.normalizeRepositoryText(
            value = note,
            maxLength = 60,
            trimStartOnly = true,
            compactWhitespace = false
        ) ?: note.trimStart().take(60)
"####,
        r####"        val safeNote = withContext(Dispatchers.Default) {
            NativeOptimizerBridge.normalizeRepositoryText(
                value = note,
                maxLength = 60,
                trimStartOnly = true,
                compactWhitespace = false
            ) ?: note.trimStart().take(60)
        }
"####,
        "note normalization dispatcher",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"        val safeName = NativeOptimizerBridge.normalizeRepositoryText(
            value = rawName,
            maxLength = 12,
            trimStartOnly = false,
            compactWhitespace = true
        ) ?: rawName.trim().replace("\\s+".toRegex(), " ").take(12)
"####,
        r####"        val safeName = withContext(Dispatchers.Default) {
            NativeOptimizerBridge.normalizeRepositoryText(
                value = rawName,
                maxLength = 12,
                trimStartOnly = false,
                compactWhitespace = true
            ) ?: rawName.trim().replace("\\s+".toRegex(), " ").take(12)
        }
"####,
        "category normalization dispatcher",
    )?;

    Ok(rendered)
}

fn render_micro_breaks(base: &str) -> Result<String, String> {
    let mut rendered = base.to_string();

    replace_exactly_once(
        &mut rendered,
        "internal const val MICRO_BREAK_ALERT_FRESHNESS_MILLIS = 3_000L\n",
        "internal const val MICRO_BREAK_ALERT_FRESHNESS_MILLIS = 3_000L\nprivate const val MICRO_BREAK_RESOLUTION_MAX_PHASE_STEPS = 128\n",
        "micro-break exact page budget",
    )?;

    replace_exactly_once(
        &mut rendered,
        "internal data class TimerSlotMicroBreakResolution(\n    val slot: TimerSlot,\n    val sessions: List<TimerSession>,\n    val transitions: List<MicroBreakTransition>\n)\n\ninternal data class AppDataMicroBreakResolution(\n    val data: AppData,\n    val transitions: List<MicroBreakTransition>\n)",
        "internal data class TimerSlotMicroBreakResolution(\n    val slot: TimerSlot,\n    val sessions: List<TimerSession>,\n    val transitions: List<MicroBreakTransition>,\n    val complete: Boolean\n)\n\ninternal data class AppDataMicroBreakResolution(\n    val data: AppData,\n    val transitions: List<MicroBreakTransition>,\n    val complete: Boolean\n)",
        "micro-break resolution completion fields",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"            updatedAt = updatedAt
        ),
        sessions = sessions,
        transitions = transitions
    )
}"####,
        r####"        updatedAt = updatedAt
        ),
        sessions = sessions,
        transitions = transitions,
        complete = complete
    )
}"####,
        "native micro-break completion propagation",
    )?;

    replace_exactly_once(
        &mut rendered,
        "    val runningSince = normalized.runningSinceEpochMillis ?: return TimerSlotMicroBreakResolution(\n        slot = normalized,\n        sessions = emptyList(),\n        transitions = emptyList()\n    )",
        "    val runningSince = normalized.runningSinceEpochMillis ?: return TimerSlotMicroBreakResolution(\n        slot = normalized,\n        sessions = emptyList(),\n        transitions = emptyList(),\n        complete = true\n    )",
        "idle micro-break completion",
    )?;

    replace_exactly_once(
        &mut rendered,
        "    var latestTransitionAt: Long? = null\n\n    while (true) {",
        "    var latestTransitionAt: Long? = null\n    var phaseSteps = 0\n    var complete = true\n\n    while (true) {",
        "fallback micro-break page state",
    )?;

    replace_exactly_once(
        &mut rendered,
        "                latestTransitionAt = activeSegmentStart\n            }\n            continue\n",
        "                latestTransitionAt = activeSegmentStart\n                phaseSteps += 1\n                if (phaseSteps >= MICRO_BREAK_RESOLUTION_MAX_PHASE_STEPS) {\n                    complete = false\n                    break\n                }\n            }\n            continue\n",
        "fallback zero-length transition budget",
    )?;

    replace_exactly_once(
        &mut rendered,
        "        activeSegmentStart = transitionAt\n        remainingElapsed -= remainingInPhase\n    }\n\n    return TimerSlotMicroBreakResolution(",
        "        activeSegmentStart = transitionAt\n        remainingElapsed -= remainingInPhase\n        phaseSteps += 1\n        if (phaseSteps >= MICRO_BREAK_RESOLUTION_MAX_PHASE_STEPS) {\n            complete = false\n            break\n        }\n    }\n\n    return TimerSlotMicroBreakResolution(",
        "fallback phase transition budget",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"            microBreakPhaseProgressMillis = phaseProgress,
            updatedAt = latestTransitionAt?.coerceAtLeast(normalized.updatedAt) ?: normalized.updatedAt
        ),
        sessions = sessions,
        transitions = transitions
    )
}"####,
        r####"            microBreakPhaseProgressMillis = phaseProgress,
            updatedAt = latestTransitionAt?.coerceAtLeast(normalized.updatedAt) ?: normalized.updatedAt
        ),
        sessions = sessions,
        transitions = transitions,
        complete = complete
    )
}"####,
        "fallback micro-break completion",
    )?;

    replace_section_exactly_once(
        &mut rendered,
        "internal fun AppData.resolveMicroBreaks(now: Long = System.currentTimeMillis()): AppDataMicroBreakResolution {",
        "\n\ninternal fun TimerSlot.resolveMicroBreak(now: Long): TimerSlotMicroBreakResolution {",
        r####"internal fun AppData.resolveMicroBreaks(now: Long = System.currentTimeMillis()): AppDataMicroBreakResolution {
    if (slots.isEmpty()) {
        return AppDataMicroBreakResolution(
            data = this,
            transitions = emptyList(),
            complete = true
        )
    }

    val resolvedSlots = ArrayList<TimerSlot>(slots.size)
    val generatedSessions = mutableListOf<TimerSession>()
    val transitions = mutableListOf<MicroBreakTransition>()
    var complete = true

    slots.forEach { slot ->
        val resolution = slot.resolveMicroBreak(now)
        resolvedSlots += resolution.slot
        generatedSessions += resolution.sessions
        transitions += resolution.transitions
        complete = complete && resolution.complete
    }

    if (generatedSessions.isEmpty() && resolvedSlots == slots) {
        return AppDataMicroBreakResolution(
            data = this,
            transitions = emptyList(),
            complete = complete
        )
    }

    return AppDataMicroBreakResolution(
        data = copy(
            slots = resolvedSlots,
            sessions = generatedSessions.sortedByDescending(TimerSession::endedAtEpochMillis) + sessions
        ),
        transitions = transitions.sortedBy(MicroBreakTransition::occurredAtEpochMillis),
        complete = complete
    )
}"####,
        "aggregate micro-break completion",
    )?;

    Ok(rendered)
}

fn render_native_bridge(base: &str) -> Result<String, String> {
    let mut rendered = base.to_string();

    replace_exactly_once(
        &mut rendered,
        "    val updatedAt: Long,\n    val sessions: List<NativeMicroBreakSession>,\n",
        "    val updatedAt: Long,\n    val complete: Boolean,\n    val sessions: List<NativeMicroBreakSession>,\n",
        "native bridge completion field",
    )?;

    replace_exactly_once(
        &mut rendered,
        "    private const val MICRO_BREAK_RESOLUTION_HEADER_FIELD_COUNT = 8\n",
        "    private const val MICRO_BREAK_RESOLUTION_HEADER_FIELD_COUNT = 9\n",
        "native bridge header width",
    )?;

    replace_exactly_once(
        &mut rendered,
        "    private const val MICRO_BREAK_TRANSITION_FIELD_COUNT = 2\n",
        "    private const val MICRO_BREAK_TRANSITION_FIELD_COUNT = 2\n    private const val MICRO_BREAK_RESOLUTION_MAX_SESSIONS = 64\n    private const val MICRO_BREAK_RESOLUTION_MAX_TRANSITIONS = 128\n",
        "native bridge page bounds",
    )?;

    replace_section_exactly_once(
        &mut rendered,
        "    private fun parseNativeMicroBreakResolution(values: LongArray): NativeMicroBreakResolution? {",
        "\n\n    private fun parseTimerSessionUiIndexStats(",
        r####"    private fun parseNativeMicroBreakResolution(values: LongArray): NativeMicroBreakResolution? {
        if (values.size < MICRO_BREAK_RESOLUTION_HEADER_FIELD_COUNT) {
            return null
        }
        val sessionCountLong = values[6]
        val transitionCountLong = values[7]
        if (
            sessionCountLong !in 0L..MICRO_BREAK_RESOLUTION_MAX_SESSIONS.toLong() ||
            transitionCountLong !in 0L..MICRO_BREAK_RESOLUTION_MAX_TRANSITIONS.toLong()
        ) {
            return null
        }
        val expectedSizeLong = runCatching {
            Math.addExact(
                Math.addExact(
                    MICRO_BREAK_RESOLUTION_HEADER_FIELD_COUNT.toLong(),
                    Math.multiplyExact(
                        sessionCountLong,
                        MICRO_BREAK_SESSION_FIELD_COUNT.toLong()
                    )
                ),
                Math.multiplyExact(
                    transitionCountLong,
                    MICRO_BREAK_TRANSITION_FIELD_COUNT.toLong()
                )
            )
        }.getOrNull() ?: return null
        if (values.size.toLong() != expectedSizeLong) {
            return null
        }
        val complete = when (values[8]) {
            0L -> false
            1L -> true
            else -> return null
        }
        val phase = when (values[2]) {
            MICRO_BREAK_PHASE_FOCUS.toLong() -> MicroBreakPhase.FOCUS
            MICRO_BREAK_PHASE_BREAK.toLong() -> MicroBreakPhase.BREAK
            else -> return null
        }
        val sessionCount = sessionCountLong.toInt()
        val transitionCount = transitionCountLong.toInt()

        var offset = MICRO_BREAK_RESOLUTION_HEADER_FIELD_COUNT
        val sessions = buildList(sessionCount) {
            repeat(sessionCount) {
                add(
                    NativeMicroBreakSession(
                        startedAtEpochMillis = values[offset],
                        endedAtEpochMillis = values[offset + 1],
                        durationMillis = values[offset + 2]
                    )
                )
                offset += MICRO_BREAK_SESSION_FIELD_COUNT
            }
        }
        val transitions = buildList(transitionCount) {
            repeat(transitionCount) {
                add(
                    NativeMicroBreakTransition(
                        typeCode = values[offset].toInt(),
                        occurredAtEpochMillis = values[offset + 1]
                    )
                )
                offset += MICRO_BREAK_TRANSITION_FIELD_COUNT
            }
        }

        return NativeMicroBreakResolution(
            accumulatedMillis = values[0],
            runningSinceEpochMillis = values[1],
            microBreakPhase = phase,
            microBreakCycleIndex = values[3].coerceIn(0L, Int.MAX_VALUE.toLong()).toInt(),
            microBreakPhaseProgressMillis = values[4].coerceAtLeast(0L),
            updatedAt = values[5].coerceAtLeast(0L),
            complete = complete,
            sessions = sessions,
            transitions = transitions
        )
    }"####,
        "strict micro-break JNI parser",
    )?;

    replace_exactly_once(
        &mut rendered,
        "    fun computeMicroBreakTargetMillis(slotId: Int, cycleIndex: Int): Long {",
        r####"    fun verifySyncRendezvousSignature(
        payload: String,
        publicKeyBase64: String,
        signatureBase64: String
    ): Boolean {
        if (!nativeAvailable) {
            return false
        }
        return runCatching {
            nativeVerifySyncRendezvousSignature(
                payload = payload,
                publicKeyBase64 = publicKeyBase64,
                signatureBase64 = signatureBase64
            )
        }.getOrDefault(false)
    }

    fun computeMicroBreakTargetMillis(slotId: Int, cycleIndex: Int): Long {"####,
        "sync rendezvous signature wrapper",
    )?;

    replace_exactly_once(
        &mut rendered,
        "    @JvmStatic\n    private external fun nativeComputeMicroBreakTargetMillis(\n",
        "    @JvmStatic\n    private external fun nativeVerifySyncRendezvousSignature(\n        payload: String,\n        publicKeyBase64: String,\n        signatureBase64: String\n    ): Boolean\n\n    @JvmStatic\n    private external fun nativeComputeMicroBreakTargetMillis(\n",
        "sync rendezvous signature JNI declaration",
    )?;

    Ok(rendered)
}

fn render_ui_indexes(base: &str) -> Result<String, String> {
    let mut rendered = base.to_string();
    replace_exactly_once(
        &mut rendered,
        r####"@Composable
internal fun rememberAppDataUiIndex(appData: AppData): AppDataUiIndex {
    return remember(appData) {
        buildAppDataUiIndex(appData)
    }
}"####,
        r####"@Composable
internal fun rememberAppDataUiIndex(appData: AppData): AppDataUiIndex {
    val slotIdentityKey = appData.slots.map { it.id }
    return remember(
        appData.categories,
        appData.sessions,
        appData.archivedTasks,
        slotIdentityKey
    ) {
        buildAppDataUiIndex(appData)
    }
}"####,
        "domain-scoped UI index keys",
    )?;
    Ok(rendered)
}

fn render_diagnostic_log(base: &str) -> Result<String, String> {
    let mut rendered = base.to_string();

    replace_exactly_once(
        &mut rendered,
        r####"import java.time.format.DateTimeFormatter

object DiagnosticLogStore {
    private const val LOG_FILE_NAME = "diagnostic_events.log"
    private const val MAX_LOG_FILE_SIZE_BYTES = 256 * 1024L
    private const val TRIMMED_LOG_LINE_COUNT = 700
    private const val EMPTY_LOG_MESSAGE = "No in-app diagnostic events recorded yet."
    private val fileTimestampFormatter =
        DateTimeFormatter.ofPattern("yyyy-MM-dd HH:mm:ss.SSS Z").withZone(ZoneId.systemDefault())
    private val lock = Any()
"####,
        r####"import java.time.format.DateTimeFormatter
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.selects.select
import kotlinx.coroutines.withTimeoutOrNull

object DiagnosticLogStore {
    private const val LOG_FILE_NAME = "diagnostic_events.log"
    private const val ROTATED_LOG_FILE_NAME = "diagnostic_events.1.log"
    private const val SNAPSHOT_LOG_FILE_NAME = "diagnostic_events_snapshot.log"
    private const val MAX_LOG_FILE_SIZE_BYTES = 256 * 1024L
    private const val MAX_PENDING_LOG_WRITES = 256
    private const val MAX_PENDING_FLUSH_REQUESTS = 4
    private const val FLUSH_TIMEOUT_MILLIS = 5_000L
    private const val EMPTY_LOG_MESSAGE = "No in-app diagnostic events recorded yet."
    private val fileTimestampFormatter =
        DateTimeFormatter.ofPattern("yyyy-MM-dd HH:mm:ss.SSS Z").withZone(ZoneId.systemDefault())
    private val fileLock = Any()
    private val pendingEntries = Channel<PendingLogEntry>(
        capacity = MAX_PENDING_LOG_WRITES,
        onBufferOverflow = BufferOverflow.DROP_OLDEST
    )
    private val flushRequests = Channel<CompletableDeferred<Unit>>(
        capacity = MAX_PENDING_FLUSH_REQUESTS
    )
    private val writerScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    private data class PendingLogEntry(
        val context: Context,
        val category: String,
        val message: String,
        val throwable: Throwable?,
        val recordedAt: Instant,
        val callerThreadName: String
    )

    @Suppress("unused")
    private val writerJob = writerScope.launch {
        while (isActive) {
            select<Unit> {
                flushRequests.onReceive { completion ->
                    drainPendingEntries()
                    completion.complete(Unit)
                }
                pendingEntries.onReceive { entry ->
                    writeEntrySafely(entry)
                }
            }
        }
    }
"####,
        "bounded diagnostic writer configuration",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"    fun record(
        context: Context,
        category: String,
        message: String,
        throwable: Throwable? = null
    ) {
        runCatching {
            appendEntry(
                context = context.applicationContext,
                category = category,
                message = message,
                throwable = throwable
            )
        }
    }
"####,
        r####"    fun record(
        context: Context,
        category: String,
        message: String,
        throwable: Throwable? = null
    ) {
        pendingEntries.trySend(
            PendingLogEntry(
                context = context.applicationContext,
                category = category,
                message = message,
                throwable = throwable,
                recordedAt = Instant.now(),
                callerThreadName = Thread.currentThread().name
            )
        )
    }
"####,
        "non-blocking diagnostic record entrypoint",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"    fun readRecentEntries(context: Context, maxLines: Int = 300): String {
        return runCatching {
            val file = logFile(context.applicationContext)
            if (!file.exists()) {
                EMPTY_LOG_MESSAGE
            } else {
                file.readLines()
                    .takeLast(maxLines)
                    .joinToString(separator = "\n")
                    .ifBlank { EMPTY_LOG_MESSAGE }
            }
        }.getOrElse { throwable ->
            "Failed to read in-app diagnostic events: ${throwable.message ?: throwable::class.java.simpleName}"
        }
    }
"####,
        r####"    fun readRecentEntries(context: Context, maxLines: Int = 300): String {
        flushPendingWrites()
        return runCatching {
            synchronized(fileLock) {
                val files = logFilesOldestFirst(context.applicationContext)
                if (files.isEmpty()) {
                    EMPTY_LOG_MESSAGE
                } else {
                    files
                        .flatMap { file -> file.readLines() }
                        .takeLast(maxLines)
                        .joinToString(separator = "\n")
                        .ifBlank { EMPTY_LOG_MESSAGE }
                }
            }
        }.getOrElse { throwable ->
            "Failed to read in-app diagnostic events: ${throwable.message ?: throwable::class.java.simpleName}"
        }
    }
"####,
        "flushed bounded diagnostic line read",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"    fun readEntriesSince(
        context: Context,
        sinceEpochMillis: Long,
        maxEntries: Int = 180
    ): String {
        return runCatching {
            val file = logFile(context.applicationContext)
            if (!file.exists()) {
                EMPTY_LOG_MESSAGE
            } else {
                readStructuredEntries(file)
                    .filter { entry ->
                        val entryEpochMillis = parseEntryEpochMillis(entry) ?: Long.MAX_VALUE
                        entryEpochMillis >= sinceEpochMillis
                    }
                    .takeLast(maxEntries)
                    .joinToString(separator = "\n\n")
                    .ifBlank { EMPTY_LOG_MESSAGE }
            }
        }.getOrElse { throwable ->
            "Failed to read in-app diagnostic events since $sinceEpochMillis: ${throwable.message ?: throwable::class.java.simpleName}"
        }
    }
"####,
        r####"    fun readEntriesSince(
        context: Context,
        sinceEpochMillis: Long,
        maxEntries: Int = 180
    ): String {
        flushPendingWrites()
        return runCatching {
            synchronized(fileLock) {
                val files = logFilesOldestFirst(context.applicationContext)
                if (files.isEmpty()) {
                    EMPTY_LOG_MESSAGE
                } else {
                    files
                        .flatMap { file -> readStructuredEntries(file) }
                        .filter { entry ->
                            val entryEpochMillis = parseEntryEpochMillis(entry) ?: Long.MAX_VALUE
                            entryEpochMillis >= sinceEpochMillis
                        }
                        .takeLast(maxEntries)
                        .joinToString(separator = "\n\n")
                        .ifBlank { EMPTY_LOG_MESSAGE }
                }
            }
        }.getOrElse { throwable ->
            "Failed to read in-app diagnostic events since $sinceEpochMillis: ${throwable.message ?: throwable::class.java.simpleName}"
        }
    }
"####,
        "flushed bounded diagnostic structured read",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"    fun snapshotFile(context: Context): File? {
        val file = logFile(context.applicationContext)
        return file.takeIf(File::exists)
    }
"####,
        r####"    fun snapshotFile(context: Context): File? {
        flushPendingWrites()
        return runCatching {
            synchronized(fileLock) {
                val applicationContext = context.applicationContext
                val files = logFilesOldestFirst(applicationContext)
                if (files.isEmpty()) {
                    null
                } else {
                    val snapshot = File(applicationContext.cacheDir, SNAPSHOT_LOG_FILE_NAME)
                    snapshot.outputStream().buffered().use { output ->
                        files.forEachIndexed { index, file ->
                            file.inputStream().buffered().use { input -> input.copyTo(output) }
                            if (index < files.lastIndex && file.length() > 0L) {
                                output.write('\n'.code)
                            }
                        }
                    }
                    snapshot
                }
            }
        }.getOrNull()
    }
"####,
        "flushed complete diagnostic snapshot boundary",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"    private fun appendEntry(
        context: Context,
        category: String,
        message: String,
        throwable: Throwable?
    ) {
        synchronized(lock) {
            val file = logFile(context)
            file.parentFile?.mkdirs()
            val recordedAt = Instant.now()
            val recordedAtEpochMillis = recordedAt.toEpochMilli()
            val sanitizedMessage = message
                .replace("\r\n", "\n")
                .replace('\r', '\n')
                .trim()
                .ifBlank { "(empty message)" }
            val stackTrace = throwable?.stackTraceToString()?.trim()
            val entry = buildString {
                append(fileTimestampFormatter.format(recordedAt))
                append(" | epoch=")
                append(recordedAtEpochMillis)
                append(" | pid=")
                append(Process.myPid())
                append(" | thread=")
                append(Thread.currentThread().name)
                append(" | category=")
                append(category)
                append(" | ")
                append(sanitizedMessage)
                if (!stackTrace.isNullOrBlank()) {
                    append('\n')
                    append(stackTrace)
                }
                append("\n\n")
            }
            file.appendText(entry)
            trimIfNeeded(file)
        }
    }

    private fun trimIfNeeded(file: File) {
        if (!file.exists() || file.length() <= MAX_LOG_FILE_SIZE_BYTES) {
            return
        }
        val trimmed = file.readLines()
            .takeLast(TRIMMED_LOG_LINE_COUNT)
            .joinToString(separator = "\n")
            .trim()
        file.writeText(if (trimmed.isBlank()) "" else "$trimmed\n")
    }
"####,
        r####"    private fun writeEntrySafely(entry: PendingLogEntry) {
        runCatching {
            appendEntry(entry)
        }
    }

    private fun drainPendingEntries() {
        repeat(MAX_PENDING_LOG_WRITES) {
            val entry = pendingEntries.tryReceive().getOrNull() ?: return
            writeEntrySafely(entry)
        }
    }

    private fun flushPendingWrites() {
        runBlocking {
            withTimeoutOrNull(FLUSH_TIMEOUT_MILLIS) {
                val completion = CompletableDeferred<Unit>()
                flushRequests.send(completion)
                completion.await()
            }
        }
    }

    private fun appendEntry(pending: PendingLogEntry) {
        val sanitizedMessage = pending.message
            .replace("\r\n", "\n")
            .replace('\r', '\n')
            .trim()
            .ifBlank { "(empty message)" }
        val stackTrace = pending.throwable?.stackTraceToString()?.trim()
        val entry = buildString {
            append(fileTimestampFormatter.format(pending.recordedAt))
            append(" | epoch=")
            append(pending.recordedAt.toEpochMilli())
            append(" | pid=")
            append(Process.myPid())
            append(" | thread=")
            append(pending.callerThreadName)
            append(" | category=")
            append(pending.category)
            append(" | ")
            append(sanitizedMessage)
            if (!stackTrace.isNullOrBlank()) {
                append('\n')
                append(stackTrace)
            }
            append("\n\n")
        }
        synchronized(fileLock) {
            val file = logFile(pending.context)
            file.parentFile?.mkdirs()
            rotateBeforeAppend(file, entry.toByteArray(Charsets.UTF_8).size.toLong())
            file.appendText(entry)
        }
    }

    private fun rotateBeforeAppend(file: File, incomingBytes: Long) {
        if (!file.exists() || file.length() + incomingBytes <= MAX_LOG_FILE_SIZE_BYTES) {
            return
        }
        val rotated = rotatedLogFile(file.parentFile)
        if (rotated.exists() && !rotated.delete()) {
            return
        }
        if (!file.renameTo(rotated)) {
            file.copyTo(rotated, overwrite = true)
            if (!file.delete()) {
                return
            }
        }
    }
"####,
        "bounded asynchronous diagnostic append and rotation",
    )?;

    replace_exactly_once(
        &mut rendered,
        r####"    private fun logFile(context: Context): File {
        return File(context.filesDir, LOG_FILE_NAME)
    }
"####,
        r####"    private fun logFilesOldestFirst(context: Context): List<File> {
        return listOf(
            rotatedLogFile(context.filesDir),
            logFile(context)
        ).filter(File::exists)
    }

    private fun rotatedLogFile(parent: File?): File {
        return File(parent, ROTATED_LOG_FILE_NAME)
    }

    private fun logFile(context: Context): File {
        return File(context.filesDir, LOG_FILE_NAME)
    }
"####,
        "rotated diagnostic log lookup",
    )?;

    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered_source(path: &str) -> String {
        let base = crate::kotlin_sources::SOURCES
            .iter()
            .find(|source| source.path == path)
            .unwrap_or_else(|| panic!("missing base source: {path}"))
            .contents;
        render(path, base).unwrap_or_else(|error| panic!("failed to render {path}: {error}"))
    }

    fn bounded<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
        let start_index = source
            .find(start)
            .unwrap_or_else(|| panic!("missing start: {start}"));
        let end_index = source[start_index..]
            .find(end)
            .map(|offset| start_index + offset)
            .unwrap_or_else(|| panic!("missing end after {start}: {end}"));
        &source[start_index..end_index]
    }

    #[test]
    fn running_pulse_is_observed_only_by_small_draw_or_layer_nodes() {
        let screen = rendered_source(SCREEN_PATH);
        assert!(
            screen.contains("private fun rememberRunningEmphasis(enabled: Boolean): State<Float>")
        );
        assert!(screen.contains("return emphasis.asState()"));
        assert!(!screen.contains("return emphasis.value"));

        let ambient = bounded(
            &screen,
            "private fun AmbientBackdrop(",
            "@OptIn(ExperimentalLayoutApi::class)",
        );
        assert!(ambient.contains(".drawBehind {"));
        assert!(ambient.contains("val pulse = runningPulse.value"));
        assert!(!ambient.contains("runningPulse *"));

        let dock = bounded(
            &screen,
            "private fun HomeNavigationDock(",
            "private fun Color.mixOverDock(",
        );
        assert!(dock.contains("selectedIndicatorBaseColor"));
        assert!(dock.contains("alpha = (runningPulse.value / 21f)"));
        assert!(!dock.contains("0.16f + runningPulse * 0.04f"));

        let button = bounded(
            &screen,
            "private fun DockNavigationButton(",
            "@OptIn(ExperimentalLayoutApi::class)",
        );
        assert!(button.contains("livePulse: State<Float>? = null"));
        assert!(button.contains(".size(17.5.dp)"));
        assert!(button.contains("val pulse = emphasis.value"));
        assert!(!button.contains(".size((12.5f + livePulse * 5f).dp)"));

        let tile = bounded(
            &screen,
            "private fun TimerTile(",
            "private fun LazyGridState.findVisibleSlotItemInfo(",
        );
        assert!(tile.contains("alpha = 0.16f + runningHighlight.value * 0.24f"));
        assert!(!tile.contains("runningHighlight *"));

        let status = bounded(
            &screen,
            "private fun StatusLight(",
            "@Composable\nprivate fun CategoryPill(",
        );
        assert!(status.contains("emphasis: State<Float>"));
        assert!(status.contains(".size(10.dp)"));
        assert!(status.contains("val pulse = emphasis.value"));
        assert!(!status.contains(".size((7.5f + emphasis * 2.5f).dp)"));
    }

    #[test]
    fn release_home_performance_sampling_has_callsite_and_sink_gates() {
        let screen = rendered_source(SCREEN_PATH);
        let root = bounded(
            &screen,
            "fun GridTimerRoot(",
            "private fun AmbientBackdrop(",
        );
        assert!(root.contains("if (HomePerformanceTrace.enabled)"));
        assert!(root.contains("HomePerformanceTrace.recordHomeRecompose("));

        let trace = rendered_source(HOME_TRACE_PATH);
        assert!(trace.contains("import com.ofairyo.gridtimer.BuildConfig"));
        assert!(trace.contains("get() = BuildConfig.DEBUG"));
        assert!(trace.contains("if (!enabled) {\n            return false"));
        assert!(trace.matches("if (!enabled) {").count() >= 3);
    }

    #[test]
    fn stalled_micro_break_updates_suspend_and_do_not_emit_false_notifications() {
        let repository = rendered_source(REPOSITORY_PATH);
        let monitor = bounded(
            &repository,
            "private suspend fun monitorMicroBreaks()",
            "suspend fun handleMicroBreakAlarm(",
        );
        assert!(monitor.contains("val progressed = advanceMicroBreaks(evaluationNow)"));
        assert!(monitor.contains("delay(stalledRetryMillis)"));
        assert!(monitor.contains("coerceAtMost(4_000L)"));

        let advance = bounded(
            &repository,
            "private suspend fun advanceMicroBreaks(",
            "private suspend fun persistSafely(",
        );
        assert!(advance.contains("): Boolean {"));
        assert!(advance.contains("resolutionChanged = resolution.data !== data"));
        let failure_guard = advance
            .find("if (!resolutionChanged)")
            .expect("failed update guard");
        let notifications = advance
            .find("freshTransitions.forEach")
            .expect("transition notifications");
        assert!(failure_guard < notifications);
        assert!(advance.contains("return true"));
    }

    #[test]
    fn diagnostic_writes_are_bounded_serial_and_flushed_for_reads() {
        let logs = rendered_source(DIAGNOSTIC_LOG_PATH);
        assert!(logs.contains("capacity = MAX_PENDING_LOG_WRITES"));
        assert!(logs.contains("onBufferOverflow = BufferOverflow.DROP_OLDEST"));
        assert!(logs.contains("private val flushRequests = Channel<CompletableDeferred<Unit>>"));
        assert!(logs.contains("pendingEntries.trySend("));
        assert!(logs.matches("flushPendingWrites()").count() >= 4);
        assert!(logs.contains("rotateBeforeAppend("));
        assert!(logs.contains("SNAPSHOT_LOG_FILE_NAME"));
        assert!(logs.contains("logFilesOldestFirst(applicationContext)"));
        assert!(!logs.contains("Channel.UNLIMITED"));
        assert!(!logs.contains("TRIMMED_LOG_LINE_COUNT"));
        assert!(!logs.contains("private fun trimIfNeeded("));
    }


    #[test]
    fn micro_break_and_jni_contracts_are_bounded_and_continuable() {
        let micro_breaks = rendered_source(MICRO_BREAKS_PATH);
        assert!(micro_breaks.contains("MICRO_BREAK_RESOLUTION_MAX_PHASE_STEPS = 128"));
        assert!(micro_breaks.contains("var phaseSteps = 0"));
        assert!(micro_breaks.contains("complete = complete && resolution.complete"));

        let bridge = rendered_source(BRIDGE_PATH);
        let parser = bounded(
            &bridge,
            "private fun parseNativeMicroBreakResolution(",
            "private fun parseTimerSessionUiIndexStats(",
        );
        assert!(bridge.contains("MICRO_BREAK_RESOLUTION_HEADER_FIELD_COUNT = 9"));
        assert!(bridge.contains("MICRO_BREAK_RESOLUTION_MAX_SESSIONS = 64"));
        assert!(parser.contains("Math.multiplyExact"));
        assert!(parser.contains("sessionCountLong !in 0L.."));
        assert!(parser.contains("complete = complete"));
        assert!(!parser.contains("toInt().coerceAtLeast(0)"));
    }

    #[test]
    fn ui_index_rebuild_uses_domain_keys() {
        let indexes = rendered_source(UI_INDEXES_PATH);
        assert!(indexes.contains("val slotIdentityKey = appData.slots.map { it.id }"));
        assert!(indexes.contains("appData.categories,"));
        assert!(indexes.contains("appData.sessions,"));
        assert!(indexes.contains("appData.archivedTasks,"));
        assert!(!indexes.contains("return remember(appData)"));
    }

    #[test]
    fn repository_mutation_cpu_is_explicitly_default_dispatched() {
        let repository = rendered_source(REPOSITORY_PATH);
        assert!(repository.contains("withContext(Dispatchers.Default)"));
        assert!(repository.contains("val changed = withContext(Dispatchers.Default)"));
        assert!(repository.contains("var catchUpCutoff: Long? = null"));
        assert!(repository.contains("advanceMicroBreaks(evaluationNow)"));
    }

    #[test]
    fn unrelated_sources_are_unchanged() {
        let source = "package example\n";
        assert_eq!(
            source,
            render("example.kt", source).expect("unrelated source")
        );
    }
}
