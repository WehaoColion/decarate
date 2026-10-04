// v2.22.49.2 - Apply account guards after timer and reminder transformations.
// v2.22.49.1 - Recover published phase reminders and play bells on the alarm stream.
// v2.22.48 - Bind reminder playback to the live run and phase; stop stale audio.
const REPOSITORY: &str = "com/ofairyo/gridtimer/data/TimerRepository.kt";
const PLAYER: &str = "com/ofairyo/gridtimer/notifications/TimerBellPlayer.kt";
const NOTIFIER: &str = "com/ofairyo/gridtimer/notifications/MicroBreakReminderNotifier.kt";
const PHASES: &str = "com/ofairyo/gridtimer/data/MicroBreaks.kt";
const SCHEDULER: &str = "com/ofairyo/gridtimer/notifications/MicroBreakAlarmScheduler.kt";

fn replace(source: &mut String, from: &str, to: &str) -> Result<(), String> {
    if source.matches(from).count() != 1 {
        return Err(format!("rest bell anchor must occur once: {from}"));
    }
    *source = source.replacen(from, to, 1);
    Ok(())
}

pub fn render(path: &str, base: &str) -> Result<String, String> {
    let mut source = base.to_string();
    match path {
        PHASES => {
            replace(
                &mut source,
                "import java.util.UUID",
                "import java.util.UUID\nimport kotlinx.coroutines.flow.collectLatest",
            )?;
            source.push_str(TOKEN);
        }
        PLAYER => {
            let start = source
                .find("internal object TimerBellPlayer {")
                .ok_or("bell object missing")?;
            let pool = source
                .find("    private fun ensureSoundPool(")
                .ok_or("bell pool missing")?;
            source.replace_range(start..pool, PLAYER_CONTROL);
            let start = source
                .find("    private fun playLoadedBell(")
                .ok_or("bell playback missing")?;
            source.truncate(start);
            source.push_str(PLAYER_PLAYBACK);
            source.push_str(GATE);
            replace(&mut source,
                "            // These bells are reminder prompts, not tap/click UI sounds. Routing them through the\n            // notification stream keeps them audible on newer HyperOS / Android builds.\n            .setUsage(AudioAttributes.USAGE_NOTIFICATION_EVENT)",
                "            // Timer deadlines follow the user's alarm volume and alarm policy.\n            .setUsage(AudioAttributes.USAGE_ALARM)")?;
            replace(
                &mut source,
                "            .setLegacyStreamType(AudioManager.STREAM_NOTIFICATION)\n",
                "",
            )?;
        }
        NOTIFIER => {
            let start = source
                .find("    fun notifyTransition(")
                .ok_or("notifier missing")?;
            let end = source
                .find("    private fun postTransitionNotification(")
                .ok_or("notification post missing")?;
            source.replace_range(start..end, NOTIFY);
            replace(
                &mut source,
                "\"${transition.slotTitle} 休息 15 秒\"",
                "\"${transition.slotTitle} 休息中\"",
            )?;
            replace(
                &mut source,
                "\"该放松一下了，15 秒后会自动回到下一轮专注。\"",
                "\"休息结束后自动继续。\"",
            )?;
            replace(
                &mut source,
                "\"微休息结束，回来继续当前格子的节奏。\"",
                "\"休息结束，继续计时。\"",
            )?;
            replace(&mut source, "        val notification = NotificationCompat.Builder(context, MICRO_BREAK_CHANNEL_ID)", "        val notification = NotificationCompat.Builder(context, MICRO_BREAK_CHANNEL_ID)\n            .setTimeoutAfter(if (transition.type == MicroBreakTransitionType.BREAK_STARTED)\n                (transition.occurredAtEpochMillis + 15_000L - System.currentTimeMillis()).coerceAtLeast(1L) else 0L)")?;
            replace(&mut source, "            .setTimeoutAfter(12_000L)", "")?;
            replace(
                &mut source,
                ".coerceAtLeast(1L) else 0L)",
                ".coerceAtLeast(1L) else 12_000L)",
            )?;
        }
        REPOSITORY => {
            replace(&mut source, "    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)", "    private val bellDispatch = TimerBellDispatch()\n    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)")?;
            replace(&mut source, "        var startedPhase: MicroBreakPhase? = null", "        var startedPhase: MicroBreakPhase? = null\n        var startedBell: TimerBellToken? = null")?;
            replace(&mut source, "                    startedPhase = updatedSlot.microBreakPhase", "                    startedPhase = updatedSlot.microBreakPhase\n                    startedBell = updatedSlot.bellToken(activeWorkspaceKey, now)")?;
            replace(&mut source, "                            startedPhase = resumed.microBreakPhase", "                            startedPhase = resumed.microBreakPhase\n                            startedBell = resumed.bellToken(activeWorkspaceKey, now)")?;
            replace(&mut source, "        startedPhase?.let { phase ->\n            TimerBellPlayer.play(appContext, phase.toTimerBellKind())\n        }", "        startedBell?.let { token ->\n            TimerBellPlayer.play(appContext, token.phase.toTimerBellKind(), token.slotId, { isCurrentBell(token) })\n        }")?;
            replace(&mut source, "                val progressed = advanceMicroBreaks(now())", "                // Publishing the new phase cancels collectLatest's old collector.\n                // Complete its accepted reminder, then the next collector takes over.\n                val progressed = finishTimerTransition { advanceMicroBreaks(now()) }")?;
            replace(&mut source, "        var freshTransitions = emptyList<MicroBreakTransition>()", "        var freshTransitions = emptyList<MicroBreakTransition>()\n        var bellWorkspace = activeWorkspaceKey")?;
            replace(&mut source, "            before = _appData.value\n            val resolution = data.resolveMicroBreaks(now)", "            before = _appData.value\n            bellWorkspace = activeWorkspaceKey\n            val resolution = data.resolveMicroBreaks(now)")?;
            replace(&mut source, "            val candidates = resolution.transitions.filter { transition ->\n                (now - transition.occurredAtEpochMillis) in 0L..alertFreshnessMillis\n            }\n            freshTransitions = if (latestTransitionOnly) candidates.takeLast(1) else candidates", "            freshTransitions = currentBellTransitions(resolution, now, alertFreshnessMillis)")?;
            replace(&mut source, "        freshTransitions.forEach { transition ->", "        freshTransitions.forEach { transition ->\n            val slot = attempt.snapshot?.slots?.firstOrNull { it.id == transition.slotId } ?: return@forEach\n            val token = slot.bellToken(bellWorkspace, now) ?: return@forEach\n            if (!isCurrentBell(token)) return@forEach")?;
            replace(&mut source, "MicroBreakReminderNotifier.notifyTransitionAndAwaitBell(appContext, transition)", "MicroBreakReminderNotifier.notifyTransitionAndAwaitBell(appContext, transition) { isCurrentBell(token) && (now() - transition.occurredAtEpochMillis) in 0L..MICRO_BREAK_ALERT_FRESHNESS_MILLIS }")?;
            replace(&mut source, "MicroBreakReminderNotifier.notifyTransition(appContext, transition)", "MicroBreakReminderNotifier.notifyTransition(appContext, transition) { isCurrentBell(token) && (now() - transition.occurredAtEpochMillis) in 0L..MICRO_BREAK_ALERT_FRESHNESS_MILLIS }")?;
            replace(&mut source, "    private fun syncMicroBreakAlarm(snapshot: AppData = _appData.value) {", "    private fun isCurrentBell(token: TimerBellToken): Boolean = token.isCurrent(\n        activeWorkspaceKey, _appData.value.slots.firstOrNull { it.id == token.slotId }, now()\n    )\n\n    @Synchronized private fun syncMicroBreakAlarm(snapshot: AppData = _appData.value) {\n        // This runs as part of publication, before potentially slow persistence.\n        TimerBellPlayer.reconcile()")?;
            // Read live state while serializing scheduler calls. A delayed caller must
            // never re-arm a previously paused snapshot.
            replace(
                &mut source,
                "nextDelayMillis = snapshot.nextMicroBreakDelayMillis(now())",
                "nextDelayMillis = _appData.value.nextMicroBreakDelayMillis(now())",
            )?;
            replace(
                &mut source,
                "            latestTransitionOnly = true,\n",
                "",
            )?;
            replace(&mut source,
                "        targetSlotIds.forEach { MicroBreakReminderNotifier.cancelSlot(appContext, it) }",
                "        targetSlotIds.filter { id -> _appData.value.slots.firstOrNull { it.id == id }?.runningSinceEpochMillis == null }\n            .forEach { MicroBreakReminderNotifier.cancelSlot(appContext, it) }")?;
            replace(
                &mut source,
                "        latestTransitionOnly: Boolean = false,\n",
                "",
            )?;
            // Keep wall-clock delivery independent of snapshot writes and global
            // history scans. Those may hold the data writer for longer than a rest.
            replace(&mut source, "        monitorMicroBreaks()", "        launch(Dispatchers.Default) { monitorBellDeadlines() }\n        monitorMicroBreaks()")?;
            replace(&mut source,
                "        if (startedPhase != null) {\n            persistNow(\"slot_start\")\n        }\n        startedBell?.let { token ->\n            TimerBellPlayer.play(appContext, token.phase.toTimerBellKind(), token.slotId, { isCurrentBell(token) })\n        }",
                "        startedBell?.let { token ->\n            if (bellDispatch.claim(token) { isCurrentBell(token) }) {\n                TimerBellPlayer.play(appContext, token.phase.toTimerBellKind(), token.slotId, { isCurrentBell(token) })\n            }\n        }\n        if (startedPhase != null) {\n            persistNow(\"slot_start\")\n        }")?;
            let start = source
                .find("    suspend fun startSlot(")
                .ok_or("start missing")?;
            let end = source[start..]
                .find("    suspend fun pauseSlot(")
                .map(|n| start + n)
                .ok_or("start end missing")?;
            let mut resume = source[start..end].to_string();
            replace(
                &mut resume,
                "        val now = now()",
                "        var now = now()",
            )?;
            replace(&mut resume, "        updateData(timerOnly = true) { data ->", "        updateData(timerOnly = true) { data ->\n            // Queueing behind a save must not consume the remaining rest.\n            now = now()")?;
            source.replace_range(start..end, &resume);
            replace(&mut source,
                "        val now = now()\n        logDiagnosticEvent(\n            category = \"slot.pause\",",
                "        val pauseWorkspace = activeWorkspaceKey\n        bellDispatch.beginPause(pauseWorkspace, targetSlotIds)\n        TimerBellPlayer.reconcile()\n        val now = now()\n        logDiagnosticEvent(\n            category = \"slot.pause\",")?;
            let start = source
                .find("    suspend fun pauseSlots(")
                .ok_or("pause missing")?;
            let end = source[start..]
                .find("    suspend fun resetSlot(")
                .map(|n| start + n)
                .ok_or("pause end missing")?;
            let mut pause = source[start..end].to_string();
            replace(
                &mut pause,
                "        updateData(timerOnly = true) { data ->",
                "        try {\n        updateData(timerOnly = true) { data ->",
            )?;
            replace(&mut pause, "        persistNow(\"slot_pause\")", "        } finally {\n            bellDispatch.endPause(pauseWorkspace, targetSlotIds)\n        }\n        persistNow(\"slot_pause\")")?;
            source.replace_range(start..end, &pause);
            let start = source
                .find("        freshTransitions.forEach { transition ->")
                .ok_or("transition delivery missing")?;
            let end = source[start..]
                .find("        syncTimerLiveUpdate()")
                .map(|n| start + n)
                .ok_or("transition delivery end missing")?;
            source.replace_range(start..end,
                "        deliverMicroBreakTransitions(\n            AppDataMicroBreakResolution(attempt.snapshot ?: return false, freshTransitions),\n            bellWorkspace, now, awaitBell\n        )\n");
            let start = source
                .find("    suspend fun handleMicroBreakAlarm(")
                .ok_or("alarm missing")?;
            let end = source[start..]
                .find("    private suspend fun advanceMicroBreaks(")
                .map(|n| start + n)
                .ok_or("alarm end missing")?;
            source.replace_range(start..end, ALARM_DELIVERY);
            replace(&mut source,
                "    private fun isCurrentBell(token: TimerBellToken): Boolean = token.isCurrent(\n        activeWorkspaceKey, _appData.value.slots.firstOrNull { it.id == token.slotId }, now()\n    )",
                BELL_CLOCK)?;
            replace(
                &mut source,
                "nextDelayMillis = _appData.value.nextMicroBreakDelayMillis(now())",
                "nextDelayMillis = bellResolution(now()).data.nextMicroBreakDelayMillis(now())",
            )?;
        }
        SCHEDULER => {
            // User-visible timer boundaries may be only 15 s apart. Idle quotas
            // on setExactAndAllowWhileIdle cannot represent that deadline.
            replace(&mut source, "                alarmManager.setExactAndAllowWhileIdle(\n                    AlarmManager.RTC_WAKEUP,\n                    triggerAt,\n                    operation\n                )", "                alarmManager.setAlarmClock(\n                    AlarmManager.AlarmClockInfo(triggerAt, PendingIntent.getActivity(\n                        appContext, MICRO_BREAK_ALARM_REQUEST_CODE, MainActivity.createLaunchIntent(appContext),\n                        PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE\n                    )), operation\n                )")?;
        }
        _ => {}
    }
    super::android_timer_action::bind_workspace(path, &source)
}

pub const TOKEN: &str = r####"

internal data class TimerBellToken(
    val workspace: String,
    val slotId: Int,
    val runId: String,
    val phase: MicroBreakPhase,
    val cycle: Int,
    val segmentStart: Long,
    val deadline: Long
) {
    fun isCurrent(workspace: String, slot: TimerSlot?, now: Long): Boolean =
        this.workspace == workspace && slot != null && slot.id == slotId &&
            slot.runningSinceEpochMillis == segmentStart && slot.activeRunId == runId &&
            slot.microBreakPhase == phase && slot.microBreakCycleIndex == cycle &&
            now >= segmentStart && now < deadline
}

internal fun TimerSlot.bellToken(workspace: String, now: Long): TimerBellToken? {
    val start = runningSinceEpochMillis ?: return null
    val remaining = currentPhaseRemainingMillis(now)
    if (remaining <= 0L || start > now) return null
    return TimerBellToken(workspace, id, activeRunId, microBreakPhase,
        microBreakCycleIndex, start, now + remaining)
}

internal fun currentBellTransitions(
    resolution: AppDataMicroBreakResolution, now: Long, freshness: Long
): List<MicroBreakTransition> {
    // Publication or startup recovery may already have consumed the transition
    // list. The active segment still records its exact boundary. Recover only a
    // fresh automatic boundary, never a manual start/resume or an old reminder.
    val explicitSlots = resolution.transitions.map { it.slotId }.toSet()
    val recovered = resolution.data.slots.mapNotNull { slot ->
        val start = slot.runningSinceEpochMillis ?: return@mapNotNull null
        if (slot.id in explicitSlots || slot.activeRunId.isBlank() ||
            slot.microBreakPhaseProgressMillis != 0L ||
            (now - start) !in 0L..minOf(freshness, MICRO_BREAK_ALERT_FRESHNESS_MILLIS)) return@mapNotNull null
        val manualRun = deterministicTimerRunId(slot.id, start)
        if (slot.activeRunId == manualRun || slot.activeRunId.startsWith("$manualRun-")) return@mapNotNull null
        MicroBreakTransition(
            if (slot.microBreakPhase == MicroBreakPhase.BREAK) MicroBreakTransitionType.BREAK_STARTED
            else MicroBreakTransitionType.FOCUS_RESUMED,
            slot.id, slot.title.ifBlank { "任务 ${slot.id.toString().padStart(2, '0')}" }, start
        )
    }
    return (resolution.transitions + recovered).groupBy { it.slotId }.values.mapNotNull { items ->
    val latest = items.maxByOrNull { it.occurredAtEpochMillis } ?: return@mapNotNull null
    val slot = resolution.data.slots.firstOrNull { it.id == latest.slotId } ?: return@mapNotNull null
    val phase = if (latest.type == MicroBreakTransitionType.BREAK_STARTED) MicroBreakPhase.BREAK else MicroBreakPhase.FOCUS
    latest.takeIf {
        slot.runningSinceEpochMillis == it.occurredAtEpochMillis && slot.microBreakPhase == phase &&
            (now - it.occurredAtEpochMillis) in 0L..minOf(freshness, MICRO_BREAK_ALERT_FRESHNESS_MILLIS) &&
            slot.currentPhaseRemainingMillis(now) > 0L
    }
}.sortedBy { it.occurredAtEpochMillis }
}

internal suspend fun monitorTimerBellDeadlines(
    snapshots: kotlinx.coroutines.flow.Flow<AppData>,
    now: () -> Long,
    deliver: suspend (AppDataMicroBreakResolution, Long) -> Unit
) {
    snapshots.collectLatest { snapshot ->
        var clock = AppData(slots = snapshot.slots)
        while (true) {
            val at = now()
            val resolution = clock.resolveMicroBreaks(at)
            // Inspect the accepted snapshot before waiting for its NEXT phase.
            // This also covers a collector replaced exactly at a deadline.
            deliver(resolution, at)
            clock = resolution.data
            val wait = clock.slots.asSequence().filter { it.runningSinceEpochMillis != null }
                .map { it.currentPhaseRemainingMillis(now()) }.minOrNull() ?: break
            kotlinx.coroutines.delay(wait.coerceAtLeast(1L))
        }
    }
}

internal suspend fun <T> finishTimerTransition(block: suspend () -> T): T =
    kotlinx.coroutines.withContext(kotlinx.coroutines.NonCancellable) { block() }

// The clock and alarm can observe the same boundary while the database writer
// is blocked. Admit it once, and silence pause requests before they wait for IO.
internal class TimerBellDispatch {
    private val delivered = mutableMapOf<Pair<String, Int>, TimerBellToken>()
    private val pauses = mutableMapOf<Pair<String, Int>, Int>()

    @Synchronized fun beginPause(workspace: String, ids: Collection<Int>) {
        ids.forEach { id -> val key = workspace to id; pauses[key] = (pauses[key] ?: 0) + 1 }
    }
    @Synchronized fun endPause(workspace: String, ids: Collection<Int>) {
        ids.forEach { id ->
            val key = workspace to id
            val remaining = (pauses[key] ?: 1) - 1
            if (remaining == 0) pauses.remove(key) else pauses[key] = remaining
        }
    }
    @Synchronized fun permits(token: TimerBellToken): Boolean =
        (pauses[token.workspace to token.slotId] ?: 0) == 0

    @Synchronized fun claim(token: TimerBellToken, isCurrent: () -> Boolean): Boolean {
        val key = token.workspace to token.slotId
        if (!permits(token) || delivered[key] == token || !isCurrent()) return false
        delivered[key] = token
        return true
    }
}
"####;

const ALARM_DELIVERY: &str = r####"    suspend fun handleMicroBreakAlarm(triggeredAt: Long = now()) {
        awaitInitialized()
        val at = maxOf(triggeredAt, now())
        deliverMicroBreakTransitions(bellResolution(at), activeWorkspaceKey, at, true)
        syncMicroBreakAlarm()
        // The broadcast receipt only owns playback. Persistence has its own
        // repository lifetime, so a slow history scan cannot replace the next alarm.
        scope.launch { finishTimerTransition { advanceMicroBreaks(now(), source = "alarm") } }
    }

"####;

const BELL_CLOCK: &str = r####"    private fun bellResolution(at: Long): AppDataMicroBreakResolution =
        AppData(slots = _appData.value.slots).resolveMicroBreaks(at)

    private fun isCurrentBell(token: TimerBellToken): Boolean {
        val at = now()
        val slot = _appData.value.slots.firstOrNull { it.id == token.slotId }
        return bellDispatch.permits(token) && token.isCurrent(
            activeWorkspaceKey, slot?.resolveMicroBreak(at)?.slot, at
        )
    }

    private suspend fun monitorBellDeadlines() {
        monitorTimerBellDeadlines(_appData, { now() }) { resolution, at ->
            deliverMicroBreakTransitions(resolution, activeWorkspaceKey, at, false)
            syncMicroBreakAlarm()
        }
    }

    private suspend fun deliverMicroBreakTransitions(
        resolution: AppDataMicroBreakResolution, workspace: String, at: Long, awaitBell: Boolean
    ) {
        currentBellTransitions(resolution, at, MICRO_BREAK_ALERT_FRESHNESS_MILLIS).forEach { transition ->
            val slot = resolution.data.slots.firstOrNull { it.id == transition.slotId } ?: return@forEach
            val token = slot.bellToken(workspace, at) ?: return@forEach
            if (!bellDispatch.claim(token) { isCurrentBell(token) }) return@forEach
            val valid = { isCurrentBell(token) &&
                (now() - transition.occurredAtEpochMillis) in 0L..MICRO_BREAK_ALERT_FRESHNESS_MILLIS }
            if (awaitBell) {
                MicroBreakReminderNotifier.notifyTransitionAndAwaitBell(appContext, transition, valid)
            } else {
                MicroBreakReminderNotifier.notifyTransition(appContext, transition, valid)
            }
        }
    }
"####;

pub const GATE: &str = r####"

// Serializes validity checks, playback/notification side effects and invalidation.
// Kept free of Android APIs so race boundaries can be tested deterministically.
internal class TimerBellGate {
    class Ticket(val slotId: Int, val isCurrent: () -> Boolean) {
        val cleanup = mutableListOf<() -> Unit>()
    }
    private val tickets = mutableMapOf<Int, Ticket>()

    @Synchronized fun open(slotId: Int, isCurrent: () -> Boolean): Ticket? {
        if (!isCurrent()) return null
        cancel(slotId)
        return Ticket(slotId, isCurrent).also { tickets[slotId] = it }
    }

    @Synchronized fun use(ticket: Ticket, action: () -> Unit): Boolean {
        if (tickets[ticket.slotId] !== ticket) return false
        if (!ticket.isCurrent()) {
            cancel(ticket.slotId)
            return false
        }
        action()
        return true
    }

    @Synchronized fun reconcile() {
        tickets.values.toList().filter { !it.isCurrent() }.forEach { cancel(it.slotId) }
    }

    @Synchronized fun cancel(slotId: Int) {
        tickets.remove(slotId)?.cleanup?.forEach { runCatching(it) }
    }

    @Synchronized fun cancelIfOwned(ticket: Ticket) {
        if (tickets[ticket.slotId] === ticket) cancel(ticket.slotId)
    }
}
"####;

const PLAYER_CONTROL: &str = r####"internal object TimerBellPlayer {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val lock = Any()
    private val gate = TimerBellGate()
    private var soundPool: SoundPool? = null
    private var focusSoundId: Int = 0
    private var breakSoundId: Int = 0
    private var focusLoaded = false
    private var breakLoaded = false

    fun preload(context: Context) { ensureSoundPool(context.applicationContext) }
    fun reconcile() { gate.reconcile() }

    fun play(context: Context, kind: TimerBellKind, slotId: Int, isCurrent: () -> Boolean,
             onStarted: () -> Unit = {}, onCanceled: () -> Unit = {}) {
        val ticket = gate.open(slotId, isCurrent) ?: return
        gate.use(ticket) { ticket.cleanup += onCanceled; ticket.cleanup += {
            android.util.Log.i("TimerBell", "canceled slot=$slotId kind=$kind")
        } }
        scope.launch { playTicket(context.applicationContext, kind, ticket, onStarted) }
    }

    suspend fun playAndAwait(context: Context, kind: TimerBellKind, slotId: Int, isCurrent: () -> Boolean,
                             onStarted: () -> Unit = {}, onCanceled: () -> Unit = {}) {
        val ticket = gate.open(slotId, isCurrent) ?: return
        gate.use(ticket) { ticket.cleanup += onCanceled; ticket.cleanup += {
            android.util.Log.i("TimerBell", "canceled slot=$slotId kind=$kind")
        } }
        playTicket(context.applicationContext, kind, ticket, onStarted)
    }

    private suspend fun playTicket(context: Context, kind: TimerBellKind, ticket: TimerBellGate.Ticket,
                                   onStarted: () -> Unit) {
        try {
            if (!gate.use(ticket) { preload(context) }) return
            repeat(9) { attempt ->
                if (!gate.use(ticket) {}) return
                val loaded = loadedBell(kind)
                if (loaded != null) {
                    var streamId = 0
                    if (!gate.use(ticket) {
                        streamId = runCatching { loaded.pool.play(loaded.soundId, 1f, 1f, 1, 0, 1f) }.getOrDefault(0)
                        if (streamId != 0) {
                            ticket.cleanup += { loaded.pool.stop(streamId) }
                            notifyStarted(onStarted)
                        }
                    }) return
                    if (streamId != 0) {
                        android.util.Log.i("TimerBell", "started slot=${ticket.slotId} kind=$kind stream=$streamId")
                        delay(kind.loadedPlaybackHoldMillis())
                        runCatching { loaded.pool.stop(streamId) }
                        return
                    }
                }
                if (attempt < 8) delay(50L)
            }
            playFallbackBell(kind, ticket, onStarted)
        } catch (canceled: kotlinx.coroutines.CancellationException) {
            gate.cancelIfOwned(ticket)
            throw canceled
        } catch (failure: Exception) {
            gate.cancelIfOwned(ticket)
            android.util.Log.w("TimerBell", "Playback failed for slot=${ticket.slotId}", failure)
        }
    }

"####;

const PLAYER_PLAYBACK: &str = r####"    private suspend fun playFallbackBell(kind: TimerBellKind, ticket: TimerBellGate.Ticket,
                                          onStarted: () -> Unit) {
        var generator: ToneGenerator? = null
        if (!gate.use(ticket) {
            generator = ToneGenerator(AudioManager.STREAM_ALARM, 88)
            ticket.cleanup += { generator?.stopTone() }
        }) return
        try {
            val pattern = when (kind) {
                TimerBellKind.FOCUS -> listOf(ToneStep(ToneGenerator.TONE_PROP_ACK, 120), ToneStep(ToneGenerator.TONE_PROP_ACK, 150))
                TimerBellKind.BREAK -> listOf(ToneStep(ToneGenerator.TONE_PROP_BEEP2, 220))
            }
            pattern.forEachIndexed { index, step ->
                if (!gate.use(ticket) {
                    generator?.startTone(step.toneType, step.durationMillis)
                    if (index == 0) notifyStarted(onStarted)
                }) return
                delay(step.durationMillis.toLong() + 70L)
            }
        } finally {
            generator?.release()
            generator = null
        }
    }

    private fun notifyStarted(onStarted: () -> Unit) {
        // A notification/channel failure must not stop an already playing bell.
        runCatching(onStarted).onFailure {
            android.util.Log.w("TimerBell", "Reminder notification unavailable", it)
        }
    }

    private fun TimerBellKind.loadedPlaybackHoldMillis(): Long = when (this) {
        TimerBellKind.FOCUS -> 700L
        TimerBellKind.BREAK -> 520L
    }
}
"####;

const NOTIFY: &str = r####"    fun notifyTransition(context: Context, transition: MicroBreakTransition, isCurrent: () -> Boolean) {
        TimerBellPlayer.play(context, transition.type.toTimerBellKind(), transition.slotId, isCurrent,
            onStarted = { refreshChannelMetadata(context); postTransitionNotification(context, transition) },
            onCanceled = { cancelSlot(context, transition.slotId) })
    }

    suspend fun notifyTransitionAndAwaitBell(context: Context, transition: MicroBreakTransition, isCurrent: () -> Boolean) {
        TimerBellPlayer.playAndAwait(context, transition.type.toTimerBellKind(), transition.slotId, isCurrent,
            onStarted = { refreshChannelMetadata(context); postTransitionNotification(context, transition) },
            onCanceled = { cancelSlot(context, transition.slotId) })
    }

"####;
