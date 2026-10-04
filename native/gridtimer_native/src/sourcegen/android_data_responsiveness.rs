// v2.22.30 - Keep parsing, publishing and history indexing off the UI thread.

const REPOSITORY_PATH: &str = "com/ofairyo/gridtimer/data/TimerRepository.kt";
const INDEX_PATH: &str = "com/ofairyo/gridtimer/ui/UiIndexes.kt";
const LOG_PATH: &str = "com/ofairyo/gridtimer/diagnostics/DiagnosticLogStore.kt";
const CRASH_PATH: &str = "com/ofairyo/gridtimer/diagnostics/CrashLogStore.kt";

pub fn render(path: &str, base: &str) -> Result<String, String> {
    match path {
        REPOSITORY_PATH => render_repository(base),
        INDEX_PATH => render_index(base),
        LOG_PATH => render_diagnostic_log(base),
        CRASH_PATH => render_crash_log(base),
        _ => Ok(base.to_owned()),
    }
}

fn replace(target: &mut String, old: &str, new: &str, label: &str) -> Result<(), String> {
    let matches = target.match_indices(old).count();
    if matches != 1 {
        return Err(format!("expected one {label} fragment, found {matches}"));
    }
    *target = target.replacen(old, new, 1);
    Ok(())
}

fn render_repository(base: &str) -> Result<String, String> {
    let mut rendered = base.to_owned();
    replace(
        &mut rendered,
        "    private val _syncSession = MutableStateFlow(loadSyncSession())",
        "    private val _syncSession = MutableStateFlow(SyncAccountSession())\n    @Volatile\n    private var accountInitializationFailed = false",
        "non-blocking account initialization",
    )?;
    replace(
        &mut rendered,
        "    private val initializationJob = scope.launch {\n        runCatching {\n            loadFromDisk()",
        "    private val initializationJob = scope.launch {\n        var sessionRestored = false\n        runCatching {\n            val loadedSession = loadSyncSession()\n            activeWorkspaceKey = workspaceKeyForSession(loadedSession)\n            _syncSession.value = loadedSession\n            sessionRestored = true\n            loadFromDisk()",
        "IO account initialization before workspace restore",
    )?;
    replace(
        &mut rendered,
        "        }.onFailure { throwable ->\n            Log.e(TAG, \"Failed to initialize timer state; falling back to defaults.\", throwable)",
        "        }.onFailure { throwable ->\n            if (!sessionRestored) {\n                accountInitializationFailed = true\n                val message = \"账号状态读取失败，请重新打开应用\"\n                persistenceWriteBlockedReason = message\n                _syncSession.value = _syncSession.value.copy(lastMessage = message)\n            }\n            Log.e(TAG, \"Failed to initialize timer state; falling back to defaults.\", throwable)",
        "account restore failure cannot select or overwrite guest state",
    )?;
    replace(
        &mut rendered,
        "    private fun persistSyncSession(session: SyncAccountSession): Boolean {\n        val persisted = SecureSyncSessionStore.persist(appContext, json, session)",
        "    private fun persistSyncSession(session: SyncAccountSession): Boolean {\n        if (accountInitializationFailed) return false\n        val persisted = SecureSyncSessionStore.persist(appContext, json, session)",
        "unrestored credentials cannot be overwritten",
    )?;
    for signature in [
        "private suspend fun authenticateSyncAccount(email: String, password: String, register: Boolean)",
        "private suspend fun logoutSyncAccountLocked()",
    ] {
        let old = format!("    {signature} {{\n        awaitInitialized()\n");
        let new = format!("    {signature} {{\n        awaitInitialized()\n        if (accountInitializationFailed) return\n");
        replace(&mut rendered, &old, &new, "unrestored account mutation guard")?;
    }
    replace(
        &mut rendered,
        "    ): NoteDraftJournalCheckpoint? {\n        if (workspaceKey != activeWorkspaceKey) return null",
        "    ): NoteDraftJournalCheckpoint? {\n        if (!initializationJob.isCompleted || accountInitializationFailed) return null\n        if (workspaceKey != activeWorkspaceKey) return null",
        "draft journaling waits for a verified workspace",
    )?;
    for signature in [
        "updateSyncServerUrl(serverUrl: String)",
        "updateSyncEmail(email: String)",
        "updateSyncDeviceName(deviceName: String)",
        "updateAiApiKey(apiKey: String)",
        "updateAiBaseUrl(baseUrl: String)",
        "updateAiModel(model: String)",
        "updateAiLastMessage(message: String)",
    ] {
        let old = format!("    suspend fun {signature} {{\n");
        let new = format!("    suspend fun {signature} {{\n        awaitInitialized()\n");
        replace(
            &mut rendered,
            &old,
            &new,
            "account setting initialization guard",
        )?;
    }
    replace(
        &mut rendered,
        r####"                withContext(Dispatchers.Default) {
                    stampChangedFieldRevisions(
                        previous = current,
                        next = transform(current),
                        mutationAt = now()
                    ).sanitized()
                }"####,
        r####"                withContext(Dispatchers.Default) {
                    val transformed = transform(current)
                    if (transformed === current) {
                        current
                    } else {
                        stampChangedFieldRevisions(
                            previous = current,
                            next = transformed,
                            mutationAt = now()
                        ).sanitized().reuseUnchangedDomains(current)
                    }
                }"####,
        "skip no-op serialization and reuse unchanged domains",
    )?;
    // StateFlow performs another deep equality check itself. Keep that publication
    // inside the existing write lock, but execute it on Default as well.
    let start = rendered
        .find("    private suspend fun updateDataDetailed(")
        .ok_or("missing updateDataDetailed")?;
    let end = rendered[start..]
        .find("    private class RevisionOverflowException")
        .map(|offset| start + offset)
        .ok_or("missing revision exception boundary")?;
    let section = &rendered[start..end];
    if section.matches("_appData.value = updated").count() != 2 {
        return Err("expected two durable/non-durable publication paths".into());
    }
    let section = section.replace(
        "_appData.value = updated",
        "withContext(Dispatchers.Default) { _appData.value = updated }",
    );
    let section = protect_commit_cancellation(&section)?;
    rendered.replace_range(start..end, &section);
    replace(
        &mut rendered,
        "    private class RevisionOverflowException(message: String) : IllegalStateException(message)",
        r####"    // Sanitization remains authoritative. Reuse a domain only after full
    // equality, so every field/revision survives while Compose can skip untouched
    // history and notes after a timer or theme mutation.
    private fun <T> T.reuseIfEqual(previous: T): T = if (this == previous) previous else this

    private fun AppData.reuseUnchangedDomains(previous: AppData): AppData = copy(
        categories = categories.reuseIfEqual(previous.categories),
        slots = slots.reuseIfEqual(previous.slots),
        slotOrder = slotOrder.reuseIfEqual(previous.slotOrder),
        sessions = sessions.reuseIfEqual(previous.sessions),
        archivedTasks = archivedTasks.reuseIfEqual(previous.archivedTasks),
        noteFolders = noteFolders.reuseIfEqual(previous.noteFolders),
        notes = notes.reuseIfEqual(previous.notes),
        notePreferences = notePreferences.reuseIfEqual(previous.notePreferences),
        financeProfile = financeProfile.reuseIfEqual(previous.financeProfile),
        financeDayLedgerRevisions = financeDayLedgerRevisions.reuseIfEqual(previous.financeDayLedgerRevisions),
        financeMonthSnapshotRevisions = financeMonthSnapshotRevisions.reuseIfEqual(previous.financeMonthSnapshotRevisions),
        tombstones = tombstones.reuseIfEqual(previous.tombstones),
        syncConflictHistory = syncConflictHistory.reuseIfEqual(previous.syncConflictHistory)
    )

    private class RevisionOverflowException(message: String) : IllegalStateException(message)"####,
        "immutable domain reference reuse",
    )?;
    replace(
        &mut rendered,
        r####"                advanceMicroBreaks(now())
                current = _appData.value"####,
        r####"                val progressed = advanceMicroBreaks(now())
                current = _appData.value
                // A read-only workspace, failed transform or failed publication
                // must not spin at a zero deadline and saturate CPU/diagnostics.
                if (!progressed) delay(1_000L)"####,
        "micro-break failed advance backoff",
    )?;
    replace(
        &mut rendered,
        r####"        source: String = "monitor"
    ) {
        var freshTransitions = emptyList<MicroBreakTransition>()
        updateData { data ->"####,
        r####"        source: String = "monitor"
    ): Boolean {
        var freshTransitions = emptyList<MicroBreakTransition>()
        var before: AppData? = null
        val attempt = updateDataDetailed { data ->
            before = data"####,
        "observe committed micro-break update result",
    )?;
    replace(
        &mut rendered,
        "        freshTransitions.forEach { transition ->\n",
        "        if (!attempt.succeeded || attempt.snapshot === before) return false\n        freshTransitions.forEach { transition ->\n",
        "alert only successful micro-break changes",
    )?;
    replace(
        &mut rendered,
        "        syncTimerLiveUpdate()\n    }\n\n    private fun classifyPersistenceFailure",
        "        syncTimerLiveUpdate()\n        return true\n    }\n\n    private fun classifyPersistenceFailure",
        "micro-break successful progress result",
    )?;
    replace(
        &mut rendered,
        r####"        if (expectedWorkspaceKey != null) {
            return withContext(NonCancellable) {
                updateDataDetailed(
                    expectedWorkspaceKey = expectedWorkspaceKey,
                    persistImmediately = true
                ) { data -> data }.succeeded
            }
        }
        return persistNow(reason)
    }

    fun flushNowBlocking"####,
        r####"        try {
            if (expectedWorkspaceKey != null) {
                return withContext(NonCancellable) {
                    updateDataDetailed(
                        expectedWorkspaceKey = expectedWorkspaceKey,
                        persistImmediately = true
                    ) { data -> data }.succeeded
                }
            }
            return persistNow(reason)
        } finally {
            withContext(NonCancellable + Dispatchers.IO) {
                DiagnosticLogStore.flushPendingWrites()
            }
        }
    }

    fun flushNowBlocking"####,
        "lifecycle diagnostic flush after durable timer flush",
    )?;
    Ok(rendered)
}

fn protect_commit_cancellation(section: &str) -> Result<String, String> {
    let commit_start = section
        .find("            if (!updatedChanged) {")
        .ok_or("missing cancellable update preparation boundary")?;
    let finish_boundary = "        }\n        if (didChange) {";
    let commit_end = section[commit_start..]
        .find(finish_boundary)
        .map(|offset| commit_start + offset)
        .ok_or("missing update commit completion boundary")?;
    let finish_start = commit_end + "        }\n".len();
    let finish_end = section[finish_start..]
        .find("        return attempt\n    }")
        .map(|offset| finish_start + offset)
        .ok_or("missing update completion return")?;
    let commit = &section[commit_start..commit_end];
    if commit.matches("return@withLock").count() != 2 {
        return Err("expected unchanged and failed-durable commit exits".into());
    }
    let commit = commit.replace("return@withLock", "return@commit");
    let finish = &section[finish_start..finish_end];
    let mut protected = String::from(concat!(
        "            // A dispatcher hop after publication must not cancel between\n",
        "            // the visible state and its dirty marker or scheduled persistence.\n",
        "            // Keep preparation cancellable; finish an accepted commit on the\n",
        "            // caller's dispatcher under NonCancellable and the same write lock.\n",
    ));
    // These lines are emitted explicitly so the NonCancellable layer never also
    // switches dispatchers. Its inner Default publication returns to this layer.
    protected
        .push_str("            withContext(NonCancellable) commit@ {\n                try {\n");
    for line in commit.lines() {
        protected.push_str("        ");
        protected.push_str(line);
        protected.push('\n');
    }
    protected.push_str("                } finally {\n");
    for line in finish.lines() {
        protected.push_str("            ");
        protected.push_str(line);
        protected.push('\n');
    }
    protected.push_str("                }\n            }\n        }\n");
    let mut result = section.to_owned();
    result.replace_range(commit_start..finish_end, &protected);
    Ok(result)
}

fn render_crash_log(base: &str) -> Result<String, String> {
    let mut rendered = base.to_owned();
    replace(
        &mut rendered,
        "            throwable = throwable\n        )\n        val content = buildString {",
        "            throwable = throwable\n        )\n        DiagnosticLogStore.flushPendingWrites()\n        val content = buildString {",
        "bounded crash diagnostic flush",
    )?;
    Ok(rendered)
}

fn render_index(base: &str) -> Result<String, String> {
    let mut rendered = base.to_owned();
    replace(
        &mut rendered,
        "import androidx.compose.runtime.remember",
        "import androidx.compose.runtime.remember\nimport androidx.compose.runtime.produceState\nimport kotlinx.coroutines.Dispatchers\nimport kotlinx.coroutines.withContext",
        "background UI history index imports",
    )?;
    replace(
        &mut rendered,
        r####"@Composable
internal fun rememberAppDataUiIndex(appData: AppData): AppDataUiIndex {
    return remember(appData) {
        buildAppDataUiIndex(appData)
    }
}"####,
        r####"// Identity keys avoid a full history equals/hashCode scan on the UI thread.
// AppData is immutable; TimerRepository retains equal domains after sanitization.
private class UiIndexIdentityKey(private val value: Any) {
    override fun equals(other: Any?): Boolean = other is UiIndexIdentityKey && value === other.value
    override fun hashCode(): Int = System.identityHashCode(value)
}

private val EmptyAppDataUiIndex = AppDataUiIndex(
    categoriesById = emptyMap(), sessionsBySlotId = emptyMap(),
    latestSessionBySlotId = emptyMap(), sessionCountBySlotId = emptyMap(),
    archivedCountBySlotId = emptyMap(), latestSession = null, peakSession = null
)

private data class IndexedAppData(val request: Any, val index: AppDataUiIndex)

@Composable
internal fun rememberAppDataUiIndex(appData: AppData): AppDataUiIndex {
    val slotIds = appData.slots.map { it.id }
    val request = remember(
        UiIndexIdentityKey(appData.categories), UiIndexIdentityKey(appData.sessions),
        UiIndexIdentityKey(appData.archivedTasks), slotIds
    ) { Any() }
    val indexed = produceState<IndexedAppData?>(initialValue = null, key1 = request) {
        value = withContext(Dispatchers.Default) {
            IndexedAppData(request, buildAppDataUiIndex(appData))
        }
    }.value
    // Never show an index from a previous workspace while a new request completes.
    return if (indexed?.request === request) indexed.index else EmptyAppDataUiIndex
}"####,
        "domain-cached background history index",
    )?;
    Ok(rendered)
}

fn render_diagnostic_log(base: &str) -> Result<String, String> {
    let mut rendered = base.to_string();

    replace(
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
    private const val FLUSH_TIMEOUT_MILLIS = 1_000L
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

    replace(
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

    replace(
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

    replace(
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

    replace(
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

    replace(
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

    internal fun flushPendingWrites() {
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

    replace(
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
    fn source(path: &str) -> &'static str {
        crate::kotlin_sources::SOURCES
            .iter()
            .find(|source| source.path == path)
            .unwrap()
            .contents
    }

    #[test]
    fn account_restore_precedes_disk_workspace_selection() {
        let rendered = crate::android_performance_override::render(
            super::REPOSITORY_PATH,
            source(super::REPOSITORY_PATH),
        )
        .unwrap();
        assert!(!rendered.contains("MutableStateFlow(loadSyncSession())"));
        let init = rendered
            .split("private val initializationJob = scope.launch {")
            .nth(1)
            .unwrap();
        assert!(
            init.find("activeWorkspaceKey = workspaceKeyForSession(loadedSession)")
                .unwrap()
                < init.find("loadFromDisk()").unwrap()
        );
        assert!(rendered
            .contains("suspend fun updateSyncEmail(email: String) {\n        awaitInitialized()"));
        assert!(rendered.contains(
            "if (!sessionRestored) {\n                accountInitializationFailed = true"
        ));
        assert!(rendered.contains("if (accountInitializationFailed) return false\n        val persisted = SecureSyncSessionStore.persist"));
    }

    #[test]
    fn no_op_updates_skip_sanitization_and_failed_microbreaks_back_off() {
        let rendered = crate::android_performance_override::render(
            super::REPOSITORY_PATH,
            source(super::REPOSITORY_PATH),
        )
        .unwrap();
        assert!(rendered.contains("if (transformed === current) {\n                        current\n                    } else {"));
        assert_eq!(
            rendered
                .matches("withContext(Dispatchers.Default) { _appData.value = updated }")
                .count(),
            2
        );
        assert!(rendered.contains("if (!progressed) delay(1_000L)"));
        assert!(rendered
            .contains("if (!attempt.succeeded || attempt.snapshot === before) return false"));
        assert!(rendered.contains("\"Durable-first publication requires immediate persistence.\""));
    }

    #[test]
    fn publication_metadata_and_persistence_finish_in_one_cancellation_shield() {
        let rendered = crate::android_performance_override::render(
            super::REPOSITORY_PATH,
            source(super::REPOSITORY_PATH),
        )
        .unwrap();
        let update = rendered
            .split("    private suspend fun updateDataDetailed(")
            .nth(1)
            .unwrap()
            .split("    // Sanitization remains authoritative.")
            .next()
            .unwrap();
        let shield_start = update
            .find("withContext(NonCancellable) commit@ {")
            .unwrap();
        let preparation = &update[..shield_start];
        assert!(preparation.contains("writeMutex.withLock {"));
        assert!(preparation.contains("val updatedChanged = withContext(Dispatchers.Default)"));
        assert!(!preparation.contains("withContext(NonCancellable)"));
        let shield = &update[shield_start..];
        assert_eq!(
            shield
                .matches("withContext(Dispatchers.Default) { _appData.value = updated }")
                .count(),
            2
        );
        assert_eq!(shield.matches("return@commit").count(), 2);
        assert!(!shield.contains("return@withLock"));
        let finally = shield.find("} finally {").unwrap();
        assert!(shield.find("dirtySnapshot = updated").unwrap() < finally);
        assert!(shield.find("lastPersistedSnapshot = updated").unwrap() < finally);
        assert!(shield.find("schedulePersist()\n").unwrap() > finally);
        assert!(shield.find("schedulePersistRetry()\n").unwrap() > finally);
        assert!(
            shield
                .find("updatedSnapshot?.let(::syncMicroBreakAlarm)")
                .unwrap()
                > finally
        );
        // Closing the write lock happens only after the protected finalization.
        assert!(
            shield.contains("                }\n            }\n        }\n        return attempt")
        );
    }

    #[test]
    fn history_index_is_background_and_cannot_reuse_a_previous_request() {
        let rendered = super::render(super::INDEX_PATH, source(super::INDEX_PATH)).unwrap();
        assert!(rendered.contains("value = withContext(Dispatchers.Default) {\n            IndexedAppData(request, buildAppDataUiIndex(appData))"));
        assert!(rendered.contains("value === other.value"));
        assert!(rendered
            .contains("if (indexed?.request === request) indexed.index else EmptyAppDataUiIndex"));
        assert!(!rendered.contains("remember(appData)"));
    }

    #[test]
    fn ordinary_logging_does_not_write_or_wait_and_export_flushes() {
        let rendered = super::render(super::LOG_PATH, source(super::LOG_PATH)).unwrap();
        let record = rendered
            .split("    fun record(")
            .nth(1)
            .unwrap()
            .split("    fun readRecentEntries(")
            .next()
            .unwrap();
        assert!(record.contains("pendingEntries.trySend("));
        assert!(!record.contains("appendEntry("));
        assert!(!record.contains("runBlocking"));
        assert!(rendered.contains("capacity = MAX_PENDING_LOG_WRITES"));
        assert!(rendered
            .contains("fun snapshotFile(context: Context): File? {\n        flushPendingWrites()"));
        let crash = super::render(super::CRASH_PATH, source(super::CRASH_PATH)).unwrap();
        assert!(crash.contains(
            "DiagnosticLogStore.flushPendingWrites()\n        val content = buildString"
        ));
    }
}
