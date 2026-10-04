// v0.0.3 - Verify UI origin and notification preparation preserve account identity.
// v0.0.2 - Also verify category, ordering and history commands across account switches.
// v0.0.1 - Execute the generated timer commands against controlled account switches.
// Standalone rustc utility. Android calls and storage are fixture adapters; the
// command bodies, VM dispatch and workspace write barrier come from generated code.
use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn span<'a>(source: &'a str, begin: &str, end: &str) -> &'a str {
    let start = source
        .find(begin)
        .unwrap_or_else(|| panic!("missing {begin}"));
    let finish = start
        + source[start..]
            .find(end)
            .unwrap_or_else(|| panic!("missing {end}"));
    &source[start..finish]
}

fn main() {
    let root = env::args().nth(1).expect("project root");
    let root = Path::new(&root);
    let generated = env::args().nth(2).map(PathBuf::from).unwrap_or_else(|| {
        root.join("app/build/generated/source/rustAndroid/main/com/ofairyo/gridtimer")
    });
    let repository = fs::read_to_string(generated.join("data/TimerRepository.kt")).unwrap();
    let model = fs::read_to_string(generated.join("ui/TimerViewModel.kt")).unwrap();
    let action =
        fs::read_to_string(generated.join("notifications/TimerNotificationActionReceiver.kt"))
            .unwrap();
    let commands = [
        span(
            &repository,
            "    suspend fun setSlotCategory(",
            "    suspend fun restoreArchivedTask(",
        ),
        span(
            &repository,
            "    suspend fun deleteSession(",
            "    suspend fun upsertNote(",
        ),
    ]
    .join("\n");
    let dispatch = [
        span(
            &model,
            "    fun setSlotCategory(",
            "    fun restoreArchivedTask(",
        ),
        span(&model, "    fun deleteSession(", "    fun upsertNote("),
    ]
    .join("\n");
    let guard = span(&repository,
        "            if (expectedWorkspaceKey != null && activeWorkspaceKey != expectedWorkspaceKey)",
        "            if (reportPersistenceReadOnly())");
    let notification_dispatch = span(
        &repository,
        "    private fun syncTimerLiveUpdate(",
        "    private fun logDiagnosticEvent(",
    );
    let action_guard = &action[action
        .find("internal fun currentTimerNotificationWorkspace(")
        .unwrap()..];
    assert!(commands.contains("expectedWorkspaceKey = expectedWorkspaceKey"));
    for case in [
        "baseline",
        "wording_only",
        "removed_write_account",
        "removed_click_account",
        "removed_notification_account",
        "removed_notification_dispatch_account",
    ] {
        let mut commands = commands.to_owned();
        let mut dispatch = dispatch.to_owned();
        let mut action_guard = action_guard.to_owned();
        let mut notification_dispatch = notification_dispatch.to_owned();
        if case == "wording_only" {
            commands = commands
                .replace("Starting slotId=", "Start timer slotId=")
                .replace("Pausing slotIds=", "Pause selected timers slotIds=");
        }
        if case == "removed_write_account" {
            commands = commands.replace(
                "expectedWorkspaceKey = expectedWorkspaceKey",
                "expectedWorkspaceKey = null",
            );
        }
        if case == "removed_click_account" {
            dispatch = dispatch.replace(", expectedWorkspaceKey)", ")");
        }
        if case == "removed_notification_account" {
            action_guard = action_guard.replace("it.isNotBlank() && it == current", "true");
        }
        if case == "removed_notification_dispatch_account" {
            notification_dispatch = notification_dispatch.replace(
                "runningSlots, expectedWorkspaceKey)",
                "runningSlots, currentWorkspaceKey())",
            );
        }
        let output = root
            .join("release_artifacts/verification/v2.22.49.2/timer_workspace_audit")
            .join(case);
        fs::create_dir_all(&output).unwrap();
        let fixture = FIXTURE
            .replace("@COMMANDS@", &commands)
            .replace("@GUARD@", guard)
            .replace("@DISPATCH@", &dispatch)
            .replace("@NOTIFICATION_DISPATCH@", &notification_dispatch)
            .replace("@NOTIFICATION@", &action_guard);
        fs::write(output.join("TimerWorkspaceAudit.kt"), fixture).unwrap();
    }
}

const FIXTURE: &str = r####"package audit

import com.ofairyo.gridtimer.data.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import org.junit.Assert.*
import org.junit.Test
import java.util.UUID
import kotlin.coroutines.CoroutineContext

private class QueuedDispatcher : CoroutineDispatcher() {
    val pending = java.util.ArrayDeque<Runnable>()
    override fun dispatch(context: CoroutineContext, block: Runnable) { pending.add(block) }
    fun drain() { while (!pending.isEmpty()) pending.removeFirst().run() }
}
private object Dispatchers { val Default = QueuedDispatcher() }
private object NativeOptimizerBridge {
    fun normalizeRepositoryText(value: String, maxLength: Int, trimStartOnly: Boolean, compactWhitespace: Boolean): String? = null
    fun setSlotCategoryAppDataJson(appDataJson: String, slotId: Int, categoryId: String?, now: Long): String? = null
    fun setSlotOrderAppDataJson(appDataJson: String, slotOrder: IntArray, now: Long): String? = null
    fun addCategoryAndAssignAppDataJson(appDataJson: String, slotId: Int?, categoryId: String, name: String, now: Long): String? = null
    fun startSlotAppDataJson(appDataJson: String, slotId: Int, now: Long): String? = null
    fun pauseSlotsAppDataJson(appDataJson: String, slotIds: IntArray, now: Long): String? = null
    fun resetSlotAppDataJson(appDataJson: String, slotId: Int, now: Long): String? = null
    fun archiveSlotAppDataJson(appDataJson: String, slotId: Int, archivedTaskId: String, now: Long): String? = null
}
private object TimerBellPlayer {
    var calls = 0
    fun reconcile() {}
    fun play(context: Unit, kind: MicroBreakPhase, slotId: Int, valid: () -> Boolean) { if (valid()) calls++ }
}
private object MicroBreakReminderNotifier {
    var calls = 0
    fun cancelSlot(context: Unit, id: Int) { calls++ }
}
private object TimerLiveUpdateNotifier {
    var beforeSync: (() -> Unit)? = null
    var currentWorkspace: () -> String = { "account-a" }
    val posted = mutableListOf<String>()
    fun refreshChannelMetadata(context: Unit) { beforeSync?.invoke() }
    fun sync(context: Unit, slots: List<TimerSlot>, workspaceKey: String) {
        if (currentTimerNotificationWorkspace(workspaceKey, currentWorkspace()) != null) posted += workspaceKey
    }
}
private fun MicroBreakPhase.toTimerBellKind() = this
private class Repository(initial: AppData) {
    var activeWorkspaceKey = "account-a"
    val _appData = MutableStateFlow(initial)
    val writeMutex = Mutex()
    private val timerNotificationRequestLock = Any()
    private var pendingLiveUpdateJob: Job? = null
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    var beforeWrite: (() -> Unit)? = null
    var saves = 0
    val bellDispatch = TimerBellDispatch()
    val appContext = Unit
    val accentSeeds = listOf("red")
    val json = Json
    fun currentWorkspaceKey() = activeWorkspaceKey
    suspend fun awaitInitialized() {}
    fun now() = 5_000L
    fun logDiagnosticEvent(category: String, message: String, throwable: Throwable? = null) {}
    fun historyDeletionSummaryText(sessionId: String, archivedTaskId: String) = ""
    fun decodeNativeAppData(source: String?): AppData? = null
    suspend fun persistNow(reason: String) { saves++ }
    fun isCurrentBell(token: TimerBellToken) = token.isCurrent(activeWorkspaceKey, _appData.value.slots.firstOrNull { it.id == token.slotId }, now())
    data class DataUpdateAttempt(val succeeded: Boolean = true, val failure: NoteSaveFailure? = null, val detail: String = "")
    suspend fun updateData(timerOnly: Boolean = false, expectedWorkspaceKey: String? = null, transform: (AppData) -> AppData): Boolean {
        beforeWrite?.invoke()
        var attempt = DataUpdateAttempt()
        writeMutex.withLock {
@GUARD@
            _appData.value = transform(_appData.value)
        }
        return attempt.succeeded
    }
@COMMANDS@
@NOTIFICATION_DISPATCH@
}
private class ViewModel(val repository: Repository) {
    val viewModelScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    fun currentWorkspaceKey() = repository.currentWorkspaceKey()
@DISPATCH@
}
@NOTIFICATION@

class TimerWorkspaceAudit {
    private val commandNames = listOf("start", "pause", "reset", "archive", "category", "order", "create", "delete-session", "delete-archive")
    private fun data(running: Boolean = false): AppData = AppData(slots = listOf(
        TimerSlot(id = 1, title = "A task", categoryId = "shared-category", accumulatedMillis = 1_000L,
            runningSinceEpochMillis = if (running) 1_000L else null,
            activeRunId = if (running) "run-a" else ""), TimerSlot(id = 2)),
        categories = listOf(Category(id = "shared-category", name = "Existing category")),
        sessions = listOf(TimerSession(id = "shared-session", slotId = 1, slotTitle = "A task", startedAtEpochMillis = 100L, endedAtEpochMillis = 200L, durationMillis = 100L)),
        archivedTasks = listOf(ArchivedTask(id = "shared-archive", originalSlotId = 1, title = "A archive", accumulatedMillis = 100L, archivedAtEpochMillis = 200L)))
    private fun other(running: Boolean = false) = data(running).copy(slots = data(running).slots.map {
        it.copy(title = "B task", activeRunId = if (running) "run-b" else "") })
    private suspend fun command(repo: Repository, name: String, expected: String) {
        when (name) {
            "start" -> repo.startSlot(1, expected)
            "pause" -> repo.pauseSlot(1, expected)
            "reset" -> repo.resetSlot(1, expected)
            "archive" -> repo.archiveSlot(1, expected)
            "category" -> repo.setSlotCategory(1, null, expected)
            "order" -> repo.setSlotOrder(listOf(2, 1), expected)
            "create" -> repo.addCategoryAndAssign(1, "New category", expected)
            "delete-session" -> repo.deleteSession("shared-session", expected)
            "delete-archive" -> repo.deleteArchivedTask("shared-archive", expected)
        }
    }
    @Test fun currentAccountCommandsStillChangeTheRequestedTimer() = runBlocking {
        for (name in commandNames) {
            val initial = data(name == "pause")
            val repo = Repository(initial)
            command(repo, name, "account-a")
            assertNotEquals(name, initial, repo._appData.value)
            assertEquals(name, if (name in listOf("category", "order", "create")) 0 else 1, repo.saves)
        }
    }
    @Test fun accountSwitchWhileWaitingForTheWriterCannotTouchTheNewAccount() = runBlocking {
        val changed = mutableListOf<String>()
        for (name in commandNames) {
            val repo = Repository(data(name == "pause"))
            val untouched = other(name == "pause")
            repo.beforeWrite = { repo.activeWorkspaceKey = "account-b"; repo._appData.value = untouched }
            command(repo, name, "account-a")
            if (untouched != repo._appData.value) changed += name
            assertEquals(name, 0, repo.saves)
        }
        assertTrue("Commands modified the other account after waiting for the writer: $changed", changed.isEmpty())
    }
    @Test fun accountSwitchBeforeCoroutineStartsRetainsTheClickAccount() {
        val changed = mutableListOf<String>()
        for (name in commandNames) {
            val repo = Repository(data(name == "pause"))
            val vm = ViewModel(repo)
            when (name) {
                "start" -> vm.toggleSlotRunning(1, false)
                "pause" -> vm.toggleSlotRunning(1, true)
                "reset" -> vm.resetSlot(1)
                "archive" -> vm.archiveSlot(1)
                "category" -> vm.setSlotCategory(1, null)
                "order" -> vm.updateSlotOrder(listOf(2, 1))
                "create" -> vm.addCategoryAndAssign(1, "New category")
                "delete-session" -> vm.deleteSession("shared-session")
                "delete-archive" -> vm.deleteArchivedTask("shared-archive")
            }
            val untouched = other(name == "pause")
            repo.activeWorkspaceKey = "account-b"
            repo._appData.value = untouched
            Dispatchers.Default.drain()
            if (untouched != repo._appData.value || repo.saves != 0) changed += name
            vm.viewModelScope.cancel()
        }
        assertTrue("Commands lost the click account before dispatch: $changed", changed.isEmpty())
    }
    @Test fun retainedUiActionsKeepTheirOriginAccountAfterSwitching() {
        val changed = mutableListOf<String>()
        for (name in commandNames) {
            val untouched = other(name == "pause")
            val repo = Repository(untouched)
            repo.activeWorkspaceKey = "account-b"
            val vm = ViewModel(repo)
            when (name) {
                "start" -> vm.toggleSlotRunning(1, false, "account-a")
                "pause" -> vm.toggleSlotRunning(1, true, "account-a")
                "reset" -> vm.resetSlot(1, "account-a")
                "archive" -> vm.archiveSlot(1, "account-a")
                "category" -> vm.setSlotCategory(1, null, "account-a")
                "order" -> vm.updateSlotOrder(listOf(2, 1), "account-a")
                "create" -> vm.addCategoryAndAssign(1, "New category", "account-a")
                "delete-session" -> vm.deleteSession("shared-session", "account-a")
                "delete-archive" -> vm.deleteArchivedTask("shared-archive", "account-a")
            }
            Dispatchers.Default.drain()
            if (untouched != repo._appData.value || repo.saves != 0) changed += name
            vm.viewModelScope.cancel()
        }
        assertTrue("Retained UI actions changed the new account: $changed", changed.isEmpty())
    }
    @Test fun staleLegacyAndDifferentAccountNotificationActionsAreRejected() {
        assertEquals("account-a", currentTimerNotificationWorkspace("account-a", "account-a"))
        assertNull(currentTimerNotificationWorkspace(null, "account-b"))
        assertNull(currentTimerNotificationWorkspace("account-a", "account-b"))
        assertNull(currentTimerNotificationWorkspace("", "account-b"))
    }
    @Test fun notificationPreparationCannotRelabelAnOldAccountAsTheNewOne() {
        Dispatchers.Default.drain()
        val repo = Repository(data(true))
        TimerLiveUpdateNotifier.posted.clear()
        TimerLiveUpdateNotifier.currentWorkspace = repo::currentWorkspaceKey
        TimerLiveUpdateNotifier.beforeSync = { repo.activeWorkspaceKey = "account-b"; repo._appData.value = other(true) }
        try {
            repo.refreshTimerNotification("account-a")
            Dispatchers.Default.drain()
            assertTrue("Old notification was posted for the new account: ${TimerLiveUpdateNotifier.posted}", TimerLiveUpdateNotifier.posted.isEmpty())
            TimerLiveUpdateNotifier.beforeSync = null
            repo.refreshTimerNotification("account-b")
            Dispatchers.Default.drain()
            assertEquals(listOf("account-b"), TimerLiveUpdateNotifier.posted)
        } finally {
            TimerLiveUpdateNotifier.beforeSync = null
            repo.scope.cancel()
        }
    }
}
"####;
