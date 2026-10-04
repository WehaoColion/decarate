// v2.22.49.2 - Cover encrypted-note unlock cancellation and account isolation.
// v2.22.49.1 - Cover published/cold-start phase bells and collector handoff.
// v2.22.49 - Require every asynchronous verdict and join workers before startup recovery.
// v2.22.48 - Verify paused, superseded and expired bell requests and committed transitions.
// v2.22.47 - Verify structured page history, edit barriers and portable attachment kinds.
// v2.22.45 - Verify startup receipt account, history and mutation boundaries.
// v2.22.44 - Verify ordinary page retention and encrypted-session lifecycle boundaries.
// v2.22.42 - Verify timer projection, historical preservation and active-run boundaries.
pub const PATH: &str = "com/ofairyo/gridtimer/ui/TenfoldEditingTest.kt";

// Execute the repository's own transaction and finance mutation bodies with
// deterministic storage ports. The fixture never duplicates their commit logic.
pub fn finance_repository_fixture() -> String {
    let repository = super::kotlin_sources::SOURCES
        .iter()
        .find(|source| source.path.ends_with("/TimerRepository.kt"))
        .unwrap()
        .contents;
    fn section<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
        let from = source.find(start).unwrap();
        let to = from + source[from..].find(end).unwrap();
        &source[from..to]
    }
    let finance = section(
        repository,
        "    suspend fun updateFinanceProfile(",
        "    suspend fun restoreFinanceProfileSafely(",
    );
    let transaction = section(
        repository,
        "    private suspend fun updateDataDetailed(",
        "    private class RevisionOverflowException",
    );
    let results = section(
        repository,
        "private data class DataUpdateAttempt(",
        "private data class NoteDraftRecovery(",
    );
    format!("{results}\n{FINANCE_FIXTURE_PREFIX}\n{finance}\n{transaction}\n}}\n")
}

pub fn render() -> String {
    format!("{CONTENTS}\n{}", finance_repository_fixture())
}

const FINANCE_FIXTURE_PREFIX: &str = r####"
private class FinanceRepositoryStorageFixture(initial: FinanceProfile) {
    private val _appData = kotlinx.coroutines.flow.MutableStateFlow(AppData(financeProfile = initial))
    val data: AppData get() = _appData.value
    var failWrites = false
    var publishedAtPersist: AppData? = null
    private val writeMutex = Mutex()
    private var activeWorkspaceKey = "guest"
    private var pendingPersistJob: kotlinx.coroutines.Job? = null
    private var dirtySnapshot: AppData? = null
    private var lastPersistedSnapshot: AppData? = null
    private val _syncSession = kotlinx.coroutines.flow.MutableStateFlow(SyncAccountSession())
    private val json = Json { encodeDefaults = true }
    private val TAG = "FinanceStorageFixture"
    private class RevisionOverflowException(message: String) : IllegalStateException(message)
    private suspend fun awaitInitialized() = Unit
    private fun now() = 1_800_000_000_000L
    private fun reportPersistenceReadOnly() = false
    private fun persistenceBlockedUpdateAttempt() = DataUpdateAttempt(false)
    private fun decodeNativeAppData(raw: String?): AppData? = raw?.let { json.decodeFromString<AppData>(it) }
    private fun stampChangedFieldRevisions(previous: AppData, next: AppData, mutationAt: Long) = next
    private fun logDiagnosticEvent(category: String, message: String, throwable: Throwable? = null) = Unit
    private fun updateSyncSession(value: SyncAccountSession) { _syncSession.value = value }
    private fun syncMicroBreakAlarm(data: AppData) = Unit
    private fun schedulePersist() = Unit
    private fun schedulePersistRetry() = Unit
    private fun persistSafelyDetailed(updated: AppData): PersistAttempt {
        publishedAtPersist = _appData.value
        return if (failWrites) PersistAttempt(false, NoteSaveFailure.STORAGE_FULL) else PersistAttempt(true)
    }
    private fun flushPersistLockedDetailed(updated: AppData) = persistSafelyDetailed(updated)
    fun observeAnotherWrite(profile: FinanceProfile) { _appData.value = _appData.value.copy(financeProfile = profile) }
"####;

pub const CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui

import android.util.Log
import com.ofairyo.gridtimer.data.*
import com.ofairyo.gridtimer.core.NativeOptimizerBridge
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext
import kotlinx.coroutines.sync.withLock
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import com.ofairyo.gridtimer.data.NoteBlock
import com.ofairyo.gridtimer.data.NoteBlockType
import com.ofairyo.gridtimer.data.AppData
import com.ofairyo.gridtimer.data.NoteEntry
import com.ofairyo.gridtimer.data.NoteEncryptionEnvelope
import com.ofairyo.gridtimer.data.TimerRepository
import com.ofairyo.gridtimer.data.TimerSession
import com.ofairyo.gridtimer.data.TimerSlot
import com.ofairyo.gridtimer.data.DataTombstone
import com.ofairyo.gridtimer.data.TOMBSTONE_ENTITY_SESSION
import com.ofairyo.gridtimer.data.timerActionProjection
import com.ofairyo.gridtimer.data.mergeTimerAction
import com.ofairyo.gridtimer.data.SnapshotPrivacyRewrite
import com.ofairyo.gridtimer.data.requirePrivacySnapshotSchema
import kotlinx.serialization.encodeToString
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.async
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.coroutines.sync.Mutex
import kotlin.coroutines.intrinsics.suspendCoroutineUninterceptedOrReturn
import com.ofairyo.gridtimer.data.snapshotFutureSchema
import com.ofairyo.gridtimer.data.snapshotMediaReferences
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.json.Json
import com.ofairyo.gridtimer.diagnostics.collectAnrTrace
import org.junit.Assert.*
import org.junit.Test
import com.ofairyo.gridtimer.notifications.AlarmDeliveryCompletion
import com.ofairyo.gridtimer.notifications.TimerBellGate
import com.ofairyo.gridtimer.data.TimerBellToken
import com.ofairyo.gridtimer.data.MicroBreakPhase
import com.ofairyo.gridtimer.data.MicroBreakTransition
import com.ofairyo.gridtimer.data.MicroBreakTransitionType
import com.ofairyo.gridtimer.data.AppDataMicroBreakResolution
import com.ofairyo.gridtimer.data.currentBellTransitions
import com.ofairyo.gridtimer.data.finishTimerTransition
import com.ofairyo.gridtimer.data.TimerBellDispatch
import com.ofairyo.gridtimer.data.resolveMicroBreak
import com.ofairyo.gridtimer.data.bellToken
import kotlinx.coroutines.launch
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.CompletableDeferred

class TenfoldEditingTest {

    @Test fun markdownReadingCannotHideStructuredOrRichContent() {
        val text = NoteBlock(id = "text", type = NoteBlockType.TEXT, text = "\\[A=P(1+r)^n\\]")
        val document = NoteDocument(markdownEnabled = true, blocks = listOf(text))
        assertNotNull(documentMarkdownPreviewText(document, document.blocks))
        assertNull(documentMarkdownPreviewText(document.copy(markdownEnabled = false), document.blocks))
        assertNull(documentMarkdownPreviewText(document.copy(richTextEnabled = true), document.blocks))
        for (type in NoteBlockType.entries.filter { it != NoteBlockType.TEXT }) {
            assertNull(documentMarkdownPreviewText(document, listOf(text, NoteBlock(id = "structured", type = type))))
        }
        assertNull(documentMarkdownPreviewText(document, emptyList()))
        assertNull(documentMarkdownPreviewText(document, listOf(text.copy(text = "  \n  "))))
    }

    @Test fun splitFormulaReadingKeepsDraftIdsAndUsesCurrentComposition() {
        val blocks = listOf(
            NoteBlock(id = "open", type = NoteBlockType.TEXT, text = "\\["),
            NoteBlock(id = "formula", type = NoteBlockType.TEXT, text = "A=P(1+r)^n"),
            NoteBlock(id = "close", type = NoteBlockType.TEXT, text = "\\]")
        )
        val note = NoteEntry(id = "answer", updatedAtEpochMillis = 1234L,
            document = NoteDocument(markdownEnabled = true, blocks = blocks))
        val fields = mutableMapOf("formula" to TextFieldValue(
            "A=P \\times (1+r)^n", TextRange(4), TextRange(2, 4)))
        val previousFields = fields.toMap()
        assertEquals("\\[\nA=P \\times (1+r)^n\n\\]", documentMarkdownPreviewText(note.document!!, blocks, fields))
        assertEquals(previousFields, fields)
        assertEquals(listOf("open", "formula", "close"), blocks.map { it.id })
        assertEquals("A=P(1+r)^n", blocks[1].text)
        assertEquals(1234L, note.updatedAtEpochMillis)
    }

    @Test fun noteUnlockRejectsDeletedRekeyedOrReplacedTargets() {
        val requested = NoteEntry(id = "protected", protectionStateRevision = 4L,
            encryption = NoteEncryptionEnvelope(keyId = "key", ciphertextBase64 = "cipher"))
        assertTrue(noteUnlockTargetMatches(requested, requested))
        assertFalse(noteUnlockTargetMatches(requested, null))
        assertFalse(noteUnlockTargetMatches(requested, requested.copy(deletedAtEpochMillis = 5L)))
        assertFalse(noteUnlockTargetMatches(requested, requested.copy(encryption = null)))
        assertFalse(noteUnlockTargetMatches(requested, requested.copy(protectionStateRevision = 5L)))
        assertFalse(noteUnlockTargetMatches(requested, requested.copy(
            encryption = requested.encryption!!.copy(wrappedKeyBase64 = "rekeyed"))))
        assertFalse(noteUnlockTargetMatches(requested, requested.copy(
            encryption = requested.encryption!!.copy(ciphertextBase64 = "synced-content"))))
    }

    @Test fun financeRepositoryFailedStorageNeverPublishesTheCandidate() = runBlocking {
        val initial = FinanceProfile(cashReserve = 100L)
        val repository = FinanceRepositoryStorageFixture(initial)
        repository.failWrites = true
        val state = FinanceProfileCommitState(initial)
        val edited = initial.copy(cashReserve = 900L)
        val generation = state.submit(edited)
        val receipt = repository.updateFinanceProfile("guest", edited)
        state.observeUpstream(repository.data.financeProfile)
        assertFalse(receipt.first)
        assertNull(receipt.second)
        assertEquals(initial, repository.publishedAtPersist!!.financeProfile)
        assertEquals(initial, repository.data.financeProfile)
        assertTrue(state.complete(generation, receipt.first, receipt.second))
        assertEquals(initial, state.profile)
    }

    @Test fun financeRepositoryReceiptKeepsItsCommittedSnapshotAcrossLaterWrites() = runBlocking {
        val initial = FinanceProfile(cashReserve = 100L)
        val repository = FinanceRepositoryStorageFixture(initial)
        val saved = initial.copy(cashReserve = 200L)
        val receipt = repository.updateFinanceProfile("guest", saved)
        assertTrue(receipt.first)
        assertEquals(initial, repository.publishedAtPersist!!.financeProfile)
        assertEquals(saved, receipt.second)
        repository.observeAnotherWrite(initial.copy(cashReserve = 300L))
        assertEquals(saved, receipt.second)
        val wrongWorkspace = repository.updateFinanceProfile("account:other", saved)
        assertFalse(wrongWorkspace.first)
        assertEquals(300L, repository.data.financeProfile.cashReserve)
    }

    @Test fun financeRejectedSaveRestoresTheLastPersistedProfile() {
        val initial = com.ofairyo.gridtimer.data.FinanceProfile(cashReserve = 100L)
        val state = FinanceProfileCommitState(initial)
        val attempt = state.submit(initial.copy(cashReserve = 900L))
        assertEquals(900L, state.profile.cashReserve)
        assertTrue(state.complete(attempt, false))
        assertEquals(initial, state.profile)
    }

    @Test fun financeOldCompletionCannotEraseNewerDraftAndFailureUsesLatestSnapshot() {
        val initial = com.ofairyo.gridtimer.data.FinanceProfile(cashReserve = 100L)
        val state = FinanceProfileCommitState(initial)
        val first = state.submit(initial.copy(cashReserve = 200L))
        val second = state.submit(initial.copy(cashReserve = 300L))
        state.observeUpstream(initial.copy(cashReserve = 200L))
        assertFalse(state.complete(first, false))
        assertEquals(300L, state.profile.cashReserve)
        assertTrue(state.complete(second, false))
        assertEquals(200L, state.profile.cashReserve)
    }

    @Test fun financeSuccessfulNormalizedSaveReleasesFutureSyncUpdates() {
        val initial = com.ofairyo.gridtimer.data.FinanceProfile(cashReserve = 100L)
        val state = FinanceProfileCommitState(initial)
        val attempt = state.submit(initial.copy(cashReserve = 200L, acquisitionFocus = "  reserve  "))
        state.observeUpstream(initial.copy(cashReserve = 200L, acquisitionFocus = "reserve"))
        val committed = initial.copy(cashReserve = 200L, acquisitionFocus = "reserve")
        assertTrue(state.complete(attempt, true, committed))
        assertEquals(committed, state.profile)
        state.observeUpstream(initial.copy(cashReserve = 250L, acquisitionFocus = "reserve"))
        assertEquals(250L, state.profile.cashReserve)
    }

    @Test fun financeSuccessBeforeUpstreamKeepsTheSavedDraftForTheNextInput() {
        val initial = com.ofairyo.gridtimer.data.FinanceProfile(cashReserve = 100L)
        val saved = initial.copy(cashReserve = 200L)
        val state = FinanceProfileCommitState(initial)
        val first = state.submit(saved)
        assertFalse(state.complete(first, true, null))
        assertTrue(state.complete(first, true, saved))
        assertEquals(saved, state.profile)
        val next = state.profile.copy(liabilityBalance = 50L)
        val second = state.submit(next)
        state.observeUpstream(saved)
        assertEquals(200L, state.profile.cashReserve)
        assertEquals(50L, state.profile.liabilityBalance)
        assertTrue(state.complete(second, true, next))
        state.observeUpstream(next)
        assertEquals(next, state.profile)
    }

    @Test fun noteUnlockPublishesOnlyTheCurrentRequestAndMatchingWorkspace() {
        val gate = NoteUnlockRequestGate()
        val accepted = gate.begin("account:a", "note")
        assertTrue(gate.finish(accepted, "account:a", "note"))
        assertFalse(gate.finish(accepted, "account:a", "note"))
        val changedWorkspace = gate.begin("account:a", "note")
        assertFalse(gate.finish(changedWorkspace, "account:b", "note"))
        val mismatchedNote = gate.begin("account:a", "note")
        assertFalse(gate.finish(mismatchedNote, "account:a", "other"))
        val old = gate.begin("account:a", "note")
        val replacement = gate.begin("account:a", "note")
        assertFalse(gate.finish(old, "account:a", "note"))
        gate.discard(old)
        assertTrue(gate.finish(replacement, "account:a", "note"))
    }

    @Test fun noteUnlockCannotReopenAfterLockOrBackgroundCancellation() {
        val gate = NoteUnlockRequestGate()
        val locked = gate.begin("account:a", "note")
        val independent = gate.begin("account:a", "other")
        gate.invalidate("account:a", "note")
        assertFalse(gate.finish(locked, "account:a", "note"))
        assertTrue(gate.finish(independent, "account:a", "other"))
        val background = gate.begin("account:a", "note")
        gate.invalidateAll()
        assertFalse(gate.finish(background, "account:a", "note"))
        val retry = gate.begin("account:a", "note")
        assertTrue(gate.finish(retry, "account:a", "note"))
    }

    @Test fun noteBackgroundCancelsInflightUnlockEvenBeforeAnyNoteWasOpened() {
        val fixture = NoteBackgroundFixture()
        try {
            val request = fixture.unlockRequests.begin("account:a", "locked-note")
            fixture.event(androidx.lifecycle.Lifecycle.Event.ON_STOP)
            fixture.event(androidx.lifecycle.Lifecycle.Event.ON_START)
            assertFalse(fixture.unlockRequests.finish(request, "account:a", "locked-note"))
            assertEquals("ordinary-note", fixture.selectedId)
            assertTrue(fixture.locked.isEmpty())
        } finally { fixture.dispose() }
    }

    @Test fun publishedPhaseStillRingsOnceWhenItsTransitionListWasConsumed() {
        val dispatch = TimerBellDispatch()
        val before = TimerSlot(id = 1, runningSinceEpochMillis = 1_000L,
            activeRunId = com.ofairyo.gridtimer.data.deterministicTimerRunId(1, 1_000L) + "-nonce",
            microBreakPhase = MicroBreakPhase.BREAK)
        val boundary = before.resolveMicroBreak(16_050L)
        val published = AppDataMicroBreakResolution(AppData(slots = listOf(boundary.slot)), emptyList())
        assertEquals(boundary.transitions, currentBellTransitions(published, 16_050L, 3_000L))
        assertTrue(dispatch.claim(boundary.slot.bellToken("a", 16_050L)!!) { true })
        assertFalse(dispatch.claim(published.data.slots.single().bellToken("a", 16_100L)!!) { true })
        val rest = boundary.slot.copy(runningSinceEpochMillis = 20_000L, microBreakPhase = MicroBreakPhase.BREAK)
        assertEquals(MicroBreakTransitionType.BREAK_STARTED, currentBellTransitions(
            AppDataMicroBreakResolution(AppData(slots = listOf(rest)), emptyList()), 20_050L, 3_000L).single().type)
    }

    @Test fun recoveredReminderRejectsManualStartsPausesExpiredAndPartialRest() {
        val start = 20_000L
        val automatic = TimerSlot(id = 1, runningSinceEpochMillis = start, activeRunId = "older-run",
            microBreakPhase = MicroBreakPhase.BREAK)
        fun candidates(slot: TimerSlot, at: Long = 20_050L) = currentBellTransitions(
            AppDataMicroBreakResolution(AppData(slots = listOf(slot)), emptyList()), at, 3_000L)
        assertEquals(1, candidates(automatic).size)
        val manual = com.ofairyo.gridtimer.data.deterministicTimerRunId(1, start)
        listOf(manual, "$manual-nonce").forEach { assertTrue(candidates(automatic.copy(activeRunId = it)).isEmpty()) }
        assertTrue(candidates(automatic.copy(runningSinceEpochMillis = null)).isEmpty())
        assertTrue(candidates(automatic.copy(activeRunId = "")).isEmpty())
        assertTrue(candidates(automatic.copy(microBreakPhaseProgressMillis = 500L)).isEmpty())
        assertTrue(candidates(automatic, 19_999L).isEmpty())
        assertTrue(candidates(automatic, 23_001L).isEmpty())
        assertTrue(candidates(automatic, 35_000L).isEmpty())
    }

    @Test fun deadlineMonitorDeliversAtStartupAndAfterCollectorReplacementBeforeWaiting() = runBlocking {
        var at = 16_050L
        val previous = TimerSlot(id = 1, runningSinceEpochMillis = 1_000L, activeRunId = "older-run",
            microBreakPhase = MicroBreakPhase.BREAK)
        val published = previous.resolveMicroBreak(at).slot
        val snapshots = kotlinx.coroutines.flow.MutableStateFlow(AppData(slots = listOf(published)))
        val received = kotlinx.coroutines.channels.Channel<MicroBreakTransition>(kotlinx.coroutines.channels.Channel.UNLIMITED)
        val observer = launch {
            com.ofairyo.gridtimer.data.monitorTimerBellDeadlines(snapshots, { at }) { result, time ->
                currentBellTransitions(result, time, 3_000L).forEach { received.send(it) }
            }
        }
        try {
            val initial = withTimeoutOrNull(2_000L) { received.receive() }
            assertNotNull("A published phase must ring before its next deadline", initial)
            assertEquals(MicroBreakTransitionType.FOCUS_RESUMED, initial!!.type)
            at = 20_050L
            snapshots.value = AppData(slots = listOf(published.copy(
                runningSinceEpochMillis = 20_000L, microBreakPhase = MicroBreakPhase.BREAK)))
            val replaced = withTimeoutOrNull(2_000L) { received.receive() }
            assertNotNull("A collector replacement must retain the current reminder", replaced)
            assertEquals(MicroBreakTransitionType.BREAK_STARTED, replaced!!.type)
        } finally { observer.cancelAndJoin(); received.close() }
    }

    @Test fun startupPipelineWaitsForTheLastVerdictBeforeReadiness() {
        val entered = java.util.concurrent.CountDownLatch(1)
        val release = java.util.concurrent.CountDownLatch(1)
        val caller = java.util.concurrent.Executors.newSingleThreadExecutor()
        val result = caller.submit(java.util.concurrent.Callable {
            com.ofairyo.gridtimer.data.verifySnapshotPipeline(2, 2) { submit ->
                submit { true }
                submit { entered.countDown(); release.await(); true }
            }
        })
        try {
            assertTrue(entered.await(5, java.util.concurrent.TimeUnit.SECONDS))
            assertFalse(result.isDone)
            release.countDown()
            assertEquals(2L, result.get(5, java.util.concurrent.TimeUnit.SECONDS).toLong())
        } finally { release.countDown(); caller.shutdownNow() }
    }

    @Test fun startupPipelineRejectsFalseAndExceptionalVerdicts() {
        val badVerdicts = listOf<() -> Boolean>({ false }, { error("invalid snapshot") })
        badVerdicts.forEach { bad ->
            assertTrue(runCatching {
                com.ofairyo.gridtimer.data.verifySnapshotPipeline(2, 3) { submit ->
                    submit { true }
                    submit(bad)
                    submit { true }
                }
            }.isFailure)
        }
    }

    @Test fun startupReadFailureJoinsItsWorkerBeforeRecoveryCanBegin() {
        val entered = java.util.concurrent.CountDownLatch(1)
        val release = java.util.concurrent.CountDownLatch(1)
        val caller = java.util.concurrent.Executors.newSingleThreadExecutor()
        val result = caller.submit(java.util.concurrent.Callable {
            runCatching {
                com.ofairyo.gridtimer.data.verifySnapshotPipeline(1, 1) { submit ->
                    submit { entered.countDown(); release.await(); true }
                    check(entered.await(5, java.util.concurrent.TimeUnit.SECONDS))
                    error("cursor read failed")
                }
            }.isFailure
        })
        try {
            assertTrue(entered.await(5, java.util.concurrent.TimeUnit.SECONDS))
            assertFalse(result.isDone)
            release.countDown()
            assertTrue(result.get(5, java.util.concurrent.TimeUnit.SECONDS))
        } finally { release.countDown(); caller.shutdownNow() }
    }

    @Test fun incompleteOrDuplicatedSnapshotPageCannotOpenTheReadyGate() {
        for ((expected, actual) in listOf(2L to 1, 1L to 2)) {
            assertTrue(runCatching {
                com.ofairyo.gridtimer.data.verifySnapshotPipeline(2, expected) { submit ->
                    repeat(actual) { submit { true } }
                }
            }.isFailure)
        }
        assertEquals(0L, com.ofairyo.gridtimer.data.verifySnapshotPipeline(1, 0) {})
    }

    @Test fun startupBufferCannotBeReusedBeforeItsVerifierFinishes() {
        val buffers = com.ofairyo.gridtimer.data.SnapshotBufferPool(2)
        val entered = java.util.concurrent.CountDownLatch(1)
        val release = java.util.concurrent.CountDownLatch(1)
        val caller = java.util.concurrent.Executors.newSingleThreadExecutor()
        val result = caller.submit(java.util.concurrent.Callable {
            com.ofairyo.gridtimer.data.verifySnapshotPipeline(2, 3) { submit ->
                buffers.submit(1, { it[0] = 11; 1 }, submit) { bytes, size ->
                    entered.countDown()
                    release.await()
                    size == 1 && bytes[0] == 11.toByte()
                }
                check(entered.await(5, java.util.concurrent.TimeUnit.SECONDS))
                buffers.submit(1, { it[0] = 22; 1 }, submit) { bytes, size ->
                    size == 1 && bytes[0] == 22.toByte()
                }
                buffers.submit(1, { it[0] = 33; 1 }, submit) { bytes, size ->
                    release.countDown()
                    size == 1 && bytes[0] == 33.toByte()
                }
            }
        })
        try {
            assertEquals(3L, result.get(5, java.util.concurrent.TimeUnit.SECONDS).toLong())
        } finally { release.countDown(); caller.shutdownNow() }
    }

    @Test fun incompleteBufferReadRejectsReadinessAndReturnsItsLease() {
        val buffers = com.ofairyo.gridtimer.data.SnapshotBufferPool(1)
        var verified = false
        assertTrue(runCatching {
            com.ofairyo.gridtimer.data.verifySnapshotPipeline(1, 1) { submit ->
                buffers.submit(2, { 1 }, submit) { _, _ -> verified = true; true }
            }
        }.isFailure)
        assertFalse(verified)
        assertEquals(1L, com.ofairyo.gridtimer.data.verifySnapshotPipeline(1, 1) { submit ->
            buffers.submit(1, { it[0] = 44; 1 }, submit) { bytes, size ->
                size == 1 && bytes[0] == 44.toByte()
            }
        })
    }

    @Test fun nativeStartupReceiptRequiresAcceptedCurrentAndUnchangedWorkspaceTransaction() {
        val window = com.ofairyo.gridtimer.data.SnapshotVerificationWindow()
        assertFalse(window.canSkipVerifiedHistory("a", 1, true))
        window.recordNativeHistory("a", 1)
        assertTrue(window.canSkipVerifiedHistory("a", 1, true))
        assertFalse(window.canSkipVerifiedHistory("a", 1, false))
        assertFalse(window.canSkipVerifiedHistory("b", 1, true))
        assertFalse(window.canSkipVerifiedHistory("a", 2, true))
        window.record("a", true, 2)
        assertFalse(window.canSkipVerifiedHistory("a", 2, true))
        assertFalse(window.canSkipVerifiedHistory("a", 1, true))
    }

    @Test fun clockAndAlarmDeliverOnlyOnceAcrossAResume() {
        val delivery = TimerBellDispatch()
        val token = TimerBellToken("a", 1, "run", MicroBreakPhase.BREAK, 0, 1_000L, 16_000L)
        assertTrue(delivery.claim(token) { true })
        assertFalse(delivery.claim(token) { true })
        assertTrue(delivery.claim(token.copy(slotId = 2)) { true })
        assertTrue(delivery.claim(token.copy(runId = "resumed", segmentStart = 5_000L)) { true })
        assertFalse(delivery.claim(token.copy(workspace = "b")) { false })
    }

    @Test fun pauseSilencesAnUncommittedRequestWithoutWaitingForWriter() = runBlocking {
        val writer = Mutex(locked = true)
        val delivery = TimerBellDispatch()
        val token = TimerBellToken("a", 1, "run", MicroBreakPhase.BREAK, 0, 1_000L, 16_000L)
        val gate = TimerBellGate()
        var stopped = 0
        val ticket = gate.open(1) { delivery.permits(token) }!!
        gate.use(ticket) { ticket.cleanup += { stopped++ } }
        delivery.beginPause("a", listOf(1))
        val pendingWrite = launch(start = CoroutineStart.UNDISPATCHED) { writer.lock(); writer.unlock() }
        gate.reconcile()
        assertEquals(1, stopped)
        assertFalse(pendingWrite.isCompleted)
        assertFalse(delivery.claim(token) { true })
        assertTrue(delivery.claim(token.copy(slotId = 2)) { true })
        writer.unlock()
        pendingWrite.join()
        delivery.endPause("a", listOf(1))
        assertTrue(delivery.claim(token.copy(runId = "resumed")) { true })
    }

    @Test fun clockProjectsRestEndWhileStoredPhaseStillSaysBreak() {
        val stored = TimerSlot(id = 1, runningSinceEpochMillis = 1_000L, activeRunId = "run",
            microBreakPhase = MicroBreakPhase.BREAK)
        val resolved = stored.resolveMicroBreak(16_050L)
        assertEquals(MicroBreakPhase.BREAK, stored.microBreakPhase)
        assertEquals(MicroBreakPhase.FOCUS, resolved.slot.microBreakPhase)
        assertEquals(MicroBreakTransitionType.FOCUS_RESUMED, resolved.transitions.single().type)
        val next = resolved.slot.bellToken("a", 16_050L)!!
        assertTrue(next.isCurrent("a", stored.resolveMicroBreak(16_100L).slot, 16_100L))
        assertFalse(next.isCurrent("a", stored.copy(runningSinceEpochMillis = null).resolveMicroBreak(16_100L).slot, 16_100L))
        val delivery = TimerBellDispatch()
        assertTrue(delivery.claim(next) { true })
        assertFalse(delivery.claim(resolved.slot.bellToken("a", 16_100L)!!) { true })
    }

    @Test fun pausedBellCannotStartAfterLoadingOrPostAnOldNotification() {
        val gate = TimerBellGate()
        var running = true
        var played = 0
        var posted = 0
        val ticket = gate.open(1) { running }!!
        running = false
        assertFalse(gate.use(ticket) { played++; posted++ })
        assertEquals(0, played)
        assertEquals(0, posted)
        assertNull(gate.open(1) { running })
    }

    @Test fun pauseStopsPlayingBellAndNotificationOnlyForThatSlot() {
        val gate = TimerBellGate()
        var running = true
        var stopped = 0
        var canceled = 0
        val first = gate.open(1) { running }!!
        val other = gate.open(2) { true }!!
        gate.use(first) { first.cleanup += { stopped++ }; first.cleanup += { canceled++ } }
        running = false
        gate.reconcile()
        gate.reconcile()
        assertEquals(1, stopped)
        assertEquals(1, canceled)
        assertFalse(gate.use(first) { fail("Paused bell restarted") })
        assertTrue(gate.use(other) {})
    }

    @Test fun supersededBellCannotCancelOrPostOverAResumedRun() {
        val gate = TimerBellGate()
        var stopped = 0
        val old = gate.open(1) { true }!!
        gate.use(old) { old.cleanup += { stopped++ } }
        val resumed = gate.open(1) { true }!!
        gate.cancelIfOwned(old)
        assertEquals(1, stopped)
        assertFalse(gate.use(old) { fail("Old run posted") })
        assertTrue(gate.use(resumed) {})
    }

    @Test fun bellTokenRejectsPauseRunPhaseCycleWorkspaceAndDeadlineChanges() {
        val slot = TimerSlot(id = 1, runningSinceEpochMillis = 1_000L, activeRunId = "run-a",
            microBreakPhase = MicroBreakPhase.BREAK, microBreakCycleIndex = 4)
        val token = TimerBellToken("account-a", 1, "run-a", MicroBreakPhase.BREAK, 4, 1_000L, 16_000L)
        assertTrue(token.isCurrent("account-a", slot.copy(title = "Changed wording"), 2_000L))
        assertTrue(token.isCurrent("account-a", slot, 15_999L))
        assertFalse(token.isCurrent("account-a", slot.copy(runningSinceEpochMillis = null), 2_000L))
        assertFalse(token.isCurrent("account-a", slot.copy(activeRunId = "run-b"), 2_000L))
        assertFalse(token.isCurrent("account-a", slot.copy(microBreakPhase = MicroBreakPhase.FOCUS), 2_000L))
        assertFalse(token.isCurrent("account-a", slot.copy(microBreakCycleIndex = 5), 2_000L))
        assertFalse(token.isCurrent("account-b", slot, 2_000L))
        assertFalse(token.isCurrent("account-a", slot, 16_000L))
        assertFalse(token.isCurrent("account-a", slot, 999L))
        assertFalse(token.isCurrent("account-a", null, 2_000L))
    }

    @Test fun delayedRestTransitionIsRejectedAndParallelSlotsAreRetained() {
        val slots = listOf(1, 2).map { TimerSlot(id = it, runningSinceEpochMillis = 20_000L,
            activeRunId = "run-$it", microBreakPhase = MicroBreakPhase.BREAK) }
        val transitions = slots.map { MicroBreakTransition(MicroBreakTransitionType.BREAK_STARTED, it.id, "Task", 20_000L) }
        val resolved = AppDataMicroBreakResolution(AppData(slots = slots), transitions)
        assertEquals(listOf(1, 2), currentBellTransitions(resolved, 21_000L, 120_000L).map { it.slotId })
        assertTrue(currentBellTransitions(resolved, 24_000L, 120_000L).isEmpty())
        assertTrue(currentBellTransitions(resolved, 35_000L, 120_000L).isEmpty())
        val paused = resolved.copy(data = AppData(slots = slots.map { it.copy(runningSinceEpochMillis = null) }))
        assertTrue(currentBellTransitions(paused, 21_000L, 120_000L).isEmpty())
        val wrongPhase = resolved.copy(transitions = transitions.map { it.copy(type = MicroBreakTransitionType.FOCUS_RESUMED) })
        assertTrue(currentBellTransitions(wrongPhase, 21_000L, 120_000L).isEmpty())
    }

    @Test fun lateCatchUpSelectsOnlyCurrentPhasePerSlot() {
        val slot = TimerSlot(id = 1, runningSinceEpochMillis = 20_000L, activeRunId = "run",
            microBreakPhase = MicroBreakPhase.BREAK)
        val old = MicroBreakTransition(MicroBreakTransitionType.FOCUS_RESUMED, 1, "Task", 5_000L)
        val latest = MicroBreakTransition(MicroBreakTransitionType.BREAK_STARTED, 1, "Task", 20_000L)
        val result = currentBellTransitions(AppDataMicroBreakResolution(AppData(slots = listOf(slot)), listOf(latest, old)), 20_100L, 120_000L)
        assertEquals(listOf(latest), result)
    }

    @Test fun committedTransitionCompletesDespiteCollectorReplacement() = runBlocking {
        val published = CompletableDeferred<Unit>()
        val release = CompletableDeferred<Unit>()
        var delivered = 0
        val worker = launch {
            finishTimerTransition {
                published.complete(Unit)
                release.await()
                delivered++
            }
        }
        published.await()
        worker.cancel()
        release.complete(Unit)
        worker.join()
        assertEquals(1, delivered)
    }

    @Test fun structuredKnowledgeSurvivesAndroidSerializationAndKeepsEditorGuard() {
        val page = Json.parseToJsonElement("""{"version":1,"tags":["项目"],"properties":{"cost":{"kind":"number","value":0.1}}}""") as kotlinx.serialization.json.JsonObject
        val block = Json.parseToJsonElement("""{"version":1,"kind":"table","table":[["名称","值"],["甲","0"]]}""") as kotlinx.serialization.json.JsonObject
        val document = com.ofairyo.gridtimer.data.NoteDocument(knowledge = page, blocks = listOf(NoteBlock(id = "table", knowledge = block)))
        val note = NoteEntry(id = "advanced", document = document, revisions = listOf(com.ofairyo.gridtimer.data.NoteRevisionSnapshot(id = "version", label = "修改前", document = document)))
        val restored = Json.decodeFromString<NoteEntry>(Json.encodeToString(note))
        assertEquals(document, restored.document)
        assertEquals(note.revisions, restored.revisions)
        assertTrue(restored.hasStructuredKnowledge())
        assertTrue(restored.copy(document = document.copy(knowledge = null)).hasStructuredKnowledge())
        assertFalse(NoteEntry(id = "plain").hasStructuredKnowledge())
    }

    @Test fun structuredAttachmentKindsSurviveAndroidSerialization() {
        for (kind in listOf("FILE", "AUDIO", "VIDEO")) {
            val source = """{"id":"file","kind":"$kind","mimeType":"application/octet-stream","sizeBytes":12}"""
            val attachment = Json.decodeFromString<com.ofairyo.gridtimer.data.NoteAttachment>(source)
            assertEquals(kind, attachment.kind.name)
            assertEquals(attachment, Json.decodeFromString<com.ofairyo.gridtimer.data.NoteAttachment>(Json.encodeToString(attachment)))
        }
    }

    @Test fun startupVerificationDoesNotTrustAnUnreadDatabase() {
        val window = com.ofairyo.gridtimer.data.SnapshotVerificationWindow()
        assertFalse(window.covers("guest", false, 1L))
        assertFalse(window.covers("guest", true, 1L))
    }

    @Test fun startupVerificationCannotCrossAccountsOrExpandToAllAccounts() {
        val window = com.ofairyo.gridtimer.data.SnapshotVerificationWindow()
        window.record("guest", true, 1L)
        assertTrue(window.covers("guest", true, 1L))
        assertFalse(window.covers("account:other", false, 1L))
        assertFalse(window.covers(null, true, 1L))
    }

    @Test fun startupCurrentVerificationCannotAuthorizeUnreadHistory() {
        val window = com.ofairyo.gridtimer.data.SnapshotVerificationWindow()
        window.record("guest", false, 1L)
        assertTrue(window.covers("guest", false, 1L))
        assertFalse(window.covers("guest", true, 1L))
        window.record("guest", true, 1L)
        assertTrue(window.covers("guest", true, 1L))
    }

    @Test fun startupVerificationRechecksAfterAnyDatabaseMutation() {
        val window = com.ofairyo.gridtimer.data.SnapshotVerificationWindow()
        window.record(null, true, 1L)
        assertTrue(window.covers("guest", true, 1L))
        assertFalse(window.covers("guest", true, 2L))
        window.record("guest", false, 2L)
        assertTrue(window.covers("guest", false, 2L))
        assertFalse(window.covers("guest", true, 2L))
        assertFalse(window.covers("account:other", false, 2L))
    }

    @Test fun startupVerificationNewTransactionHasNoPriorReceipt() {
        val first = com.ofairyo.gridtimer.data.SnapshotVerificationWindow()
        first.record("guest", true, 1L)
        val reopened = com.ofairyo.gridtimer.data.SnapshotVerificationWindow()
        assertFalse(reopened.covers("guest", true, 1L))
    }

    @Test fun startupUnsealedHistoryCannotPrimeTheFirstSave() {
        val window = com.ofairyo.gridtimer.data.SnapshotVerificationWindow()
        window.observeEnvelope(true)
        assertTrue(window.allSnapshotsSealed)
        window.observeEnvelope(false)
        assertFalse(window.allSnapshotsSealed)
        window.observeEnvelope(true)
        assertFalse(window.allSnapshotsSealed)
    }

    private class NoteBackgroundFixture(sessionId: String? = null) {
        val scope = kotlinx.coroutines.CoroutineScope(
            kotlinx.coroutines.SupervisorJob() + kotlinx.coroutines.Dispatchers.Unconfined)
        val owner = object : androidx.lifecycle.LifecycleOwner {
            override val lifecycle: androidx.lifecycle.Lifecycle
                get() = error("The observer only consumes delivered events")
        }
        var unlockedId = sessionId
        var selectedId: String? = sessionId ?: "ordinary-note"
        val locked = mutableListOf<String>()
        val unlockRequests = NoteUnlockRequestGate()
        var timeout: kotlinx.coroutines.CancellableContinuation<Unit>? = null
        val observer = NoteBackgroundLockObserver(scope, { unlockedId }, { noteId ->
            locked += noteId
            unlockedId = null
            if (selectedId == noteId) selectedId = null
        }, {
            kotlinx.coroutines.suspendCancellableCoroutine<Unit> { timeout = it }
        }, onBackground = { unlockRequests.invalidateAll() })
        fun event(event: androidx.lifecycle.Lifecycle.Event) = observer.onStateChanged(owner, event)
        fun expire(pending: kotlinx.coroutines.CancellableContinuation<Unit>? = timeout) {
            if (pending?.isActive == true) pending.resumeWith(Result.success(Unit))
        }
        fun dispose() {
            observer.dispose()
            scope.coroutineContext[kotlinx.coroutines.Job]?.cancel()
        }
    }

    @Test fun noteBackgroundKeepsOrdinaryPageAcrossHomeAndAppSwitches() {
        val fixture = NoteBackgroundFixture()
        try {
            repeat(3) {
                fixture.event(androidx.lifecycle.Lifecycle.Event.ON_PAUSE)
                fixture.event(androidx.lifecycle.Lifecycle.Event.ON_STOP)
                fixture.expire()
                assertEquals("ordinary-note", fixture.selectedId)
                assertTrue(fixture.locked.isEmpty())
                fixture.event(androidx.lifecycle.Lifecycle.Event.ON_START)
                fixture.event(androidx.lifecycle.Lifecycle.Event.ON_RESUME)
                assertEquals("ordinary-note", fixture.selectedId)
            }
        } finally { fixture.dispose() }
    }

    @Test fun noteBackgroundStillLocksEncryptedPageAfterTimeout() {
        val fixture = NoteBackgroundFixture("protected-note")
        try {
            fixture.event(androidx.lifecycle.Lifecycle.Event.ON_STOP)
            assertEquals("protected-note", fixture.selectedId)
            fixture.expire()
            fixture.expire()
            assertEquals(listOf("protected-note"), fixture.locked)
            assertNull(fixture.unlockedId)
            assertNull(fixture.selectedId)
        } finally { fixture.dispose() }
    }

    @Test fun noteBackgroundReturnBeforeTimeoutCannotCloseForegroundPage() {
        val fixture = NoteBackgroundFixture("protected-note")
        try {
            fixture.event(androidx.lifecycle.Lifecycle.Event.ON_STOP)
            val stale = fixture.timeout
            fixture.event(androidx.lifecycle.Lifecycle.Event.ON_START)
            fixture.expire(stale)
            assertEquals("protected-note", fixture.selectedId)
            assertTrue(fixture.locked.isEmpty())
            assertFalse(stale!!.isActive)
            fixture.event(androidx.lifecycle.Lifecycle.Event.ON_STOP)
            fixture.expire()
            assertEquals(listOf("protected-note"), fixture.locked)
        } finally { fixture.dispose() }
    }

    @Test fun noteBackgroundOldSessionCannotLockAnotherDocument() {
        val fixture = NoteBackgroundFixture("old-session")
        try {
            fixture.event(androidx.lifecycle.Lifecycle.Event.ON_STOP)
            fixture.unlockedId = "new-session"
            fixture.selectedId = "new-session"
            fixture.expire()
            assertEquals("new-session", fixture.selectedId)
            assertEquals("new-session", fixture.unlockedId)
            assertTrue(fixture.locked.isEmpty())
        } finally { fixture.dispose() }
    }

    @Test fun noteBackgroundDisposalCancelsPendingNavigationChange() {
        for (destroyed in listOf(false, true)) {
            val fixture = NoteBackgroundFixture("protected-note")
            try {
                fixture.event(androidx.lifecycle.Lifecycle.Event.ON_STOP)
                val stale = fixture.timeout
                if (destroyed) fixture.event(androidx.lifecycle.Lifecycle.Event.ON_DESTROY)
                else fixture.observer.dispose()
                fixture.expire(stale)
                assertFalse(stale!!.isActive)
                assertEquals("protected-note", fixture.selectedId)
                assertTrue(fixture.locked.isEmpty())
            } finally { fixture.dispose() }
        }
    }

    @Test fun noteBackgroundRepeatedStopsReplaceTheOldTimer() {
        val fixture = NoteBackgroundFixture("protected-note")
        try {
            fixture.event(androidx.lifecycle.Lifecycle.Event.ON_STOP)
            val first = fixture.timeout
            fixture.event(androidx.lifecycle.Lifecycle.Event.ON_STOP)
            fixture.expire(first)
            assertEquals("protected-note", fixture.selectedId)
            fixture.expire()
            assertEquals(listOf("protected-note"), fixture.locked)
        } finally { fixture.dispose() }
    }

    @Test fun alarmCompletionReleasesOnceAndDoesNotRetryFinishedWork() {
        val releases = java.util.concurrent.atomic.AtomicInteger()
        val retries = java.util.concurrent.atomic.AtomicInteger()
        val completion = AlarmDeliveryCompletion { releases.incrementAndGet() }
        completion.complete()
        completion.complete { retries.incrementAndGet() }
        assertEquals(1, releases.get())
        assertEquals(0, retries.get())
    }

    @Test fun alarmDeadlineReleasesEvenIfRetryFailsAndWorkFinishesLater() {
        var releases = 0
        val completion = AlarmDeliveryCompletion { releases++ }
        try {
            completion.complete { error("Retry scheduling failed") }
            fail("Expected retry failure")
        } catch (_: IllegalStateException) { }
        completion.complete()
        assertEquals(1, releases)
    }

    @Test fun alarmDeadlineAndWorkerCannotFinishTheSameReceiptTwice() {
        val releases = java.util.concurrent.atomic.AtomicInteger()
        val completion = AlarmDeliveryCompletion { releases.incrementAndGet() }
        val start = java.util.concurrent.CountDownLatch(1)
        val workers = List(16) { Thread { start.await(); completion.complete() } }
        workers.forEach(Thread::start)
        start.countDown()
        workers.forEach(Thread::join)
        assertEquals(1, releases.get())
    }

    private fun timerFixture(running: Boolean = false): AppData = AppData.default().let { data ->
        data.copy(
            slots = data.slots.map { if (it.id == 2 && running) it.copy(
                runningSinceEpochMillis = 100, activeRunId = "run-current") else it },
            sessions = List(10_000) { index -> TimerSession(id = "history-$index", slotId = 1,
                slotTitle = "保留标题", startedAtEpochMillis = index.toLong(),
                endedAtEpochMillis = index + 10L, durationMillis = 10, updatedAtEpochMillis = index + 10L) },
            notes = listOf(NoteEntry(id = "other-note", content = "保留正文")),
            tombstones = listOf(DataTombstone(entityType = TOMBSTONE_ENTITY_SESSION,
                entityId = "deleted-history", deletedAtEpochMillis = 50))
        )
    }

    @Test fun timerNotificationRequiresTheAccountThatCreatedItsAction() {
        fun accepted(expected: String?, current: String) =
            com.ofairyo.gridtimer.notifications.currentTimerNotificationWorkspace(expected, current)
        assertEquals("account-a", accepted("account-a", "account-a"))
        assertNull(accepted("account-a", "account-b"))
        assertNull(accepted(null, "account-b"))
        assertNull(accepted("", "account-b"))
        assertNull(accepted(" ", " "))
    }

    @Test fun timerStartDoesNotEncodeHistoryOrRebuildItsIndex() {
        val original = timerFixture()
        val scope = original.timerActionProjection()
        assertTrue(scope.sessions.isEmpty())
        assertTrue(scope.tombstones.isEmpty())
        assertTrue(Json.encodeToString(scope).length < 20_000)
        val started = scope.copy(slots = scope.slots.map { if (it.id == 2) it.copy(
            runningSinceEpochMillis = 100, activeRunId = "run-current") else it })
        val merged = original.mergeTimerAction(scope, started)
        assertNotNull(merged.slots.first { it.id == 2 }.runningSinceEpochMillis)
        assertSame(original.sessions, merged.sessions)
        assertSame(original.notes, merged.notes)
        assertSame(original.tombstones, merged.tombstones)
        assertEquals(original, merged.copy(slots = original.slots))
    }

    @Test fun timerProjectionKeepsActiveRunHistoryAndDeletionFloors() {
        val active = TimerSession(id = "run-current-focus-0", slotId = 2)
        val aggregate = active.copy(id = "run-current-focus-aggregate-100-200")
        val deleted = DataTombstone(entityType = TOMBSTONE_ENTITY_SESSION,
            entityId = active.id, deletedAtEpochMillis = Long.MAX_VALUE)
        val original = timerFixture(true).let { it.copy(sessions = listOf(active, aggregate) + it.sessions,
            tombstones = listOf(deleted) + it.tombstones) }
        val scope = original.timerActionProjection()
        assertEquals(listOf(active, aggregate), scope.sessions)
        assertEquals(listOf(deleted), scope.tombstones)
        assertSame(original.slots, scope.slots)
    }

    @Test fun timerPauseMergesOneSessionWithoutDroppingOtherHistory() {
        val original = timerFixture(true)
        val scope = original.timerActionProjection()
        val session = TimerSession(id = "run-current-focus-0", slotId = 2,
            durationMillis = 20, startedAtEpochMillis = 100, endedAtEpochMillis = 120,
            updatedAtEpochMillis = 20_000)
        val result = scope.copy(sessions = listOf(session), slots = scope.slots.map {
            if (it.id == 2) it.copy(runningSinceEpochMillis = null, activeRunId = "", accumulatedMillis = 20) else it })
        val merged = original.mergeTimerAction(scope, result)
        assertEquals(10_001, merged.sessions.size)
        assertEquals(session, merged.sessions.first())
        assertEquals(original.sessions.toSet(), merged.sessions.drop(1).toSet())
        assertTrue(merged.sessions.drop(1).all { saved -> original.sessions.any { it === saved } })
        assertNull(merged.slots.first { it.id == 2 }.runningSinceEpochMillis)
        assertEquals(original, merged.copy(slots = original.slots, sessions = original.sessions))
    }

    @Test fun timerReplayReplacesSameSessionAndHonorsRemovedScopedRecord() {
        val existing = TimerSession(id = "run-current-focus-0", slotId = 2, updatedAtEpochMillis = 300)
        val original = timerFixture(true).let { it.copy(sessions = listOf(existing) + it.sessions) }
        val scope = original.timerActionProjection()
        val updated = existing.copy(durationMillis = 50, updatedAtEpochMillis = 400)
        val replaced = original.mergeTimerAction(scope, scope.copy(sessions = listOf(updated)))
        assertEquals(original.sessions.size, replaced.sessions.size)
        assertEquals(updated, replaced.sessions.single { it.id == existing.id })
        val removed = original.mergeTimerAction(scope, scope.copy(sessions = emptyList()))
        assertFalse(removed.sessions.any { it.id == existing.id })
        assertEquals(original.sessions.drop(1).toSet(), removed.sessions.toSet())
    }

    @Test fun timerMergeRejectsUnrelatedSessionAndStaleState() {
        val original = timerFixture(true)
        val scope = original.timerActionProjection()
        assertThrows(IllegalStateException::class.java) {
            original.mergeTimerAction(scope, scope.copy(sessions = listOf(TimerSession(id = "other-run-focus-0"))))
        }
        val switched = original.copy(slots = original.slots.map { it.copy(title = "另一个工作区") })
        assertThrows(IllegalStateException::class.java) { switched.mergeTimerAction(scope, scope) }
        assertSame(original, original.mergeTimerAction(scope, scope))
    }

    @Test fun timerCopyChangesDoNotAffectDomainSafety() {
        for (title in listOf("开始", "继续计时", "")) {
            val original = timerFixture().let { data -> data.copy(slots = data.slots.map { it.copy(title = title) }) }
            val scope = original.timerActionProjection()
            assertTrue(scope.sessions.isEmpty())
            assertSame(original, original.mergeTimerAction(scope, scope))
        }
    }

    private fun privacyRepository(): TimerRepository {
        // Allocate without starting Android services or touching a database.
        // Tests invoke the actual compiled methods with synthetic snapshots.
        val unsafeClass = Class.forName("sun.misc.Unsafe")
        val unsafeField = unsafeClass.getDeclaredField("theUnsafe")
        unsafeField.isAccessible = true
        val unsafe = unsafeField.get(null)
        val repo = unsafeClass.getMethod("allocateInstance", Class::class.java)
            .invoke(unsafe, TimerRepository::class.java) as TimerRepository
        TimerRepository::class.java.getDeclaredField("startupDecodeScope").apply {
            isAccessible = true
            set(repo, com.ofairyo.gridtimer.data.StartupSnapshotDecodeScope<AppData>())
        }
        for (name in listOf("json", "strictPersistedJson")) {
            val field = TimerRepository::class.java.getDeclaredField(name)
            field.isAccessible = true
            field.set(repo, Json {
                ignoreUnknownKeys = name == "json"
                encodeDefaults = name != "json"
                coerceInputValues = name == "json"
            })
        }
        return repo
    }

    private fun privacyRewrite(raw: String, note: NoteEntry = protectedFixture()): SnapshotPrivacyRewrite? {
        val method = TimerRepository::class.java.getDeclaredMethod(
            "rewriteSnapshotForEncryptedNote", String::class.java, NoteEntry::class.java)
        method.isAccessible = true
        return try {
            method.invoke(privacyRepository(), raw, note) as SnapshotPrivacyRewrite?
        } catch (failure: java.lang.reflect.InvocationTargetException) {
            throw failure.cause!!
        }
    }

    private fun protectedFixture() = NoteEntry(id = "privacy-fixture", protectionStateRevision = 1,
        encryption = NoteEncryptionEnvelope(keyId = "fixture-key", protectionRevision = 1))

    @Test fun delayedPrivacyCleanupPreservesLaterPlaintextGenerations() {
        for (revision in listOf(2L, 4L, Long.MAX_VALUE)) {
            val note = NoteEntry(id = "privacy-fixture", title = "新标题", content = "取消加密后保存的新内容",
                protectionStateRevision = revision, updatedAtEpochMillis = 1)
            val raw = Json.encodeToString(AppData(notes = listOf(note)))
            val rewrite = privacyRewrite(raw, protectedFixture().copy(updatedAtEpochMillis = Long.MAX_VALUE))!!
            assertFalse(rewrite.changed)
            assertEquals(raw, rewrite.appDataJson)
        }
    }

    @Test fun olderPlaintextIsRedactedWithoutChangingAnotherNote() {
        val other = NoteEntry(id = "unrelated-note", title = "保留", content = "另一条笔记")
        val plain = NoteEntry(id = "privacy-fixture", title = "旧标题", content = "旧明文")
        val original = AppData(notes = listOf(plain, other))
        val result = privacyRewrite(Json.encodeToString(original))!!
        val data = Json.decodeFromString<AppData>(result.appDataJson)
        assertTrue(result.changed)
        assertEquals(other, data.notes.first { it.id == other.id })
        assertEquals(original, data.copy(notes = original.notes))
        val sealed = data.notes.first { it.id == plain.id }
        assertTrue(sealed.title.isEmpty() && sealed.content.isEmpty())
        assertNotNull(sealed.encryption)
        assertEquals(1L, sealed.protectionStateRevision)
    }

    @Test fun alreadyEncryptedAndMissingNotesRemainByteExact() {
        for (data in listOf(AppData(notes = listOf(protectedFixture())), AppData())) {
            val raw = Json.encodeToString(data)
            assertEquals(raw, privacyRewrite(raw)!!.appDataJson)
            assertFalse(privacyRewrite(raw)!!.changed)
        }
    }

    @Test fun laterDeletedPlaintextCannotBeResurrectedByOldCleanup() {
        val note = NoteEntry(id = "privacy-fixture", content = "已删除的新一代内容",
            protectionStateRevision = 2, deletedAtEpochMillis = 123)
        val raw = Json.encodeToString(AppData(notes = listOf(note)))
        assertEquals(raw, privacyRewrite(raw)!!.appDataJson)
    }

    @Test fun ambiguousOrInvalidProtectionGenerationsRefuseDestructiveCleanup() {
        for (revision in listOf(1L, -1L)) {
            val raw = Json.encodeToString(AppData(notes = listOf(NoteEntry(id = "privacy-fixture",
                content = "不能丢失", protectionStateRevision = revision))))
            val error = runCatching { privacyRewrite(raw) }.exceptionOrNull()
            assertTrue(error is IllegalStateException)
        }
    }

    @Test fun futureSnapshotIsRefusedBeforeUnknownFieldsCanBeDropped() {
        for (notes in listOf("[]", """[{"id":"privacy-fixture","content":"未来内容"}]""")) {
            val raw = """{"schemaVersion":16,"futureRequired":"必须保留","notes":$notes}"""
            assertTrue(runCatching { privacyRewrite(raw) }.exceptionOrNull() is IllegalStateException)
        }
    }

    @Test fun declaredFutureDatabaseSchemaAlsoRefusesCleanup() {
        requirePrivacySnapshotSchema("""{"schemaVersion":15}""", 15)
        assertTrue(runCatching { requirePrivacySnapshotSchema("""{"schemaVersion":15}""", 16) }
            .exceptionOrNull() is IllegalStateException)
        assertTrue(runCatching { requirePrivacySnapshotSchema("malformed", 16) }
            .exceptionOrNull() is IllegalStateException)
    }

    @Test fun privacyCleanupWaitsForTheNormalSnapshotWriter() = runBlocking {
        val repo = privacyRepository()
        val mutex = Mutex(locked = true)
        TimerRepository::class.java.getDeclaredField("writeMutex").apply {
            isAccessible = true
            set(repo, mutex)
        }
        val method = TimerRepository::class.java.declaredMethods.single {
            it.name == "redactEncryptedNoteRecoveryCopies"
        }.apply { isAccessible = true }
        val task = async(start = CoroutineStart.UNDISPATCHED) {
            runCatching {
                suspendCoroutineUninterceptedOrReturn<Any?> { continuation ->
                    method.invoke(repo, "guest", NoteEntry(), null, continuation)
                }
            }
        }
        // An invalid note fails inside the critical operation. It cannot reach
        // that validation while another normal snapshot writer owns the mutex.
        assertTrue(task.isActive)
        assertFalse(task.isCompleted)
        assertNull(withTimeoutOrNull(1_000L) { task.await(); true })
        mutex.unlock()
        val failure = task.await().exceptionOrNull()
        assertTrue(failure is IllegalStateException)
    }

    @Test fun projectedMediaMatchesFullDecodeAcrossAllRetainedVersions() {
        val raw = """{"schemaVersion":15,"notes":[{"id":"a","content":"正文","attachments":[{"id":"a1","fileName":"current.png"}],"revisions":[{"attachments":[{"id":"a2","fileName":"revision.png"}]}],"versions":[{"attachments":[{"id":"a3","fileName":"version.png"}]}]},{"id":"b","deletedAtEpochMillis":1,"attachments":[{"id":"b1","fileName":"recycled.png"}]}]}"""
        val full = Json { ignoreUnknownKeys = true }.decodeFromString<AppData>(raw)
        val expected = full.notes.flatMap { note -> note.attachments + note.revisions.flatMap { it.attachments } + note.versions.flatMap { it.attachments } }
        assertEquals(expected.map { it.fileName }, snapshotMediaReferences(raw).map { it.fileName })
        assertEquals(4, expected.size)
    }

    @Test fun unrelatedAttachmentNamesCannotMasqueradeAsMediaReferences() {
        val raw = """{"schemaVersion":15,"sessions":[{"attachments":[{"fileName":"unrelated.png"}]}],"notes":[{"content":"attachments","document":{"blocks":[{"attachments":[{"fileName":"not_metadata.png"}]}]},"attachments":[{"id":"real","fileName":"real.png"}]}]}"""
        assertEquals(listOf("real.png"), snapshotMediaReferences(raw).map { it.fileName })
    }

    @Test fun malformedMediaReferencesAbortPruningInsteadOfBecomingEmpty() {
        for (raw in listOf("{", """{"notes":null}""", """{"notes":[null]}""", """{"notes":[{"attachments":null}]}""", """{"notes":[{"versions":[{"attachments":[7]}]}]}""")) {
            assertTrue("Unexpected empty fallback for $raw", runCatching { snapshotMediaReferences(raw) }.isFailure)
        }
        assertTrue(snapshotMediaReferences("{}").isEmpty())
    }

    @Test fun rawAndVerifiedDeclaredFutureSchemasBothBlockOlderStartup() {
        assertNull(snapshotFutureSchema("""{"schemaVersion":15,"notes":[]}""", 15))
        assertEquals(16, snapshotFutureSchema("""{"schemaVersion":16,"notes":[]}""", 15))
        assertEquals(17, snapshotFutureSchema("""{"schemaVersion":15,"notes":[]}""", 17))
        assertEquals(Int.MAX_VALUE, snapshotFutureSchema("""{"schemaVersion":9223372036854775807}"""))
        assertEquals(18, snapshotFutureSchema("malformed", 18))
    }

    @Test fun schemaProjectionUsesTopLevelVersionOnly() {
        val raw = """{"notes":[{"schemaVersion":999,"content":"schemaVersion:999"}],"schemaVersion":15}"""
        assertNull(snapshotFutureSchema(raw))
        assertNull(snapshotFutureSchema("{}"))
    }

    @Test fun oldAcknowledgementsCannotReplaceNewTyping() {
        val draft = TimerFieldDraft("")
        val first = draft.edit(TextFieldValue("9", TextRange(1)))!!
        val last = draft.edit(TextFieldValue("90", TextRange(1)))!!
        draft.accept(TimerFieldReceipt("9", 1))
        draft.acknowledge(first, TimerFieldReceipt("9", 1))
        assertEquals("90", draft.value.text)
        draft.acknowledge(last, TimerFieldReceipt("90", 2))
        assertEquals(TextRange(1), draft.value.selection)
        draft.acknowledge(first, TimerFieldReceipt("9", 1))
        assertEquals("90", draft.value.text)
    }

    @Test fun unchangedRepositoryValueStillAcknowledgesClearingTheDraft() {
        val draft = TimerFieldDraft("")
        val first = draft.edit(TextFieldValue("9"))!!
        val cleared = draft.edit(TextFieldValue(""))!!
        draft.acknowledge(first, TimerFieldReceipt("9", 1))
        assertEquals("", draft.value.text)
        draft.acknowledge(cleared, TimerFieldReceipt("", 2))
        draft.accept(TimerFieldReceipt("来自同步", 3))
        assertEquals("来自同步", draft.value.text)
    }

    @Test fun composingChineseIsLocalUntilCommitted() {
        val draft = TimerFieldDraft("")
        val composing = TextFieldValue("测试", TextRange(2), TextRange(0, 2))
        assertNull(draft.edit(composing))
        draft.accept(TimerFieldReceipt("旧内容", 0))
        assertEquals(composing, draft.value)
        val committed = draft.commit()!!
        assertEquals("测试", committed.text)
        assertNull(draft.value.composition)
        draft.acknowledge(committed, TimerFieldReceipt("测试", 1))
        assertEquals(TextRange(2), draft.value.selection)
    }

    @Test fun priorSaveDoesNotInterruptANewComposition() {
        val draft = TimerFieldDraft("")
        val old = draft.edit(TextFieldValue("a"))!!
        val composing = TextFieldValue("a中文", TextRange(3), TextRange(1, 3))
        assertNull(draft.edit(composing))
        draft.acknowledge(old, TimerFieldReceipt("a", 1))
        assertEquals(composing, draft.value)
        val last = draft.edit(composing.copy(composition = null))!!
        draft.acknowledge(last, TimerFieldReceipt("a中文", 2))
        assertEquals("a中文", draft.value.text)
    }

    @Test fun selectingTextDoesNotTriggerAnotherWrite() {
        val draft = TimerFieldDraft("中文909")
        assertNull(draft.edit(TextFieldValue("中文909", TextRange(1, 4))))
        assertNull(draft.commit())
        assertEquals(TextRange(1, 4), draft.value.selection)
    }

    @Test fun rejectedSaveKeepsDraftAndCanBeRetried() {
        val draft = TimerFieldDraft("旧")
        val failed = draft.edit(TextFieldValue("新😀"))!!
        draft.acknowledge(failed, null)
        assertEquals("新😀", draft.value.text)
        assertTrue(draft.saveFailed)
        draft.accept(TimerFieldReceipt("旧", 0))
        assertEquals("新😀", draft.value.text)
        val retry = draft.retry()!!
        draft.acknowledge(retry, TimerFieldReceipt("新😀", 1))
        assertFalse(draft.saveFailed)
        assertNull(draft.retry())
    }

    @Test fun normalizationKeepsTheCaretInsideUnicodeText() {
        val draft = TimerFieldDraft("")
        val submission = draft.edit(TextFieldValue("  中文😀", TextRange(6)))!!
        assertEquals("中文😀", submission.text)
        draft.acknowledge(submission, TimerFieldReceipt("中文😀", 1))
        assertEquals(TextRange(4), draft.value.selection)
        assertEquals("中文😀", draft.value.text)
    }

    @Test fun sevenDigitsSurviveEveryDelayedIntermediateSave() {
        val draft = TimerFieldDraft("")
        val submissions = "9091203".indices.map { index ->
            draft.edit(TextFieldValue("9091203".take(index + 1), TextRange(index + 1)))!!
        }
        submissions.dropLast(1).forEach {
            draft.accept(TimerFieldReceipt(it.text, it.generation))
            draft.acknowledge(it, TimerFieldReceipt(it.text, it.generation))
            assertEquals("9091203", draft.value.text)
        }
        draft.acknowledge(submissions.last(), TimerFieldReceipt("9091203", 7))
        assertEquals("9091203", draft.value.text)
    }

    @Test fun aDelayedFlowEmissionAfterTheLatestReceiptCannotRollBackText() {
        val draft = TimerFieldDraft("", 100)
        val first = draft.edit(TextFieldValue("9"))!!
        val latest = draft.edit(TextFieldValue("9091203", TextRange(3)))!!
        draft.acknowledge(latest, TimerFieldReceipt("9091203", 102))
        draft.accept(TimerFieldReceipt("9", 101))
        draft.acknowledge(first, TimerFieldReceipt("9", 101))
        assertEquals("9091203", draft.value.text)
        assertEquals(TextRange(3), draft.value.selection)
        draft.accept(TimerFieldReceipt("同步新标题", 103))
        assertEquals("同步新标题", draft.value.text)
    }

    @Test fun anExplicitWriteRejectionIsNotAcknowledgedAsAnEmptySuccess() {
        val draft = TimerFieldDraft("旧标题", 10)
        val clear = draft.edit(TextFieldValue(""))!!
        draft.acknowledge(clear, null)
        assertTrue(draft.saveFailed)
        assertEquals("", draft.value.text)
        val retry = draft.retry()!!
        draft.acknowledge(retry, TimerFieldReceipt("", 11))
        assertFalse(draft.saveFailed)
    }

    @Test fun anUnexpectedStoredValuePreservesTheUsersDraft() {
        val draft = TimerFieldDraft("旧", 10)
        val request = draft.edit(TextFieldValue("保留中文😀"))!!
        draft.acknowledge(request, TimerFieldReceipt("另一个工作区", 20))
        assertTrue(draft.saveFailed)
        draft.accept(TimerFieldReceipt("另一个工作区", 20))
        assertEquals("保留中文😀", draft.value.text)
    }

    @Test fun anrTraceKeepsReadableTextAndClosesTheStream() {
        var closed = false
        val source = "main 线程\n等待消息😀"
        val input = object : java.io.ByteArrayInputStream(source.toByteArray(Charsets.UTF_8)) {
            override fun close() { closed = true; super.close() }
        }
        val result = collectAnrTrace { input }
        assertTrue(closed)
        assertTrue(result.contains("trace_status=captured"))
        assertTrue(result.contains("trace_truncated=false"))
        assertTrue(result.endsWith(source))
    }

    @Test fun anrTraceRetainsABoundedPrefixWhileScanningForMain() {
        val input = java.io.ByteArrayInputStream(ByteArray(512 * 1024) { 65 })
        val result = collectAnrTrace { input }
        assertTrue(result.contains("trace_truncated=true"))
        assertTrue(result.contains("trace_bytes=131072"))
        assertEquals(0, input.available())
        assertTrue(result.contains("trace_main_found=false"))
        assertTrue(result.length < 132000)
    }

    @Test fun missingOrUnreadableAnrTraceDoesNotDiscardExitMetadata() {
        assertEquals("trace_status=not_available", collectAnrTrace { null })
        assertTrue(collectAnrTrace { throw SecurityException("denied") }.contains("read_failed"))
        var closed = false
        val input = object : java.io.InputStream() {
            override fun read(): Int = throw java.io.IOException("unavailable")
            override fun close() { closed = true }
        }
        assertTrue(collectAnrTrace { input }.contains("IOException"))
        assertTrue(closed)
    }

    @Test fun anrMainAfterTheOldLimitIsCapturedWithItsStack() {
        val prefix = "background stack line\n".repeat(8000)
        val main = "\"main\" prio=5 tid=19 Blocked\n  at test.Save.await(测试.kt:9)\n\n"
        val result = collectAnrTrace { (prefix + main + "\"other\" daemon tid=2\n").byteInputStream() }
        assertTrue(result.contains("trace_main_found=true"))
        assertTrue(result.contains("trace_main_section\n$main"))
        assertFalse(result.substringAfter("trace_main_section").contains("\"other\""))
        assertTrue(result.length < 132000)
    }

    @Test fun malformedUnendingTraceHasABoundedReadAndAllocation() {
        val input = java.io.ByteArrayInputStream(ByteArray(5 * 1024 * 1024) { 65 })
        val result = collectAnrTrace { input }
        assertEquals(1024 * 1024 - 1, input.available())
        assertTrue(result.contains("trace_scan_truncated=true"))
        assertTrue(result.contains("trace_main_found=false"))
        assertTrue(result.length < 132000)
    }

    @Test fun staleOverLimitCallbacksKeepTheAcceptedLastCharacter() {
        val earlierFrame = TextFieldValue("12345678901234567890123", TextRange(23))
        val next = TextFieldValue("1234567890123456789012345", TextRange(25))
        assertEquals("123456789012345678901234", boundedFieldEdit(earlierFrame, next, 24).text)
        assertEquals(TextRange(24), boundedFieldEdit(earlierFrame, next, 24).selection)
    }

    @Test fun excessInsertionDoesNotDestroyTheExistingSuffix() {
        val base = TextFieldValue("abcdefghijklmnopqrstuvwx", TextRange(4))
        val inserted = TextFieldValue("abcd12345efghijklmnopqrstuvwx", TextRange(9))
        val result = boundedFieldEdit(base, inserted, 24)
        assertEquals(base.text, result.text)
        assertEquals(TextRange(4), result.selection)
        val replacing = TextFieldValue("abcd12345678klmnopqrstuvwx", TextRange(12))
        assertEquals("abcd123456klmnopqrstuvwx", boundedFieldEdit(base, replacing, 24).text)
    }

    @Test fun truncationNeverSplitsAnEmojiSurrogatePair() {
        val result = boundedFieldEdit(TextFieldValue("abc"), TextFieldValue("abc😀", TextRange(5)), 4)
        assertEquals("abc", result.text)
        assertEquals(TextRange(3), result.selection)
        assertEquals("abc😀", boundedFieldEdit(TextFieldValue("abc"), TextFieldValue("abc😀"), 5).text)
    }

    @Test fun chineseCompositionCanGrowThenCommitIntoTheRemainingSpace() {
        val draft = TimerFieldDraft("12345678901234567890ABCD", maxLength = 24)
        val composing = TextFieldValue("12345678901234567890zhongwenCD", TextRange(28), TextRange(20, 28))
        assertNull(draft.edit(composing))
        assertEquals(composing, draft.value)
        val committed = draft.edit(TextFieldValue("12345678901234567890中文CD", TextRange(22)))!!
        assertEquals("12345678901234567890中文CD", committed.text)
        assertEquals(TextRange(22), draft.value.selection)
    }

    @Test fun leavingAComposingFieldAlsoHonoursTheLengthLimit() {
        val draft = TimerFieldDraft("abcdefghijklmnopqrstuv", maxLength = 24)
        assertNull(draft.edit(TextFieldValue("abcdefghijklmnopqrstuv中文测试", TextRange(26), TextRange(22, 26))))
        assertEquals("abcdefghijklmnopqrstuv中文", draft.commit()!!.text)
        assertNull(draft.value.composition)
        assertEquals(TextRange(24), draft.value.selection)
    }

    @Test fun legacyOverlongTextCanBeEditedWithoutLosingUnchangedContent() {
        val legacy = "a".repeat(30)
        assertEquals(legacy.dropLast(1), boundedFieldEdit(TextFieldValue(legacy), TextFieldValue(legacy.dropLast(1)), 24).text)
    }

    private fun initial(pinned: Boolean, title: String = "页面") = NoteDraftState(
        title = title, accentSeed = "amber", pinned = pinned, folderId = null,
        markdownEnabled = true,
        blocks = listOf(NoteBlock(id = "text", type = NoteBlockType.TEXT, text = "正文"))
    )

    @Test fun aCleanNewVisitRefreshesRestoredPinMetadata() {
        val model = DocumentEditorSessionViewModel()
        val old = model.session("note", "workspace", "visit-1", initial(true), false)
        val restored = model.session("note", "workspace", "visit-2", initial(false), false)
        assertNotSame(old, restored)
        assertFalse(restored.pinned.value)
    }

    @Test fun configurationChangesKeepTheEditingSession() {
        val model = DocumentEditorSessionViewModel()
        val old = model.session("note", "workspace", "same-visit", initial(true), false)
        old.titleText.value = "未提交中文"
        val rotated = model.session("note", "workspace", "same-visit", initial(false), false)
        assertSame(old, rotated)
        assertEquals("未提交中文", rotated.titleText.value)
    }

    @Test fun anUnsavedVisitRemainsRecoverable() {
        for (state in listOf(DocumentEditorSaveState.SAVING, DocumentEditorSaveState.FAILED)) {
            val model = DocumentEditorSessionViewModel()
            val old = model.session("note", "workspace", "visit-1", initial(true), false)
            old.titleText.value = "保留草稿"
            old.saveState.value = state
            val reopened = model.session("note", "workspace", "visit-2", initial(false), false)
            assertSame(old, reopened)
            assertEquals("保留草稿", reopened.titleText.value)
        }
    }

    @Test fun equalNoteIdsInDifferentAccountsHaveDifferentSessions() {
        val model = DocumentEditorSessionViewModel()
        val first = model.session("note", "one", "visit", initial(true), false)
        first.saveState.value = DocumentEditorSaveState.FAILED
        val second = model.session("note", "two", "visit", initial(false), false)
        assertNotSame(first, second)
        assertFalse(second.pinned.value)
    }

    @Test fun anOldExitCallbackCannotClearTheNewSession() {
        val model = DocumentEditorSessionViewModel()
        val old = model.session("note", "workspace", "one", initial(true), false)
        val current = model.session("note", "workspace", "two", initial(false), false)
        model.clear("note", "workspace", old)
        assertSame(current, model.session("note", "workspace", "two", initial(true), false))
    }

    @Test fun anAcknowledgedLocalSaveAllowsFreshMetadataOnReentry() {
        val model = DocumentEditorSessionViewModel()
        val old = model.session("note", "workspace", "one", initial(true), false)
        old.saveState.value = DocumentEditorSaveState.LOCAL_SAVED
        val current = model.session("note", "workspace", "two", initial(false, "恢复标题"), false)
        assertEquals("恢复标题", current.titleText.value)
        assertFalse(current.pinned.value)
    }

    @Test fun externalImageDeletionKeepsUnsavedTextSelectionAndComposition() {
        val image = NoteBlock(id = "image", type = NoteBlockType.IMAGE, attachmentId = "removed")
        val pending = image.copy(id = "pending", attachmentId = "pending:operation")
        val start = initial(false).copy(blocks = initial(false).blocks + image + pending)
        val session = DocumentEditorSession(start, false)
        session.acceptAttachmentIds(setOf("removed"))
        val typed = TextFieldValue("未提交中文", TextRange(3), TextRange(0, 3))
        session.textFieldStates["text"] = typed
        session.titleText.value = "本地标题"
        session.undoStack.add(start)
        session.redoStack.add(start)
        session.acceptAttachmentIds(emptySet())
        assertEquals(listOf("text", "pending"), session.blocks.value.map { it.id })
        assertEquals(typed, session.textFieldStates["text"])
        assertEquals("本地标题", session.titleText.value)
        assertTrue((session.undoStack + session.redoStack).all { draft -> draft.blocks.none { it.attachmentId == "removed" } })
        assertEquals(1L, session.saveRequestGeneration.value)
    }

    @Test fun firstAttachmentObservationDoesNotRemoveUnknownLegacyBlocks() {
        val image = NoteBlock(id = "image", type = NoteBlockType.IMAGE, attachmentId = "legacy")
        val session = DocumentEditorSession(initial(false).copy(blocks = listOf(image)), false)
        session.acceptAttachmentIds(emptySet())
        assertEquals(listOf(image), session.blocks.value)
    }

    @Test fun deletingTheLastImageLeavesAnEditableEmptyCanvas() {
        val image = NoteBlock(id = "image", type = NoteBlockType.IMAGE, attachmentId = "removed")
        val session = DocumentEditorSession(initial(false).copy(blocks = listOf(image)), false)
        session.acceptAttachmentIds(setOf("removed"))
        session.acceptAttachmentIds(emptySet())
        assertEquals(1, session.blocks.value.size)
        assertEquals(NoteBlockType.TEXT, session.blocks.value.single().type)
        assertEquals("", session.blocks.value.single().text)
    }
}
"####;
