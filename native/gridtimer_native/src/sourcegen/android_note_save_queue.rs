// v2.23.2.27 - Complete retained document exit saves after editor disposal.
// v2.23.2.1 - Coalesce adjacent draft autosaves without crossing durable barriers.
// Android implementation and JVM state tests are authored in this Rust generator.

const VIEW_MODEL: &str = "com/ofairyo/gridtimer/ui/TimerViewModel.kt";
const EDITOR: &str = "com/ofairyo/gridtimer/ui/SmartisanNoteUi.kt";
const DOCUMENT: &str = "com/ofairyo/gridtimer/ui/NoteDocumentEditor.kt";
const REPOSITORY: &str = "com/ofairyo/gridtimer/data/TimerRepository.kt";

pub fn render(path: &str, base: &str) -> Result<String, String> {
    let mut source = base.to_owned();
    match path {
        VIEW_MODEL => {
            let start = source
                .find("internal class NoteMutationDrainController(")
                .ok_or("missing note mutation controller")?;
            let end = source[start..]
                .find("class TimerViewModel(application:")
                .map(|offset| start + offset)
                .ok_or("missing note controller end")?;
            source.replace_range(start..end, CONTROLLER);
            replace(&mut source,
                "    private val noteMutationController = NoteMutationDrainController(noteMutationScope) { throwable ->",
                "    private val noteMutationController = NoteMutationDrainController(\n        noteMutationScope,\n        reportSlowOperation = { phase, millis ->\n            Log.w(\"NoteSaveLatency\", \"phase=$phase durationMs=$millis\")\n        }\n    ) { throwable ->")?;
            replace(&mut source,
                "    fun upsertNoteAndFlush(\n",
                "    fun noteMutationEpoch(): Long = noteMutationController.mutationEpoch()\n\n    fun upsertNoteAndFlush(\n")?;
            replace(
                &mut source,
                "    fun upsertNoteAndFlush(\n",
                &format!("{AUTOSAVE}{CAPTURED_SAVE}    fun upsertNoteAndFlush(\n"),
            )?;
        }
        EDITOR => {
            let start = source
                .find("    val autoSaveDelayMillis = if (richTextEnabled)")
                .ok_or("missing note autosave effect")?;
            let end = source[start..]
                .find("    fun persistLatestDraft(")
                .map(|offset| start + offset)
                .ok_or("missing note autosave effect end")?;
            let effect = source[start..end].to_owned();
            let mut updated = effect.clone();
            replace(
                &mut updated,
                "            viewModel.upsertNote(\n",
                "            viewModel.upsertNoteAutosave(\n",
            )?;
            source.replace_range(start..end, &updated);
            replace(&mut source,
                "    val lastDurableExitDraft = remember(note.id) { mutableStateOf<NoteEntry?>(null) }",
                "    val lastDurableExitDraft = remember(note.id) { mutableStateOf<NoteEntry?>(null) }\n    val editorSaveCallbackActive = remember(note.id, editorSessionId) { java.util.concurrent.atomic.AtomicBoolean(true) }\n    DisposableEffect(editorSaveCallbackActive) {\n        onDispose { editorSaveCallbackActive.set(false) }\n    }")?;
            let start = source
                .find("    fun persistLatestDraft(\n")
                .ok_or("missing durable draft capture")?;
            let end = source[start..]
                .find("    fun beginRichCommit(")
                .map(|offset| start + offset)
                .ok_or("missing durable draft capture end")?;
            let mut save = source[start..end].to_owned();
            let checkpoint_start = save
                .find("        val checkpointForAttempt = if (exitAction")
                .ok_or("missing main-thread forced checkpoint")?;
            let checkpoint_end = save[checkpoint_start..]
                .find("        val completePersistence:")
                .map(|offset| checkpoint_start + offset)
                .ok_or("missing forced checkpoint end")?;
            save.replace_range(checkpoint_start..checkpoint_end,
                "        // Capture the exit draft before entering the independent IO queue.\n        // Disposal cannot cancel an already accepted checkpoint/commit.\n        val capturedBase = editorBaseNote\n        nextJournalSequence += 1L\n        val capturedSequence = nextJournalSequence\n        pendingJournalWrite.getAndSet(null)?.cancel()\n\n");
            replace(&mut save, "SmartisanNoteExitAction.SaveAndFlush -> viewModel.upsertNoteAndFlushResult(",
                "SmartisanNoteExitAction.SaveAndFlush -> viewModel.upsertCapturedNoteAndFlushResult(")?;
            replace(&mut save, "                draftCheckpoint = checkpointForAttempt,",
                "                baseNote = capturedBase,\n                editorSessionId = editorSessionId,\n                sequence = capturedSequence,")?;
            replace(&mut save, "        val completePersistence: (Boolean) -> Unit = { persisted ->\n            if (persisted) {",
                "        val completePersistence: (Boolean) -> Unit = completion@ { saved ->\n            if (!editorSaveCallbackActive.get() || latestNote.id != candidate.id) return@completion\n            val persisted = saved && viewModel.currentWorkspaceKey() == editorWorkspaceKey\n            if (persisted) {")?;
            replace(&mut save,
                "        val completeNotePersistence: (com.ofairyo.gridtimer.data.NoteSaveResult) -> Unit = { result ->\n            if (result.journaledForRecovery) {",
                "        val completeNotePersistence: (com.ofairyo.gridtimer.data.NoteSaveResult) -> Unit = noteCompletion@ { result ->\n            if (!editorSaveCallbackActive.get() || latestNote.id != candidate.id) return@noteCompletion\n            if (viewModel.currentWorkspaceKey() != editorWorkspaceKey) {\n                completePersistence(false)\n                return@noteCompletion\n            }\n            if (result.journaledForRecovery) {")?;
            source.replace_range(start..end, &save);
            replace(&mut source, "        ) { created, message ->\n            versionCreationBusy = false",
                "        ) { created, message ->\n            if (!editorSaveCallbackActive.get() || viewModel.currentWorkspaceKey() != editorWorkspaceKey) {\n                return@createNextNoteVersion\n            }\n            versionCreationBusy = false")?;
        }
        DOCUMENT => {
            replace(&mut source,
                "    val isSavingDraft = mutableStateOf(false)",
                "    val isSavingDraft = mutableStateOf(false)\n    val exitSaveGate = NoteEditorExitSaveGate<NoteEntry>()")?;
            replace(&mut source,
                "            viewModel.upsertNote(\n                nextNote,\n                expectedWorkspaceKey = editorWorkspaceKey,",
                "            viewModel.upsertNoteAutosave(\n                nextNote,\n                expectedWorkspaceKey = editorWorkspaceKey,")?;
            replace(&mut source,
                "    val blankDraftDeletionEligibleAtEntry = editorSession.blankDraftDeletionEligibleAtEntry",
                "    val editorSaveCallbackActive = remember(note.id, editorSession) { java.util.concurrent.atomic.AtomicBoolean(true) }\n    DisposableEffect(editorSaveCallbackActive) {\n        onDispose { editorSaveCallbackActive.set(false) }\n    }\n    val blankDraftDeletionEligibleAtEntry = editorSession.blankDraftDeletionEligibleAtEntry")?;
            replace(&mut source,
                "                    if (requestGeneration == saveRequestGeneration) {\n                        saveState = if (saved) {",
                "                    if (editorSaveCallbackActive.get() && viewModel.currentWorkspaceKey() == editorWorkspaceKey && requestGeneration == saveRequestGeneration) {\n                        saveState = if (saved) {")?;
            replace(&mut source,
                "        val completePersistence: (Boolean) -> Unit = { persisted ->\n            if (persistenceGeneration == saveRequestGeneration) {",
                "        // This callback belongs to the retained document session. Completing it\n        // must not depend on the disposed composition still being alive. UI delivery\n        // is checked by the exit request ticket after the session state is settled.\n        val completePersistence: (Boolean) -> Unit = { saved ->\n            val persisted = saved && viewModel.currentWorkspaceKey() == editorWorkspaceKey\n            if (persistenceGeneration == saveRequestGeneration) {")?;
            replace(&mut source,
                "        saveState = DocumentEditorSaveState.SAVING\n        saveRequestGeneration += 1L\n        val persistenceGeneration = saveRequestGeneration",
                "        val persistenceTicket = editorSession.exitSaveGate.beginPersistence(\n            draft = candidate,\n            requiresBlankDeletion = needsBlankDeletion,\n            queueEpoch = viewModel.noteMutationEpoch(),\n            onComplete = onComplete\n        ) ?: return\n        saveState = DocumentEditorSaveState.SAVING\n        saveRequestGeneration += 1L\n        val persistenceGeneration = saveRequestGeneration")?;
            replace(&mut source,
                "        val completePersistence: (Boolean) -> Unit = { saved ->\n            val persisted = saved && viewModel.currentWorkspaceKey() == editorWorkspaceKey\n            if (persistenceGeneration == saveRequestGeneration) {",
                "        val completePersistence: (Boolean) -> Unit = { saved ->\n            val completion = editorSession.exitSaveGate.completePersistence(\n                ticket = persistenceTicket,\n                saved = saved,\n                workspaceMatches = viewModel.currentWorkspaceKey() == editorWorkspaceKey\n            )\n            val persisted = completion.savedLocally\n            if (completion.accepted && completion.latest && persistenceGeneration == saveRequestGeneration) {")?;
            replace(&mut source,
                "            onComplete(persisted)\n        }\n        when (\n            resolveSmartisanNoteExitAction(\n                base = latestNote,",
                "            completion.deliver()\n        }\n        when (\n            resolveSmartisanNoteExitAction(\n                base = latestNote,")?;
            replace(&mut source,
                "        val alreadyDurable = committed != null &&\n            resolveSmartisanNoteExitAction(\n                base = committed,\n                candidate = candidate,\n                deleteBlankDraft = false\n            ) == SmartisanNoteExitAction.Noop\n        if (alreadyDurable && !needsBlankDeletion) {",
                "        // Compare the complete NoteEntry, including non-text blocks and\n        // protection/history metadata. A text-only projection cannot prove this\n        // captured document is durable.\n        val alreadyDurable = editorSession.exitSaveGate.canSkipDurableExit(\n            cachedDraft = committed,\n            visibleDraft = latestNote,\n            candidate = candidate,\n            queueEpoch = viewModel.noteMutationEpoch()\n        )\n        if (alreadyDurable && !needsBlankDeletion) {")?;
            replace(&mut source,
                "        when (\n            resolveSmartisanNoteExitAction(\n                base = latestNote,\n                candidate = candidate,\n                deleteBlankDraft = deleteBlankDraft && blankDraftDeletionEligibleAtEntry\n            )\n        ) {",
                "        val requestedExitAction = resolveSmartisanNoteExitAction(\n            base = latestNote,\n            candidate = candidate,\n            deleteBlankDraft = deleteBlankDraft && blankDraftDeletionEligibleAtEntry\n        )\n        // A queued autosave or explicit mutation can change the repository before\n        // this request runs. Only the verified fast path above may omit a captured\n        // upsert; an uncertain Noop must never flush an unrelated predecessor.\n        val exitAction = if (requestedExitAction == SmartisanNoteExitAction.Noop)\n            SmartisanNoteExitAction.SaveAndFlush else requestedExitAction\n        when (exitAction) {")?;
            replace(&mut source,
                "                onComplete = completePersistence\n            )\n        }\n    }\n\n    val latestExitPersistence",
                "                onComplete = completePersistence\n            )\n        }\n        // Bind the actual queue epoch after dispatch. Preparation can fail\n        // synchronously, in which case the completed ticket cannot be marked.\n        editorSession.exitSaveGate.markEnqueued(persistenceTicket, viewModel.noteMutationEpoch())\n    }\n\n    val latestExitPersistence")?;
            replace(&mut source, DOCUMENT_BACK_OLD, DOCUMENT_BACK_NEW)?;
        }
        REPOSITORY => {
            replace(&mut source, "    private suspend fun persistToWorkspace(workspaceKey: String, appData: AppData) {\n        withContext(Dispatchers.IO) {",
                "    private suspend fun persistToWorkspace(workspaceKey: String, appData: AppData) {\n        withContext(Dispatchers.IO) {\n            var stageStarted = System.nanoTime()\n            fun stageFinished(stage: String) {\n                recordNoteSaveStage(stage, stageStarted)\n                stageStarted = System.nanoTime()\n            }")?;
            replace(&mut source, "            val snapshotItemCount = persistedItemCount(snapshot)\n            val committedStamp =",
                "            val snapshotItemCount = persistedItemCount(snapshot)\n            stageFinished(\"snapshot_encode\")\n            val committedStamp =")?;
            replace(&mut source, "                // Preflight and the authoritative write share one primary transaction;",
                "                stageFinished(\"snapshot_preflight\")\n                // Preflight and the authoritative write share one primary transaction;")?;
            replace(&mut source, "            // Only publish the cache after commit AND successful recovery mirrors.",
                "            stageFinished(\"sqlite_commit\")\n            // Only publish the cache after commit AND successful recovery mirrors.")?;
            replace(&mut source, "                    message = \"Primary SQLite snapshot committed; recovery mirror refresh is deferred.\",\n                    throwable = throwable\n                )\n            }\n        }\n    }",
                "                    message = \"Primary SQLite snapshot committed; recovery mirror refresh is deferred.\",\n                    throwable = throwable\n                )\n            }\n            stageFinished(\"recovery_mirror\")\n        }\n    }")?;
            replace(&mut source, "    private suspend fun persistToWorkspace(workspaceKey: String, appData: AppData) {",
                &format!("{STAGE_DIAGNOSTIC}    private suspend fun persistToWorkspace(workspaceKey: String, appData: AppData) {{"))?;
            instrument_write_wait(&mut source,
                "    suspend fun createNextNoteVersionDurably(",
                "    private suspend fun redactEncryptedNoteRecoveryCopies(",
                "        withContext(NonCancellable + Dispatchers.Default) {\n            writeMutex.withLock {",
                "        val writeWaitStarted = System.nanoTime()\n        withContext(NonCancellable + Dispatchers.Default) {\n            writeMutex.withLock {\n                recordNoteSaveStage(\"version_write_lock\", writeWaitStarted)")?;
            instrument_write_wait(&mut source,
                "    private suspend fun updateDataDetailed(",
                "    // Sanitization remains authoritative.",
                "        writeMutex.withLock {",
                "        val writeWaitStarted = System.nanoTime()\n        writeMutex.withLock {\n            if (noteMutationId != null) recordNoteSaveStage(\"draft_write_lock\", writeWaitStarted)")?;
        }
        _ => {}
    }
    Ok(source)
}

fn replace(source: &mut String, old: &str, new: &str) -> Result<(), String> {
    if source.matches(old).count() != 1 {
        return Err(format!("note save queue anchor is not unique: {old}"));
    }
    *source = source.replacen(old, new, 1);
    Ok(())
}

fn instrument_write_wait(
    source: &mut String,
    start: &str,
    end: &str,
    old: &str,
    new: &str,
) -> Result<(), String> {
    let from = source
        .find(start)
        .ok_or_else(|| format!("missing {start}"))?;
    let to = source[from..]
        .find(end)
        .map(|offset| from + offset)
        .ok_or_else(|| format!("missing {end}"))?;
    let mut section = source[from..to].to_owned();
    replace(&mut section, old, new)?;
    source.replace_range(from..to, &section);
    Ok(())
}

const STAGE_DIAGNOSTIC: &str = r####"    // Bounded slow-operation diagnostics contain only a fixed phase and duration.
    // Never record note text, note identifiers, workspace/account identity or credentials.
    private fun recordNoteSaveStage(stage: String, startedNanos: Long) {
        val elapsedMillis = ((System.nanoTime() - startedNanos).coerceAtLeast(0L) / 1_000_000L)
        if (elapsedMillis < 1_000L) return
        logDiagnosticEvent(
            category = "notes.local_save.latency",
            message = "phase=$stage durationMs=$elapsedMillis"
        )
    }

"####;

const AUTOSAVE: &str = r####"    // Only the editor debounce uses replacement semantics. Explicit saves, local
    // versions, deletion, protection transitions and lifecycle flushes stay FIFO barriers.
    fun upsertNoteAutosave(
        note: NoteEntry,
        expectedWorkspaceKey: String = currentWorkspaceKey(),
        onComplete: (Boolean) -> Unit = {}
    ) {
        // Protected drafts must be sealed before the existing background lock can
        // revoke the session. Preserve that protection path and do not retain an
        // unsealed encrypted draft in a replaceable queue entry.
        if (note.encryption != null) {
            upsertNote(note, expectedWorkspaceKey, onComplete)
            return
        }
        val completion = noteMutationController.enqueueAutosave(expectedWorkspaceKey, note.id) {
            // Prepare only the surviving draft on the IO worker. A workspace switch
            // is still rejected by repository.upsertNote before any publication.
            val persistedNote = noteForPersistence(note, expectedWorkspaceKey)
                ?: return@enqueueAutosave NoteSaveResult.failed(NoteSaveFailure.SERIALIZATION_FAILED)
            if (repository.upsertNote(persistedNote, expectedWorkspaceKey)) {
                NoteSaveResult.committed()
            } else {
                NoteSaveResult.failed(NoteSaveFailure.FLUSH_FAILED)
            }
        }
        viewModelScope.launch {
            val result = completion.await()
            onComplete(result.committed && currentWorkspaceKey() == expectedWorkspaceKey)
        }
    }

"####;

const CAPTURED_SAVE: &str = r####"    fun upsertCapturedNoteAndFlushResult(
        note: NoteEntry,
        baseNote: NoteEntry,
        expectedWorkspaceKey: String,
        reason: String,
        editorSessionId: String,
        sequence: Long,
        onComplete: (NoteSaveResult) -> Unit
    ) {
        if (note.encryption != null) {
            // Preserve ciphertext sealing before the existing timed background
            // lock. The ordinary document path below never does checkpoint IO on Main.
            val checkpoint = journalNoteDraft(expectedWorkspaceKey, baseNote, note, editorSessionId, sequence)
            if (checkpoint == null) {
                onComplete(NoteSaveResult.failed(NoteSaveFailure.FLUSH_FAILED))
                return
            }
            upsertNoteAndFlushResult(note, expectedWorkspaceKey, reason, checkpoint, onComplete)
            return
        }
        // Accept one immutable request on the main dispatcher. Checkpoint IO,
        // protection preparation and durable commit execute as one FIFO barrier.
        enqueueNoteSaveMutation(onComplete = { result ->
            if (currentWorkspaceKey() == expectedWorkspaceKey) onComplete(result)
            else onComplete(NoteSaveResult.failed(NoteSaveFailure.WORKSPACE_CHANGED))
        }) {
            if (currentWorkspaceKey() != expectedWorkspaceKey || note.id != baseNote.id) {
                return@enqueueNoteSaveMutation NoteSaveResult.failed(NoteSaveFailure.WORKSPACE_CHANGED)
            }
            val persistedNote = noteForPersistence(note, expectedWorkspaceKey)
                ?: return@enqueueNoteSaveMutation NoteSaveResult.failed(NoteSaveFailure.SERIALIZATION_FAILED)
            runRequiredNoteCheckpointSave(
                checkpoint = {
                    journalNoteDraft(expectedWorkspaceKey, baseNote, note, editorSessionId, sequence)
                },
                commit = { checkpoint ->
                    repository.upsertNoteDurablyResult(
                        note = persistedNote,
                        expectedWorkspaceKey = expectedWorkspaceKey,
                        reason = reason,
                        draftCheckpoint = checkpoint
                    )
                }
            )
        }
    }

"####;

pub const CONTROLLER: &str = r####"internal class NoteMutationDrainController(
    private val scope: CoroutineScope,
    private val reportSlowOperation: ((String, Long) -> Unit)? = null,
    private val reportFailure: (Throwable) -> Unit
) {
    private class QueuedNoteMutation(
        var operation: suspend () -> NoteSaveResult,
        completion: CompletableDeferred<NoteSaveResult>,
        val autosaveKey: Pair<String, String>? = null
    ) {
        val completions = mutableListOf(completion)
        val queuedAtNanos = System.nanoTime()
    }

    private val queue = Channel<QueuedNoteMutation>(capacity = Channel.BUFFERED)
    private val lifecycleLock = Any()
    // Only an adjacent, accepted, not-yet-started autosave can be replaced. This
    // reference is cleared by every explicit mutation and before execution starts.
    private var replaceableTail: QueuedNoteMutation? = null
    private var acceptedMutationEpoch = 0L
    fun mutationEpoch(): Long = synchronized(lifecycleLock) { acceptedMutationEpoch }
    private val shutdownRequested = AtomicBoolean(false)
    private val shutdownCompletion = CompletableDeferred<NoteSaveResult>()
    private val worker = scope.launch { runWorker() }

    private fun reportSlow(phase: String, startedNanos: Long) {
        val millis = (System.nanoTime() - startedNanos).coerceAtLeast(0L) / 1_000_000L
        if (millis >= 1_000L) reportSlowOperation?.invoke(phase, millis)
    }

    private suspend fun runWorker() {
        try {
            for (mutation in queue) {
                val operation = synchronized(lifecycleLock) {
                    if (replaceableTail === mutation) replaceableTail = null
                    mutation.operation
                }
                val kind = if (mutation.autosaveKey == null) "explicit" else "autosave"
                reportSlow("$kind.queue_wait", mutation.queuedAtNanos)
                val started = System.nanoTime()
                val result = execute(mutation, operation)
                reportSlow("$kind.operation", started)
                mutation.completions.forEach { it.complete(result) }
            }
        } finally {
            queue.close()
            synchronized(lifecycleLock) { replaceableTail = null }
            drainAbandonedMutations()
        }
    }

    private suspend fun execute(
        mutation: QueuedNoteMutation,
        operation: suspend () -> NoteSaveResult
    ): NoteSaveResult {
        return try {
            operation()
        } catch (cancellation: CancellationException) {
            val failed = NoteSaveResult.failed(
                NoteSaveFailure.QUEUE_UNAVAILABLE,
                "The note mutation worker was cancelled."
            )
            mutation.completions.forEach { it.complete(failed) }
            throw cancellation
        } catch (throwable: Throwable) {
            reportFailure(throwable)
            NoteSaveResult.failed(
                NoteSaveFailure.VERSION_REPAIR_FAILED,
                "${throwable::class.java.simpleName}: ${throwable.message.orEmpty()}"
            )
        }
    }

    private fun drainAbandonedMutations() {
        while (true) {
            val abandoned = queue.tryReceive().getOrNull() ?: break
            val failed = NoteSaveResult.failed(
                NoteSaveFailure.QUEUE_UNAVAILABLE,
                "The note mutation queue closed before this operation ran."
            )
            abandoned.completions.forEach { it.complete(failed) }
        }
    }

    fun enqueue(operation: suspend () -> NoteSaveResult): CompletableDeferred<NoteSaveResult> {
        val completion = CompletableDeferred<NoteSaveResult>()
        val queued = synchronized(lifecycleLock) {
            replaceableTail = null
            val accepted = !shutdownRequested.get() && queue.trySend(
                QueuedNoteMutation(operation = operation, completion = completion)
            ).isSuccess
            if (accepted) acceptedMutationEpoch += 1L
            accepted
        }
        if (!queued) completion.complete(queueUnavailableResult())
        return completion
    }

    fun enqueueAutosave(
        workspaceKey: String,
        noteId: String,
        operation: suspend () -> NoteSaveResult
    ): CompletableDeferred<NoteSaveResult> {
        val completion = CompletableDeferred<NoteSaveResult>()
        val key = workspaceKey to noteId
        val accepted = synchronized(lifecycleLock) {
            val admitted = if (shutdownRequested.get() || workspaceKey.isBlank() || noteId.isBlank()) {
                false
            } else {
                val previous = replaceableTail
                if (previous != null && previous.autosaveKey == key) {
                    previous.operation = operation
                    previous.completions.add(completion)
                    true
                } else {
                    val candidate = QueuedNoteMutation(operation, completion, key)
                    val queued = queue.trySend(candidate).isSuccess
                    replaceableTail = candidate.takeIf { queued }
                    queued
                }
            }
            if (admitted) acceptedMutationEpoch += 1L
            admitted
        }
        if (!accepted) completion.complete(queueUnavailableResult())
        return completion
    }

    private fun queueUnavailableResult(): NoteSaveResult = NoteSaveResult.failed(
        NoteSaveFailure.QUEUE_UNAVAILABLE,
        "The note mutation queue is full, closing, or closed."
    )

    private suspend fun finishShutdown(
        flushOperation: suspend () -> NoteSaveResult,
        afterClosed: () -> Unit
    ) {
        var terminalResult = queueUnavailableResult()
        try {
            terminalResult = runFlushBarrier(flushOperation)
        } catch (cancellation: CancellationException) {
            throw cancellation
        } catch (throwable: Throwable) {
            reportFailure(throwable)
        } finally {
            withContext(NonCancellable) {
                queue.close()
                worker.join()
                runCatching(afterClosed).onFailure(reportFailure)
                shutdownCompletion.complete(terminalResult)
            }
        }
    }

    private suspend fun runFlushBarrier(
        flushOperation: suspend () -> NoteSaveResult
    ): NoteSaveResult {
        val flushCompletion = CompletableDeferred<NoteSaveResult>()
        queue.send(QueuedNoteMutation(flushOperation, flushCompletion))
        return flushCompletion.await()
    }

    fun shutdownAfterDrain(
        afterClosed: () -> Unit = {},
        flushOperation: suspend () -> NoteSaveResult
    ): CompletableDeferred<NoteSaveResult> {
        val shouldStart = synchronized(lifecycleLock) {
            replaceableTail = null
            shutdownRequested.compareAndSet(false, true)
        }
        if (shouldStart) scope.launch { finishShutdown(flushOperation, afterClosed) }
        return shutdownCompletion
    }

    fun isAcceptingMutations(): Boolean = !shutdownRequested.get()
}

// A failed recovery checkpoint must keep the editor open. The commit callback
// is unreachable until the exact captured draft has a durable recovery receipt.
internal suspend fun <T : Any> runRequiredNoteCheckpointSave(
    checkpoint: suspend () -> T?,
    commit: suspend (T) -> NoteSaveResult
): NoteSaveResult {
    val receipt = checkpoint() ?: return NoteSaveResult.failed(NoteSaveFailure.FLUSH_FAILED)
    return commit(receipt)
}

"####;

pub const TEST_PATH: &str = "com/ofairyo/gridtimer/ui/NoteSaveQueueTest.kt";
pub const TEST_CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui

import com.ofairyo.gridtimer.data.NoteSaveFailure
import com.ofairyo.gridtimer.data.NoteSaveResult
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.*
import org.junit.Test

class NoteSaveQueueTest {
    @Test fun slowWriteKeepsOnlyLatestAdjacentAutosaveBeforeExplicitSave() = runBlocking {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
        val gate = CompletableDeferred<Unit>()
        val started = CompletableDeferred<Unit>()
        val events = mutableListOf<String>()
        val controller = NoteMutationDrainController(scope) { throw it }
        try {
            val blocking = controller.enqueue { started.complete(Unit); gate.await(); NoteSaveResult.committed() }
            withTimeout(5_000L) { started.await() }
            val drafts = (1..100).map { revision ->
                controller.enqueueAutosave("workspace-a", "note-a") {
                    events += "draft-$revision"
                    NoteSaveResult.committed()
                }
            }
            val explicit = controller.enqueue { events += "version"; NoteSaveResult.committed() }
            assertFalse(explicit.isCompleted)
            assertTrue(drafts.none { it.isCompleted })
            gate.complete(Unit)
            withTimeout(5_000L) {
                assertTrue(blocking.await().committed)
                drafts.forEach { assertTrue(it.await().committed) }
                assertTrue(explicit.await().committed)
            }
            assertEquals(listOf("draft-100", "version"), events)
        } finally { scope.cancel() }
    }

    @Test fun explicitBarrierWorkspaceAndNoteIdentityCannotBeCrossed() = runBlocking {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
        val gate = CompletableDeferred<Unit>()
        val started = CompletableDeferred<Unit>()
        val events = mutableListOf<String>()
        val controller = NoteMutationDrainController(scope) { throw it }
        try {
            controller.enqueue { started.complete(Unit); gate.await(); NoteSaveResult.committed() }
            withTimeout(5_000L) { started.await() }
            controller.enqueueAutosave("w-a", "note-a") { events += "a-before"; NoteSaveResult.committed() }
            controller.enqueue { events += "protection-barrier"; NoteSaveResult.committed() }
            controller.enqueueAutosave("w-a", "note-a") { events += "a-after"; NoteSaveResult.committed() }
            controller.enqueueAutosave("w-b", "note-a") { events += "b"; NoteSaveResult.committed() }
            controller.enqueueAutosave("w-b", "note-b") { events += "b-other-note"; NoteSaveResult.committed() }
            val shutdown = controller.shutdownAfterDrain { events += "flush"; NoteSaveResult.committed() }
            val refused = controller.enqueueAutosave("w-b", "note-b") { fail("late draft ran"); NoteSaveResult.committed() }
            assertEquals(NoteSaveFailure.QUEUE_UNAVAILABLE, refused.await().failure)
            gate.complete(Unit)
            assertTrue(withTimeout(5_000L) { shutdown.await() }.committed)
            assertEquals(listOf("a-before", "protection-barrier", "a-after", "b", "b-other-note", "flush"), events)
        } finally { scope.cancel() }
    }

    @Test fun replacementCannotMutateRunningWriteAndFailureIsNeverReportedAsSaved() = runBlocking {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
        val gate = CompletableDeferred<Unit>()
        val started = CompletableDeferred<Unit>()
        val events = mutableListOf<String>()
        val controller = NoteMutationDrainController(scope) { throw it }
        try {
            val running = controller.enqueueAutosave("w", "note") {
                started.complete(Unit); gate.await(); events += "running"
                NoteSaveResult.failed(NoteSaveFailure.DATABASE_WRITE_FAILED)
            }
            withTimeout(5_000L) { started.await() }
            val old = controller.enqueueAutosave("w", "note") { fail("superseded draft ran"); NoteSaveResult.committed() }
            val latest = controller.enqueueAutosave("w", "note") {
                events += "latest"; NoteSaveResult.failed(NoteSaveFailure.READ_ONLY_PROTECTION)
            }
            gate.complete(Unit)
            withTimeout(5_000L) {
                assertEquals(NoteSaveFailure.DATABASE_WRITE_FAILED, running.await().failure)
                assertEquals(NoteSaveFailure.READ_ONLY_PROTECTION, old.await().failure)
                assertEquals(NoteSaveFailure.READ_ONLY_PROTECTION, latest.await().failure)
            }
            assertEquals(listOf("running", "latest"), events)
        } finally { scope.cancel() }
    }

    @Test fun failedCheckpointCannotEnterCommitAndReceiptPrecedesDurableCommit() = runBlocking {
        val events = mutableListOf<String>()
        val failed = runRequiredNoteCheckpointSave<String>(
            checkpoint = { events += "failed-checkpoint"; null },
            commit = { fail("commit must not run without a checkpoint"); NoteSaveResult.committed() }
        )
        assertFalse(failed.savedLocally)
        assertEquals(NoteSaveFailure.FLUSH_FAILED, failed.failure)
        val saved = runRequiredNoteCheckpointSave(
            checkpoint = { events += "durable-checkpoint"; "exact-receipt" },
            commit = { receipt ->
                assertEquals("exact-receipt", receipt)
                events += "durable-commit"
                NoteSaveResult.committed()
            }
        )
        assertTrue(saved.committed)
        assertEquals(listOf("failed-checkpoint", "durable-checkpoint", "durable-commit"), events)
    }

    @Test fun acceptedQueueMutationsAdvanceFenceIncludingCoalescedAutosaves() = runBlocking {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
        val gate = CompletableDeferred<Unit>()
        val started = CompletableDeferred<Unit>()
        val controller = NoteMutationDrainController(scope) { throw it }
        try {
            assertEquals(0L, controller.mutationEpoch())
            controller.enqueue { started.complete(Unit); gate.await(); NoteSaveResult.committed() }
            withTimeout(5_000L) { started.await() }
            assertEquals(1L, controller.mutationEpoch())
            controller.enqueueAutosave("w", "note") { NoteSaveResult.committed() }
            assertEquals(2L, controller.mutationEpoch())
            controller.enqueueAutosave("w", "note") { NoteSaveResult.committed() }
            assertEquals(3L, controller.mutationEpoch())
            controller.enqueue { NoteSaveResult.committed() }
            assertEquals(4L, controller.mutationEpoch())
            controller.enqueueAutosave("", "note") { fail("invalid draft admitted"); NoteSaveResult.committed() }
            assertEquals(4L, controller.mutationEpoch())
        } finally { gate.complete(Unit); scope.cancel() }
    }

}
"####;

pub const EXIT_SAVE_GATE_PATH: &str = "com/ofairyo/gridtimer/ui/NoteEditorExitSaveGate.kt";
pub const EXIT_SAVE_GATE_CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui

// An accepted exit belongs to the retained draft session, rather than a single
// composition. Disposing its UI cannot abandon its eventual storage result.
internal class NoteEditorExitSaveGate<D> {
    class Ticket internal constructor()

    data class Completion(
        val accepted: Boolean,
        val savedLocally: Boolean,
        val notifyUi: Boolean
    )

    class PersistenceTicket<D> internal constructor(
        internal val draft: D,
        internal val requiresBlankDeletion: Boolean,
        internal val callbacks: MutableList<(Boolean) -> Unit>
    ) {
        internal var enqueuedEpoch: Long? = null
    }

    class PersistenceCompletion internal constructor(
        val accepted: Boolean,
        val latest: Boolean,
        val savedLocally: Boolean,
        private val callbacks: List<(Boolean) -> Unit>
    ) {
        fun deliver() = callbacks.forEach { it(savedLocally) }
    }

    private val pendingPersistence = mutableListOf<PersistenceTicket<D>>()
    private var latestPersistence: PersistenceTicket<D>? = null
    private var durableEpoch: Long? = null
    private var current: Ticket? = null

    @Synchronized
    fun beginPersistence(
        draft: D,
        requiresBlankDeletion: Boolean,
        queueEpoch: Long = 0L,
        onComplete: (Boolean) -> Unit
    ): PersistenceTicket<D>? {
        // Only join the latest accepted persistence. Looking through older
        // requests would reorder A -> B -> A into A -> B and lose the final A.
        val pending = latestPersistence?.takeIf {
            it.enqueuedEpoch == queueEpoch &&
                it.draft == draft && (it.requiresBlankDeletion || !requiresBlankDeletion)
        }
        if (pending != null) {
            pending.callbacks += onComplete
            return null
        }
        return PersistenceTicket(draft, requiresBlankDeletion, mutableListOf(onComplete)).also {
            pendingPersistence += it
            latestPersistence = it
        }
    }

    @Synchronized
    fun markEnqueued(ticket: PersistenceTicket<D>, queueEpoch: Long): Boolean {
        if (ticket !in pendingPersistence) return false
        ticket.enqueuedEpoch = queueEpoch
        return true
    }

    @Synchronized
    fun isDurableAtEpoch(queueEpoch: Long): Boolean =
        durableEpoch != null && durableEpoch == queueEpoch

    @Synchronized
    fun canSkipDurableExit(
        cachedDraft: D?,
        visibleDraft: D,
        candidate: D,
        queueEpoch: Long
    ): Boolean = cachedDraft != null &&
        candidate == cachedDraft && candidate == visibleDraft &&
        pendingPersistence.isEmpty() && isDurableAtEpoch(queueEpoch)

    @Synchronized
    fun hasPendingPersistence(): Boolean = pendingPersistence.isNotEmpty()

    @Synchronized
    fun completePersistence(
        ticket: PersistenceTicket<D>,
        saved: Boolean,
        workspaceMatches: Boolean
    ): PersistenceCompletion {
        if (!pendingPersistence.remove(ticket)) {
            return PersistenceCompletion(false, false, false, emptyList())
        }
        val latest = latestPersistence === ticket
        if (latest) {
            latestPersistence = null
            durableEpoch = ticket.enqueuedEpoch.takeIf { saved && workspaceMatches }
        }
        return PersistenceCompletion(true, latest, saved && workspaceMatches, ticket.callbacks.toList())
    }

    @Synchronized
    fun begin(): Ticket? {
        if (current != null) return null
        return Ticket().also { current = it }
    }

    @Synchronized
    fun isSaving(): Boolean = current != null

    @Synchronized
    fun complete(
        ticket: Ticket,
        saved: Boolean,
        uiIsCurrent: Boolean,
        workspaceMatches: Boolean
    ): Completion {
        if (current !== ticket) return Completion(false, false, false)
        current = null
        return Completion(
            accepted = true,
            savedLocally = saved && workspaceMatches,
            notifyUi = uiIsCurrent && workspaceMatches
        )
    }
}
"####;

const DOCUMENT_BACK_OLD: &str = r####"    fun handleBack() {
        if (isSavingDraft) {
            return
        }
        saveRequestGeneration += 1L
        isSavingDraft = true
        persistLatestDraft(
            reason = "document_note_editor_back",
            deleteBlankDraft = true
        ) { persisted ->
            isSavingDraft = false
            if (persisted) {
                editorSessionViewModel.clear(note.id, editorWorkspaceKey, editorSession)
                onBack()
            } else {
                Toast.makeText(context, "保存失败，笔记仍留在编辑页。", Toast.LENGTH_LONG).show()
            }
        }
    }"####;

const DOCUMENT_BACK_NEW: &str = r####"    fun handleBack() {
        val exitTicket = editorSession.exitSaveGate.begin() ?: return
        isSavingDraft = true
        persistLatestDraft(
            reason = "document_note_editor_back",
            deleteBlankDraft = true
        ) exitCompletion@ { persisted ->
            val completion = editorSession.exitSaveGate.complete(
                ticket = exitTicket,
                saved = persisted,
                uiIsCurrent = editorSaveCallbackActive.get() && latestNote.id == note.id,
                workspaceMatches = viewModel.currentWorkspaceKey() == editorWorkspaceKey
            )
            if (!completion.accepted) return@exitCompletion
            // Release the retained busy state even when this editor was recreated
            // or removed. An old callback may never navigate the replacement UI.
            isSavingDraft = false
            if (!completion.notifyUi) return@exitCompletion
            if (completion.savedLocally) {
                editorSessionViewModel.clear(note.id, editorWorkspaceKey, editorSession)
                onBack()
            } else {
                Toast.makeText(context, "保存失败，笔记仍留在编辑页。", Toast.LENGTH_LONG).show()
            }
        }
    }"####;

pub const EXIT_SAVE_GATE_TEST_PATH: &str = "com/ofairyo/gridtimer/ui/NoteEditorExitSaveGateTest.kt";
pub const EXIT_SAVE_GATE_TEST_CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui

import org.junit.Assert.*
import org.junit.Test

class NoteEditorExitSaveGateTest {
    @Test fun successfulCurrentSaveClearsBusyAndAllowsNavigation() {
        val gate = NoteEditorExitSaveGate<String>()
        val ticket = checkNotNull(gate.begin())
        assertTrue(gate.isSaving())
        val completion = gate.complete(ticket, saved = true, uiIsCurrent = true, workspaceMatches = true)
        assertTrue(completion.accepted)
        assertTrue(completion.savedLocally)
        assertTrue(completion.notifyUi)
        assertFalse(gate.isSaving())
    }

    @Test fun failedSaveClearsBusyAndAllowsRetryWithoutClaimingSuccess() {
        val gate = NoteEditorExitSaveGate<String>()
        val ticket = checkNotNull(gate.begin())
        val completion = gate.complete(ticket, saved = false, uiIsCurrent = true, workspaceMatches = true)
        assertTrue(completion.accepted)
        assertFalse(completion.savedLocally)
        assertTrue(completion.notifyUi)
        assertFalse(gate.isSaving())
        assertNotNull(gate.begin())
    }

    @Test fun disposedEditorStillSettlesRetainedSaveWithoutNavigatingReplacement() {
        val gate = NoteEditorExitSaveGate<String>()
        val ticket = checkNotNull(gate.begin())
        val completion = gate.complete(ticket, saved = true, uiIsCurrent = false, workspaceMatches = true)
        assertTrue(completion.accepted)
        assertTrue(completion.savedLocally)
        assertFalse(completion.notifyUi)
        assertFalse(gate.isSaving())
        assertNotNull(gate.begin())
    }

    @Test fun workspaceChangeSettlesOldBusyWithoutSuccessOrNavigation() {
        val gate = NoteEditorExitSaveGate<String>()
        val ticket = checkNotNull(gate.begin())
        val completion = gate.complete(ticket, saved = true, uiIsCurrent = true, workspaceMatches = false)
        assertTrue(completion.accepted)
        assertFalse(completion.savedLocally)
        assertFalse(completion.notifyUi)
        assertFalse(gate.isSaving())
    }

    @Test fun duplicateBackDoesNotAcceptAnotherInFlightExit() {
        val gate = NoteEditorExitSaveGate<String>()
        checkNotNull(gate.begin())
        assertNull(gate.begin())
        assertTrue(gate.isSaving())
    }

    @Test fun staleCompletionCannotReleaseNewerExit() {
        val gate = NoteEditorExitSaveGate<String>()
        val old = checkNotNull(gate.begin())
        gate.complete(old, saved = false, uiIsCurrent = true, workspaceMatches = true)
        val fresh = checkNotNull(gate.begin())
        val stale = gate.complete(old, saved = true, uiIsCurrent = true, workspaceMatches = true)
        assertFalse(stale.accepted)
        assertFalse(stale.savedLocally)
        assertFalse(stale.notifyUi)
        assertTrue(gate.isSaving())
        assertTrue(gate.complete(fresh, saved = true, uiIsCurrent = true, workspaceMatches = true).accepted)
    }

    @Test fun duplicateCompletionCannotNavigateTwice() {
        val gate = NoteEditorExitSaveGate<String>()
        val ticket = checkNotNull(gate.begin())
        gate.complete(ticket, saved = true, uiIsCurrent = true, workspaceMatches = true)
        val duplicate = gate.complete(ticket, saved = true, uiIsCurrent = true, workspaceMatches = true)
        assertFalse(duplicate.accepted)
        assertFalse(duplicate.notifyUi)
        assertFalse(gate.isSaving())
    }

    @Test fun sameDraftLifecycleRequestsJoinOnePersistenceAndAllReceiveResult() {
        val gate = NoteEditorExitSaveGate<String>()
        val delivered = mutableListOf<Boolean>()
        val first = checkNotNull(gate.beginPersistence("draft", false) { delivered += it })
        assertTrue(gate.markEnqueued(first, 0L))
        assertNull(gate.beginPersistence("draft", false) { delivered += it })
        val completion = gate.completePersistence(first, saved = true, workspaceMatches = true)
        assertTrue(completion.accepted)
        assertTrue(completion.latest)
        completion.deliver()
        assertEquals(listOf(true, true), delivered)
    }

    @Test fun blankDeletionCannotJoinWeakerPendingPersistence() {
        val gate = NoteEditorExitSaveGate<String>()
        val weak = checkNotNull(gate.beginPersistence("blank", false) {})
        assertTrue(gate.markEnqueued(weak, 0L))
        val acceptedStrong = gate.beginPersistence("blank", true) {}
        assertNotNull(acceptedStrong)
        val strong = checkNotNull(acceptedStrong)
        val old = gate.completePersistence(weak, saved = true, workspaceMatches = true)
        assertFalse(old.latest)
        assertTrue(gate.completePersistence(strong, saved = true, workspaceMatches = true).latest)
    }

    @Test fun differentDraftCannotJoinOrOverwriteLatestSessionCompletion() {
        val gate = NoteEditorExitSaveGate<String>()
        val old = checkNotNull(gate.beginPersistence("old", false) {})
        val fresh = checkNotNull(gate.beginPersistence("fresh", false) {})
        assertFalse(gate.completePersistence(old, saved = true, workspaceMatches = true).latest)
        assertTrue(gate.completePersistence(fresh, saved = true, workspaceMatches = true).latest)
    }

    @Test fun failureAndWorkspaceRejectionReachAllJoinedCallbacksAsFailure() {
        for (workspaceMatches in listOf(false, true)) {
            val gate = NoteEditorExitSaveGate<String>()
            val delivered = mutableListOf<Boolean>()
            val first = checkNotNull(gate.beginPersistence("draft", false) { delivered += it })
            assertTrue(gate.markEnqueued(first, 0L))
            assertNull(gate.beginPersistence("draft", false) { delivered += it })
            val completion = gate.completePersistence(first, saved = !workspaceMatches, workspaceMatches = workspaceMatches)
            assertFalse(completion.savedLocally)
            completion.deliver()
            assertEquals(listOf(false, false), delivered)
            assertNotNull(gate.beginPersistence("draft", false) {})
        }
    }

    @Test fun duplicatePersistenceCompletionCannotDeliverCallbacksAgain() {
        val gate = NoteEditorExitSaveGate<String>()
        var calls = 0
        val ticket = checkNotNull(gate.beginPersistence("draft", false) { calls++ })
        gate.completePersistence(ticket, saved = true, workspaceMatches = true).deliver()
        val duplicate = gate.completePersistence(ticket, saved = true, workspaceMatches = true)
        assertFalse(duplicate.accepted)
        duplicate.deliver()
        assertEquals(1, calls)
    }


    @Test fun sameDraftCannotJoinAcrossAnotherAcceptedDraft() {
        val gate = NoteEditorExitSaveGate<String>()
        val events = mutableListOf<String>()
        val firstA = checkNotNull(gate.beginPersistence("A", false) { events += "first-A" })
        assertTrue(gate.markEnqueued(firstA, 0L))
        val middleB = checkNotNull(gate.beginPersistence("B", false) { events += "B" })
        assertTrue(gate.markEnqueued(middleB, 0L))
        val acceptedFinalA = gate.beginPersistence("A", false) { events += "last-A" }
        assertNotNull(acceptedFinalA)
        val finalA = checkNotNull(acceptedFinalA)
        assertFalse(gate.completePersistence(firstA, true, true).latest)
        gate.completePersistence(middleB, true, true).deliver()
        val finalCompletion = gate.completePersistence(finalA, true, true)
        assertTrue(finalCompletion.latest)
        finalCompletion.deliver()
        assertEquals(listOf("B", "last-A"), events)
    }



    @Test fun pendingBlankDeletionAlsoSatisfiesWeakerLifecycleSave() {
        val gate = NoteEditorExitSaveGate<String>()
        val callbacks = mutableListOf<String>()
        val deletion = checkNotNull(gate.beginPersistence("blank", true) { callbacks += "back-delete" })
        assertTrue(gate.markEnqueued(deletion, 0L))
        assertNull(gate.beginPersistence("blank", false) { callbacks += "stop" })
        assertNull(gate.beginPersistence("blank", false) { callbacks += "dispose" })
        val completion = gate.completePersistence(deletion, true, true)
        completion.deliver()
        assertEquals(listOf("back-delete", "stop", "dispose"), callbacks)
        assertFalse(gate.hasPendingPersistence())
    }


    @Test fun interveningQueueBarrierPreventsJoiningOlderLifecycleWrite() {
        val gate = NoteEditorExitSaveGate<String>()
        val beforeRestore = checkNotNull(gate.beginPersistence("draft", false, 1L) {})
        assertTrue(gate.markEnqueued(beforeRestore, 2L))
        val afterRestore = gate.beginPersistence("draft", false, 3L) {}
        assertNotNull(afterRestore)
    }

    @Test fun actualBoundEpochAllowsJoinButLaterAutosaveEpochForcesNewSave() {
        val gate = NoteEditorExitSaveGate<String>()
        val ticket = checkNotNull(gate.beginPersistence("draft", false, 5L) {})
        assertTrue(gate.markEnqueued(ticket, 6L))
        assertNull(gate.beginPersistence("draft", false, 6L) {})
        assertNotNull(gate.beginPersistence("draft", false, 7L) {})
    }

    @Test fun unmarkedOrAlreadyCompletedTicketCannotRepresentAcceptedQueueTail() {
        val gate = NoteEditorExitSaveGate<String>()
        val first = checkNotNull(gate.beginPersistence("draft", false, 8L) {})
        assertNotNull(gate.beginPersistence("draft", false, 8L) {})
        gate.completePersistence(first, false, true)
        assertFalse(gate.markEnqueued(first, 9L))
    }


    @Test fun completedDurableProofExpiresWhenAnotherQueueMutationIsAccepted() {
        val gate = NoteEditorExitSaveGate<String>()
        val ticket = checkNotNull(gate.beginPersistence("A", false, 10L) {})
        assertTrue(gate.markEnqueued(ticket, 11L))
        gate.completePersistence(ticket, true, true)
        assertTrue(gate.isDurableAtEpoch(11L))
        assertFalse(gate.isDurableAtEpoch(12L))
        assertFalse(gate.hasPendingPersistence())
    }

    @Test fun failedUnmarkedOrWrongWorkspacePersistenceCannotCreateDurableProof() {
        for (mode in 0..2) {
            val gate = NoteEditorExitSaveGate<String>()
            val ticket = checkNotNull(gate.beginPersistence("draft", false, 4L) {})
            if (mode != 0) gate.markEnqueued(ticket, 5L)
            gate.completePersistence(ticket, saved = mode != 1, workspaceMatches = mode != 2)
            assertFalse(gate.isDurableAtEpoch(5L))
        }
    }


    private data class CompleteDocumentPayload(
        val text: String,
        val contactCaption: String,
        val callSummary: String,
        val attachments: List<String>,
        val protectionRevision: Long
    )

    @Test fun equalTextWithChangedNonTextPayloadCannotSkipDurableExit() {
        val cached = CompleteDocumentPayload("same text", "old contact", "old call", listOf("image"), 4L)
        val gate = NoteEditorExitSaveGate<CompleteDocumentPayload>()
        val ticket = checkNotNull(gate.beginPersistence(cached, false, 1L) {})
        gate.markEnqueued(ticket, 2L)
        gate.completePersistence(ticket, true, true)
        assertTrue(gate.canSkipDurableExit(cached, cached, cached, 2L))
        for (changed in listOf(
            cached.copy(contactCaption = "new contact"),
            cached.copy(callSummary = "new call"),
            cached.copy(attachments = listOf("image", "document")),
            cached.copy(protectionRevision = 5L)
        )) {
            assertEquals(cached.text, changed.text)
            assertFalse(gate.canSkipDurableExit(cached, cached, changed, 2L))
            assertFalse(gate.canSkipDurableExit(cached, changed, cached, 2L))
        }
        assertFalse(gate.canSkipDurableExit(cached, cached, cached, 3L))
        gate.beginPersistence(cached, false, 2L) {}
        assertFalse(gate.canSkipDurableExit(cached, cached, cached, 2L))
    }

}
"####;
