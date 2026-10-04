// v2.22.49.2 - Bind queued timer commands and notification actions to their account.
// v2.22.42 - Limit timer commands to active runs and keep side effects off the UI thread.
const REPOSITORY: &str = "com/ofairyo/gridtimer/data/TimerRepository.kt";
const VIEW_MODEL: &str = "com/ofairyo/gridtimer/ui/TimerViewModel.kt";
const SCREEN: &str = "com/ofairyo/gridtimer/ui/GridTimerScreen.kt";
const DATABASE: &str = "com/ofairyo/gridtimer/data/AppStateDatabase.kt";
const ALARM: &str = "com/ofairyo/gridtimer/notifications/MicroBreakAlarmReceiver.kt";
const ACTION: &str = "com/ofairyo/gridtimer/notifications/TimerNotificationActionReceiver.kt";
const LIVE_UPDATE: &str = "com/ofairyo/gridtimer/notifications/TimerLiveUpdateNotifier.kt";
const XIAOMI: &str = "com/ofairyo/gridtimer/notifications/XiaomiIslandNotifier.kt";
const EFFECTS: &str = "com/ofairyo/gridtimer/ui/TimerNotificationEffects.kt";

fn replace(source: &mut String, old: &str, new: &str) -> Result<(), String> {
    if source.matches(old).count() != 1 {
        return Err(format!("timer action anchor changed: {old}"));
    }
    *source = source.replacen(old, new, 1);
    Ok(())
}

pub fn render(path: &str, base: &str) -> Result<String, String> {
    let mut source = base.to_owned();
    match path {
        REPOSITORY => {
            replace(
                &mut source,
                "timerOnly -> current.timerMutationData()",
                "timerOnly -> current.timerActionProjection()",
            )?;
            replace(
                &mut source,
                "timerOnly -> current.acceptTimerMutation(normalized)",
                "timerOnly -> current.mergeTimerAction(mutationBase, normalized)",
            )?;
            let start = source
                .find("    // Only these three operations may use this projection:")
                .ok_or("old timer domain start missing")?;
            let end = source[start..]
                .find("    // Only upsert and explicit version creation")
                .map(|offset| start + offset)
                .ok_or("old timer domain end missing")?;
            source.replace_range(start..end, "");
            replace(&mut source,
                "        var didChange = false\n        var attempt = DataUpdateAttempt(succeeded = true)",
                "        val timerTrace = timerOnly && Log.isLoggable(\"TimerLatency\", Log.DEBUG)\n        val timerRequested = if (timerTrace) System.nanoTime() else 0L\n        var timerLockWait = 0L\n        var timerTransform = 0L\n        var timerPublished = 0L\n        var didChange = false\n        var attempt = DataUpdateAttempt(succeeded = true)")?;
            replace(&mut source,
                "        var updatedSnapshot: AppData? = null\n        writeMutex.withLock {",
                "        var updatedSnapshot: AppData? = null\n        writeMutex.withLock {\n            if (timerTrace) timerLockWait = System.nanoTime() - timerRequested")?;
            replace(&mut source,
                "            transformFailure?.let { failure ->",
                "            if (timerTrace) timerTransform = System.nanoTime() - timerRequested - timerLockWait\n            transformFailure?.let { failure ->")?;
            replace(&mut source,
                "                        dirtySnapshot = updated\n                        updatedSnapshot = updated\n                        didChange = true",
                "                        dirtySnapshot = updated\n                        updatedSnapshot = updated\n                        didChange = true\n                        if (timerTrace) timerPublished = System.nanoTime() - timerRequested")?;
            replace(&mut source,
                "        return attempt\n    }\n\n    // Sanitization remains authoritative.",
                "        if (timerTrace) {\n            Log.i(\"TimerLatency\", \"mutation lockUs=${timerLockWait / 1_000} transformUs=${timerTransform / 1_000} publishUs=${timerPublished / 1_000} changed=$didChange success=${attempt.succeeded}\")\n        }\n        return attempt\n    }\n\n    // Sanitization remains authoritative.")?;
            source.push_str(TIMER_SCOPE);
            replace(&mut source,
                "            now = triggeredAt,\n            alertFreshnessMillis = MICRO_BREAK_ALARM_ALERT_FRESHNESS_MILLIS,",
                "            now = maxOf(triggeredAt, now()),\n            alertFreshnessMillis = MICRO_BREAK_ALARM_ALERT_FRESHNESS_MILLIS,")?;
        }
        VIEW_MODEL => {
            replace(&mut source,
                "    fun toggleSlotRunning(slotId: Int, running: Boolean) {\n        viewModelScope.launch {",
                "    fun toggleSlotRunning(slotId: Int, running: Boolean) {\n        // Repository state is published before persistence. Alarm/bell Binder calls\n        // and persistence bookkeeping must also stay off the main dispatcher.\n        viewModelScope.launch(Dispatchers.Default) {")?;
        }
        DATABASE => {
            // Keep validation, quarantine and ordering intact. Only bound each
            // payload cursor so refilling it cannot rescan large earlier rows.
            for (start_anchor, end_anchor) in [
                (
                    "    fun forEachSnapshot(",
                    "    // Compatibility for bounded test fixtures;",
                ),
                (
                    "    fun forEachRetainedSnapshot(",
                    "    fun forEachQuarantinedCurrentSnapshot(",
                ),
                (
                    "    private fun quarantineDigestMismatchesInTransaction(",
                    "    private fun quarantineUnverifiedCurrentInTransaction(",
                ),
            ] {
                let start = source
                    .find(start_anchor)
                    .ok_or("snapshot visitor start missing")?;
                let end = source[start..]
                    .find(end_anchor)
                    .map(|n| start + n)
                    .ok_or("snapshot visitor end missing")?;
                let old = &source[start..end];
                let mut bounded = old.replace("database.query(", "database.visitSnapshotRows(");
                // These are the only payload .use blocks in the selected spans.
                // The quarantine raw query is converted separately below.
                bounded = bounded.replace(") .use { cursor ->", ") { cursor ->");
                bounded = bounded.replace(").use { cursor ->", ") { cursor ->");
                source.replace_range(start..end, &bounded);
            }
            replace(
                &mut source,
                r####"        database.rawQuery(
            """
            SELECT id, schema_version, revision, item_count, app_data_json,
                observed_sha256 AS sha256,
                source_timestamp_epoch_millis AS created_at_epoch_millis
            FROM $QUARANTINE_TABLE
            WHERE workspace_key = ?
            ORDER BY quarantined_at_epoch_millis DESC, id DESC
            """.trimIndent(),
            arrayOf(workspaceKey)
        ) { cursor ->"####,
                r####"        database.visitSnapshotRows(
            QUARANTINE_TABLE,
            arrayOf("id", "schema_version", "revision", "item_count", "app_data_json",
                "observed_sha256 AS sha256", "source_timestamp_epoch_millis AS created_at_epoch_millis"),
            "workspace_key = ?", arrayOf(workspaceKey), null, null,
            "quarantined_at_epoch_millis DESC, id DESC"
        ) { cursor ->"####,
            )?;
            source.push_str(SNAPSHOT_VISITOR);
        }
        ALARM => {
            replace(&mut source, "import android.os.PowerManager", "import android.os.PowerManager\nimport android.os.Handler\nimport android.os.Looper")?;
            replace(&mut source, "        val wakeLock = appContext.acquireMicroBreakWakeLock()",
                "        val wakeLock = runCatching { appContext.acquireMicroBreakWakeLock() }.getOrNull()\n        val completion = AlarmDeliveryCompletion {\n            try {\n                if (wakeLock?.isHeld == true) runCatching { wakeLock.release() }\n            } finally {\n                pendingResult.finish()\n            }\n        }\n        val deadlineHandler = Handler(Looper.getMainLooper())\n        val deadline = Runnable {\n            completion.complete {\n                // A queued or blocking database operation must not keep the\n                // broadcast receipt alive. The same alarm identity coalesces retries.\n                runCatching { MicroBreakAlarmScheduler.sync(appContext, 15_000L) }\n                Log.w(TAG, \"Alarm work deferred beyond broadcast budget.\")\n            }\n        }\n        deadlineHandler.postDelayed(deadline, 8_000L)")?;
            replace(&mut source, "                if (wakeLock?.isHeld == true) {\n                    runCatching { wakeLock.release() }\n                }\n                pendingResult.finish()",
                "                deadlineHandler.removeCallbacks(deadline)\n                completion.complete()")?;
            source.push_str(ALARM_COMPLETION);
        }
        _ => {}
    }
    Ok(source)
}

// Apply after the bell layer so its phase/IO hooks retain their existing anchors.
pub fn bind_workspace(path: &str, base: &str) -> Result<String, String> {
    let mut source = base.to_owned();
    match path {
        REPOSITORY => {
            for (name, arguments, next) in [
                (
                    "setSlotCategory",
                    "slotId: Int, categoryId: String?",
                    "setSlotOrder",
                ),
                (
                    "setSlotOrder",
                    "slotOrder: List<Int>",
                    "addCategoryAndAssign",
                ),
                (
                    "addCategoryAndAssign",
                    "slotId: Int?, rawName: String",
                    "startSlot",
                ),
                ("startSlot", "slotId: Int", "pauseSlot"),
                ("pauseSlots", "slotIds: Collection<Int>", "resetSlot"),
                ("resetSlot", "slotId: Int", "archiveSlot"),
                ("archiveSlot", "slotId: Int", "restoreArchivedTask"),
                ("deleteSession", "sessionId: String", "deleteArchivedTask"),
                ("deleteArchivedTask", "archivedTaskId: String", "upsertNote"),
            ] {
                let start = source
                    .find(&format!("    suspend fun {name}("))
                    .ok_or("timer command missing")?;
                let end = source[start..]
                    .find(&format!("    suspend fun {next}("))
                    .map(|n| start + n)
                    .ok_or("timer command end missing")?;
                let mut command = source[start..end].to_owned();
                replace(&mut command,
                    &format!("suspend fun {name}({arguments})"),
                    &format!("suspend fun {name}({arguments}, expectedWorkspaceKey: String = currentWorkspaceKey())"))?;
                replace(&mut command, "        awaitInitialized()", "        awaitInitialized()\n        if (activeWorkspaceKey != expectedWorkspaceKey) return")?;
                if name == "startSlot" || name == "pauseSlots" {
                    replace(&mut command, "        updateData(timerOnly = true) { data ->", "        val accepted = updateData(timerOnly = true, expectedWorkspaceKey = expectedWorkspaceKey) { data ->")?;
                } else if matches!(
                    name,
                    "setSlotCategory" | "setSlotOrder" | "addCategoryAndAssign"
                ) {
                    replace(
                        &mut command,
                        "        updateData { data ->",
                        "        updateData(expectedWorkspaceKey = expectedWorkspaceKey) { data ->",
                    )?;
                } else {
                    replace(&mut command, "        updateData { data ->", "        val accepted = updateData(expectedWorkspaceKey = expectedWorkspaceKey) { data ->")?;
                }
                match name {
                    "startSlot" => replace(&mut command, "        startedBell?.let { token ->", "        if (!accepted || activeWorkspaceKey != expectedWorkspaceKey) return\n        startedBell?.let { token ->")?,
                    "pauseSlots" => {
                        replace(&mut command, "        val pauseWorkspace = activeWorkspaceKey", "        val pauseWorkspace = expectedWorkspaceKey")?;
                        replace(&mut command, "        try {\n        val accepted = updateData", "        val accepted = try {\n        updateData")?;
                        replace(&mut command, "        persistNow(\"slot_pause\")", "        if (!accepted || activeWorkspaceKey != expectedWorkspaceKey) return\n        persistNow(\"slot_pause\")\n        if (activeWorkspaceKey != expectedWorkspaceKey) return")?;
                    }
                    "resetSlot" | "archiveSlot" | "deleteSession" | "deleteArchivedTask" => {
                        let reason = match name { "resetSlot" => "slot_reset", "archiveSlot" => "slot_archive", "deleteSession" => "session_delete", _ => "archive_delete" };
                        replace(&mut command, &format!("        persistNow(\"{reason}\")"), &format!("        if (!accepted || activeWorkspaceKey != expectedWorkspaceKey) return\n        persistNow(\"{reason}\")\n        if (activeWorkspaceKey != expectedWorkspaceKey) return"))?;
                    }
                    _ => {}
                }
                source.replace_range(start..end, &command);
            }
            replace(&mut source, "    suspend fun pauseSlot(slotId: Int) {\n        pauseSlots(listOf(slotId))", "    suspend fun pauseSlot(slotId: Int, expectedWorkspaceKey: String = currentWorkspaceKey()) {\n        pauseSlots(listOf(slotId), expectedWorkspaceKey)")?;
            replace(&mut source, "    @Volatile\n    private var pendingLiveUpdateJob: Job? = null", "    private val timerNotificationRequestLock = Any()\n    @Volatile\n    private var pendingLiveUpdateJob: Job? = null")?;
            let start = source
                .find("    private fun syncTimerLiveUpdate(")
                .ok_or("notification dispatch missing")?;
            let end = start
                + source[start..]
                    .find("    private fun logDiagnosticEvent(")
                    .ok_or("notification dispatch end missing")?;
            source.replace_range(start..end, NOTIFICATION_DISPATCH);
            // Every refresh reads current data together with its account under
            // the writer lock; old detached snapshots must not be relabelled.
            for old in [
                "syncTimerLiveUpdate(restored)",
                "syncTimerLiveUpdate(data)",
                "syncTimerLiveUpdate(prepared.data)",
                "syncTimerLiveUpdate(applied)",
            ] {
                replace(&mut source, old, "syncTimerLiveUpdate()")?;
            }
            replace(
                &mut source,
                "fallbackData?.let(::syncTimerLiveUpdate)",
                "fallbackData?.let { syncTimerLiveUpdate() }",
            )?;
        }
        VIEW_MODEL => {
            replace(&mut source, "    fun restoreArchivedTask(archivedTaskId: String, onComplete: (Int?, String) -> Unit) {\n        if (!_archiveRestoreInProgress.compareAndSet(false, true)) return\n        val expectedWorkspaceKey = currentWorkspaceKey()", "    fun restoreArchivedTask(archivedTaskId: String, expectedWorkspaceKey: String = currentWorkspaceKey(), onComplete: (Int?, String) -> Unit) {\n        if (!_archiveRestoreInProgress.compareAndSet(false, true)) return")?;
            for (name, arguments, next) in [
                (
                    "setSlotCategory",
                    "slotId: Int, categoryId: String?",
                    "updateSlotOrder",
                ),
                (
                    "updateSlotOrder",
                    "slotOrder: List<Int>",
                    "addCategoryAndAssign",
                ),
                (
                    "addCategoryAndAssign",
                    "slotId: Int?, name: String",
                    "toggleSlotRunning",
                ),
                (
                    "toggleSlotRunning",
                    "slotId: Int, running: Boolean",
                    "resetSlot",
                ),
                ("resetSlot", "slotId: Int", "archiveSlot"),
                ("archiveSlot", "slotId: Int", "restoreArchivedTask"),
                ("deleteSession", "sessionId: String", "deleteArchivedTask"),
                ("deleteArchivedTask", "archivedTaskId: String", "upsertNote"),
            ] {
                let start = source
                    .find(&format!("    fun {name}("))
                    .ok_or("timer view command missing")?;
                let end = source[start..]
                    .find(&format!("    fun {next}("))
                    .map(|n| start + n)
                    .ok_or("timer view command end missing")?;
                let mut command = source[start..end].to_owned();
                replace(&mut command, &format!("    fun {name}({arguments}) {{"), &format!("    fun {name}({arguments}, expectedWorkspaceKey: String = currentWorkspaceKey()) {{"))?;
                for (operation, call_arguments) in [
                    ("startSlot", "slotId"),
                    ("pauseSlot", "slotId"),
                    ("resetSlot", "slotId"),
                    ("archiveSlot", "slotId"),
                    ("setSlotCategory", "slotId, categoryId"),
                    ("setSlotOrder", "slotOrder"),
                    ("addCategoryAndAssign", "slotId, name"),
                    ("deleteSession", "sessionId"),
                    ("deleteArchivedTask", "archivedTaskId"),
                ] {
                    command = command.replace(
                        &format!("repository.{operation}({call_arguments})"),
                        &format!("repository.{operation}({call_arguments}, expectedWorkspaceKey)"),
                    );
                }
                source.replace_range(start..end, &command);
            }
        }
        SCREEN => {
            replace(&mut source, "TimerNotificationEffects(slots = appData.slots)", "TimerNotificationEffects(slots = appData.slots, workspaceKey = activeWorkspaceKey)")?;
            replace(
                &mut source,
                "viewModel.restoreArchivedTask(archivedTaskId) {",
                "viewModel.restoreArchivedTask(archivedTaskId, activeWorkspaceKey) {",
            )?;
            replace(
                &mut source,
                "private data class PendingHistoryDeletion(\n",
                "private data class PendingHistoryDeletion(\n    val workspaceKey: String,\n",
            )?;
            replace(
                &mut source,
                "var pendingHistoryDeletion by remember {",
                "var pendingHistoryDeletion by remember(activeWorkspaceKey) {",
            )?;
            replace(
                &mut source,
                "var homeReordering by remember {",
                "var homeReordering by remember(activeWorkspaceKey) {",
            )?;
            replace(
                &mut source,
                "var detailSlotSnapshot by remember {",
                "var detailSlotSnapshot by remember(activeWorkspaceKey) {",
            )?;
            for kind in ["SESSION", "ARCHIVED_TASK"] {
                replace(&mut source,
                    &format!("                            kind = HistoryDeletionKind.{kind},"),
                    &format!("                            workspaceKey = activeWorkspaceKey,\n                            kind = HistoryDeletionKind.{kind},"))?;
            }
            for operation in ["deleteSession", "deleteArchivedTask"] {
                replace(
                    &mut source,
                    &format!("viewModel.{operation}(deletion.id)"),
                    &format!("viewModel.{operation}(deletion.id, deletion.workspaceKey)"),
                )?;
            }
            replace(&mut source,
                "onCommitSlotOrder = viewModel::updateSlotOrder,",
                "onCommitSlotOrder = { order -> viewModel.updateSlotOrder(order, activeWorkspaceKey) },")?;
            for (old, new) in [
                ("viewModel.setSlotCategory(slot.id, it)", "viewModel.setSlotCategory(slot.id, it, activeWorkspaceKey)"),
                ("viewModel.addCategoryAndAssign(slot.id, it)", "viewModel.addCategoryAndAssign(slot.id, it, activeWorkspaceKey)"),
                ("viewModel.toggleSlotRunning(slot.id, slot.runningSinceEpochMillis != null)", "viewModel.toggleSlotRunning(slot.id, slot.runningSinceEpochMillis != null, activeWorkspaceKey)"),
                ("viewModel.resetSlot(slot.id)", "viewModel.resetSlot(slot.id, activeWorkspaceKey)"),
                ("viewModel.archiveSlot(slot.id)", "viewModel.archiveSlot(slot.id, activeWorkspaceKey)"),
            ] {
                if !source.contains(old) { return Err(format!("timer UI command missing: {old}")); }
                source = source.replace(old, new);
            }
            // Recreate the board and cancel its drag coroutine scope on account
            // changes. A retained drag must never submit an old slot order to B.
            replace(&mut source,
                "                        HomeNavigationDestination.HISTORY -> {\n                                TimerHomeScreen(",
                "                        HomeNavigationDestination.HISTORY -> {\n                            androidx.compose.runtime.key(activeWorkspaceKey) {\n                                TimerHomeScreen(")?;
            replace(&mut source,
                "                                    onPrepareDataExport = viewModel::snapshotForDataExport\n                            )\n                        }",
                "                                    onPrepareDataExport = viewModel::snapshotForDataExport\n                                )\n                            }\n                        }")?;
        }
        ACTION => {
            replace(&mut source, "        val slotIds = intent.getIntArrayExtra(EXTRA_SLOT_IDS)?.toList().orEmpty().distinct()", "        val repository = GridTimerApplication.timerRepository(context)\n        repository.awaitInitialized()\n        val expectedWorkspaceKey = currentTimerNotificationWorkspace(\n            intent.getStringExtra(EXTRA_WORKSPACE_KEY), repository.currentWorkspaceKey()\n        ) ?: return\n        val slotIds = intent.getIntArrayExtra(EXTRA_SLOT_IDS)?.toList().orEmpty().distinct()")?;
            replace(&mut source, "        val repository = GridTimerApplication.timerRepository(context)\n        repository.pauseSlots(slotIds)", "        repository.pauseSlots(slotIds, expectedWorkspaceKey)\n        if (repository.currentWorkspaceKey() != expectedWorkspaceKey) return")?;
            replace(&mut source, "        repository.flushNowBlocking(reason = \"notification_pause_action\")", "        repository.flushNowBlocking(reason = \"notification_pause_action\")\n        if (repository.currentWorkspaceKey() != expectedWorkspaceKey) return")?;
            replace(&mut source, "        private const val EXTRA_SLOT_IDS = \"com.ofairyo.gridtimer.extra.SLOT_IDS\"", "        private const val EXTRA_SLOT_IDS = \"com.ofairyo.gridtimer.extra.SLOT_IDS\"\n        private const val EXTRA_WORKSPACE_KEY = \"com.ofairyo.gridtimer.extra.WORKSPACE_KEY\"")?;
            replace(&mut source, "        fun createPauseIntent(context: Context, slotIds: List<Int>): Intent {", "        fun createPauseIntent(context: Context, slotIds: List<Int>, workspaceKey: String): Intent {")?;
            replace(&mut source, "                putExtra(EXTRA_SLOT_IDS, slotIds.toIntArray())", "                // Extras do not participate in PendingIntent identity. Keep an old\n                // account's action from being rewritten by FLAG_UPDATE_CURRENT.\n                data = android.net.Uri.parse(\"gridtimer://timer-pause/\" + android.net.Uri.encode(workspaceKey))\n                putExtra(EXTRA_SLOT_IDS, slotIds.toIntArray())\n                putExtra(EXTRA_WORKSPACE_KEY, workspaceKey)")?;
            replace(&mut source, "        if (slotIds.isEmpty()) {\n            TimerLiveUpdateNotifier.cancel(context)", "        if (slotIds.isEmpty()) {\n            repository.refreshTimerNotification(expectedWorkspaceKey)")?;
            let start = source
                .find("        val runningSlots = repository.appData.value.slots")
                .ok_or("action refresh missing")?;
            let end = start
                + source[start..]
                    .find("    }\n\n    companion object")
                    .ok_or("action refresh end missing")?;
            source.replace_range(
                start..end,
                "        repository.refreshTimerNotification(expectedWorkspaceKey)\n",
            );
            source.push_str(NOTIFICATION_WORKSPACE);
        }
        LIVE_UPDATE => {
            replace(&mut source, "    fun sync(context: Context, runningSlots: List<TimerSlot>) {", "    fun sync(context: Context, runningSlots: List<TimerSlot>, workspaceKey: String) {")?;
            replace(
                &mut source,
                "    fun cancel(context: Context) {",
                "    fun cancel(context: Context, workspaceKey: String) {",
            )?;
            let lock = "        synchronized(operationLock) {\n";
            if source.matches(lock).count() != 2 {
                return Err("notification operation locks changed".into());
            }
            source = source.replace(lock, "        synchronized(operationLock) {\n            if (currentTimerNotificationWorkspace(workspaceKey, com.ofairyo.gridtimer.GridTimerApplication.timerRepository(context).currentWorkspaceKey()) == null) return\n");
            source = source.replace("cancel(context)", "cancel(context, workspaceKey)");
            replace(&mut source, "XiaomiIslandNotifier.prepare(context = context, state = state)", "XiaomiIslandNotifier.prepare(context = context, state = state, workspaceKey = workspaceKey)")?;
            replace(&mut source, "TimerNotificationActionReceiver.createPauseIntent(context, state.runningSlotIds)", "TimerNotificationActionReceiver.createPauseIntent(context, state.runningSlotIds, workspaceKey)")?;
            replace(&mut source, "                notificationManager.notify(TIMER_LIVE_UPDATE_NOTIFICATION_ID, notification)", "                if (currentTimerNotificationWorkspace(workspaceKey, com.ofairyo.gridtimer.GridTimerApplication.timerRepository(context).currentWorkspaceKey()) == null) return\n                notificationManager.notify(TIMER_LIVE_UPDATE_NOTIFICATION_ID, notification)")?;
        }
        XIAOMI => {
            replace(&mut source, "    fun prepare(context: Context, state: TimerLiveUpdateState): XiaomiIslandPlan {", "    fun prepare(context: Context, state: TimerLiveUpdateState, workspaceKey: String): XiaomiIslandPlan {")?;
            replace(
                &mut source,
                "            buildXiaomiFocusExtras(",
                "            buildXiaomiFocusExtras(\n                workspaceKey = workspaceKey,",
            )?;
            replace(
                &mut source,
                "    private fun buildXiaomiFocusExtras(\n",
                "    private fun buildXiaomiFocusExtras(\n        workspaceKey: String,\n",
            )?;
            replace(
                &mut source,
                "buildXiaomiActionExtras(context, state)",
                "buildXiaomiActionExtras(context, state, workspaceKey)",
            )?;
            replace(&mut source, "        state: TimerLiveUpdateState\n    ): Bundle {", "        state: TimerLiveUpdateState,\n        workspaceKey: String\n    ): Bundle {")?;
            replace(&mut source, "TimerNotificationActionReceiver.createPauseIntent(context, state.runningSlotIds)", "TimerNotificationActionReceiver.createPauseIntent(context, state.runningSlotIds, workspaceKey)")?;
        }
        EFFECTS => {
            replace(&mut source, "internal fun TimerNotificationEffects(slots: List<TimerSlot>) {", "internal fun TimerNotificationEffects(slots: List<TimerSlot>, workspaceKey: String) {")?;
            replace(&mut source, "    val latestRunningSlots by rememberUpdatedState(runningSlots)\n    val latestContext by rememberUpdatedState(context)", "    val latestWorkspaceKey by rememberUpdatedState(workspaceKey)")?;
            source = source.replace(
                "context = latestContext,\n",
                "workspaceKey = latestWorkspaceKey,\n",
            );
            source = source.replace("context = context,\n", "workspaceKey = workspaceKey,\n");
            source = source.replace("appContext = appContext,\n                                runningSlots = latestRunningSlots", "appContext = appContext");
            for indent in ["                ", "                    "] {
                source = source.replace(
                    &format!("appContext = appContext,\n{indent}runningSlots = runningSlots"),
                    "appContext = appContext",
                );
            }
            source = source.replace(
                "LaunchedEffect(runningSlots, context,",
                "LaunchedEffect(workspaceKey, runningSlots, context,",
            );
            let start = source
                .find("private fun syncTimerLiveUpdateNow(")
                .ok_or("UI notification dispatch missing")?;
            let end = start
                + source[start..]
                    .find("internal fun shouldSyncTimerLiveUpdate(")
                    .ok_or("UI notification dispatch end missing")?;
            source.replace_range(start..end, "private fun syncTimerLiveUpdateNow(workspaceKey: String, appContext: Context) {\n    com.ofairyo.gridtimer.GridTimerApplication.timerRepository(appContext)\n        .refreshTimerNotification(workspaceKey)\n}\n\n");
        }
        _ => {}
    }
    Ok(source)
}

pub const NOTIFICATION_WORKSPACE: &str = r####"

internal fun currentTimerNotificationWorkspace(expected: String?, current: String): String? =
    expected?.takeIf { it.isNotBlank() && it == current }
"####;

const NOTIFICATION_DISPATCH: &str = r####"    private fun syncTimerLiveUpdate() = refreshTimerNotification(currentWorkspaceKey())

    fun refreshTimerNotification(expectedWorkspaceKey: String) {
        synchronized(timerNotificationRequestLock) {
            if (activeWorkspaceKey != expectedWorkspaceKey) return
            pendingLiveUpdateJob?.cancel()
            pendingLiveUpdateJob = scope.launch {
                awaitInitialized()
                val runningSlots = writeMutex.withLock {
                    if (activeWorkspaceKey != expectedWorkspaceKey) return@launch
                    _appData.value.slots.filter { it.runningSinceEpochMillis != null }.sortedBy(TimerSlot::id)
                }
                runCatching {
                    TimerLiveUpdateNotifier.refreshChannelMetadata(appContext)
                    TimerLiveUpdateNotifier.sync(appContext, runningSlots, expectedWorkspaceKey)
                }.onFailure { throwable ->
                    logDiagnosticEvent(
                        category = "notification.live_update",
                        message = "Failed to sync timer live update notification from repository.",
                        throwable = throwable
                    )
                }
            }
        }
    }

"####;

pub const SNAPSHOT_VISITOR: &str = r####"

// SQLiteCursor counts/refills an entire result. With hundreds of large JSON
// rows that can reread the same payloads quadratically. Enumerate only row IDs,
// then read one selected row per cursor, retaining the original filter/order/limit.
private fun android.database.sqlite.SQLiteDatabase.visitSnapshotRows(
    table: String,
    columns: Array<String>,
    selection: String?,
    selectionArgs: Array<String>?,
    groupBy: String?,
    having: String?,
    orderBy: String?,
    limit: String? = null,
    action: (android.database.Cursor) -> Unit
) {
    check(groupBy == null && having == null) { "Snapshot reads cannot aggregate rows." }
    // Pin the connection and row set for the whole visit. A concurrent REPLACE
    // must not remove a selected row ID between enumeration and payload reading.
    beginTransactionNonExclusive()
    try {
        val rowIds = query(table, arrayOf("rowid"), selection, selectionArgs,
            null, null, orderBy, limit).use { cursor ->
            buildList { while (cursor.moveToNext()) add(cursor.getLong(0)) }
        }
        val rowSelection = "rowid = ?" + (selection?.let { " AND ($it)" } ?: "")
        rowIds.forEach { rowId ->
            query(table, columns, rowSelection,
                arrayOf(rowId.toString(), *(selectionArgs ?: emptyArray())),
                null, null, null, "1").use(action)
        }
        setTransactionSuccessful()
    } finally {
        endTransaction()
    }
}
"####;

pub const ALARM_COMPLETION: &str = r####"

internal class AlarmDeliveryCompletion(private val finish: () -> Unit) {
    private val finished = java.util.concurrent.atomic.AtomicBoolean(false)

    fun complete(beforeFinish: () -> Unit = {}) {
        if (!finished.compareAndSet(false, true)) return
        try {
            beforeFinish()
        } finally {
            finish()
        }
    }
}
"####;

pub const TIMER_SCOPE: &str = r####"

// A start never creates a session for its new run. Pause and phase advancement
// can only produce sessions for runs already present in the input slots. Keep
// those runs' existing sessions and deletion revisions for collision/replay rules;
// unrelated history never enters JNI, revision stamping or sanitization.
private fun timerActionRunPrefixes(slots: List<TimerSlot>): List<String> = slots.mapNotNull { slot ->
    slot.runningSinceEpochMillis?.let { started ->
        val run = slot.activeRunId.ifEmpty { deterministicTimerRunId(slot.id, started.coerceAtLeast(0L)) }
        "$run-focus-"
    }
}.distinct()

internal fun AppData.timerActionProjection(): AppData {
    val prefixes = timerActionRunPrefixes(slots)
    fun belongs(id: String): Boolean = prefixes.any(id::startsWith)
    return AppData(
        schemaVersion = schemaVersion,
        categories = categories,
        slots = slots,
        slotOrder = slotOrder,
        slotOrderUpdatedAtEpochMillis = slotOrderUpdatedAtEpochMillis,
        sessions = if (prefixes.isEmpty()) emptyList() else sessions.filter { belongs(it.id) },
        tombstones = if (prefixes.isEmpty()) emptyList() else tombstones.filter {
            it.entityType == TOMBSTONE_ENTITY_SESSION && belongs(it.entityId)
        }
    )
}

internal fun AppData.mergeTimerAction(projection: AppData, result: AppData): AppData {
    check(projection.slots === slots && projection.categories === categories && projection.slotOrder === slotOrder) {
        "Timer input changed before publication."
    }
    val prefixes = timerActionRunPrefixes(projection.slots)
    check(result.sessions.all { session -> prefixes.any(session.id::startsWith) }) {
        "Timer command returned a session outside its active runs."
    }
    val nextSlots = if (result.slots == slots) slots else result.slots
    val nextSessions = if (result.sessions == projection.sessions) {
        // Starting a paused timer normally takes this branch. Preserve identity
        // so the UI does not rebuild history indexes after a start.
        sessions
    } else {
        val replacedIds = (projection.sessions.asSequence() + result.sessions.asSequence())
            .map(TimerSession::id).toHashSet()
        (result.sessions + sessions.filterNot { it.id in replacedIds })
            .sortedByDescending(TimerSession::updatedAtEpochMillis)
            .let { if (it == sessions) sessions else it }
    }
    return if (nextSlots === slots && nextSessions === sessions) this
    else copy(slots = nextSlots, sessions = nextSessions)
}
"####;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_timer_pipeline_retains_storage_and_read_only_guards() {
        let base = super::super::android_timer_latency::tests::rendered(REPOSITORY);
        let base = super::super::android_note_latency::render(REPOSITORY, &base).unwrap();
        let base = super::super::android_test_fixes::render(REPOSITORY, &base).unwrap();
        let base = super::super::android_startup_loading::render(REPOSITORY, &base).unwrap();
        let base = super::super::android_privacy_generation::render(REPOSITORY, &base).unwrap();
        let source = render(REPOSITORY, &base).unwrap();
        assert!(source.contains("timerOnly -> current.timerActionProjection()"));
        assert!(source.contains("timerOnly -> current.mergeTimerAction(mutationBase, normalized)"));
        for guard in [
            "persistNow(\"slot_start\")",
            "persistNow(\"slot_pause\")",
            "if (reportPersistenceReadOnly())",
            "stateDatabase.withSnapshotWriteTransaction",
            "check(futureSchema == null)",
        ] {
            assert_eq!(source.matches(guard).count(), base.matches(guard).count());
        }
    }
}
