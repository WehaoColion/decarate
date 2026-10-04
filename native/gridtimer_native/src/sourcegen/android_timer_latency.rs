// v2.22.49.2 - Keep history rows visible during same-workspace filter refreshes.
// All Android implementation remains authored in Rust source templates.

const REPOSITORY: &str = "com/ofairyo/gridtimer/data/TimerRepository.kt";
const DATABASE: &str = "com/ofairyo/gridtimer/data/AppStateDatabase.kt";
const SCREEN: &str = "com/ofairyo/gridtimer/ui/GridTimerScreen.kt";

pub fn render(path: &str, base: &str) -> Result<String, String> {
    match path {
        REPOSITORY => repository(base),
        DATABASE => {
            let mut source = base.to_owned();
            replace(
                &mut source,
                "    fun writeSnapshot(",
                &format!("{DATABASE_WRITE_GUARD}    fun writeSnapshot("),
            )?;
            Ok(source)
        }
        SCREEN => screen(base),
        _ => Ok(base.to_owned()),
    }
}

fn replace(source: &mut String, old: &str, new: &str) -> Result<(), String> {
    if source.matches(old).count() != 1 {
        return Err(format!("timer latency anchor is not unique: {old}"));
    }
    *source = source.replacen(old, new, 1);
    Ok(())
}

fn section<'a>(source: &'a str, start: &str, end: &str) -> Result<&'a str, String> {
    let from = source
        .find(start)
        .ok_or_else(|| format!("missing {start}"))?;
    let to = source[from..]
        .find(end)
        .ok_or_else(|| format!("missing {end}"))?
        + from;
    Ok(&source[from..to])
}

fn repository(base: &str) -> Result<String, String> {
    let mut source = base.to_owned();
    for (start, end) in [
        ("    suspend fun startSlot(", "    suspend fun pauseSlot("),
        ("    suspend fun pauseSlots(", "    suspend fun resetSlot("),
    ] {
        let old = section(&source, start, end)?.to_owned();
        let mut new = old.clone();
        replace(
            &mut new,
            "        updateData { data ->",
            "        updateData(timerOnly = true) { data ->",
        )?;
        replace(&mut source, &old, &new)?;
    }
    replace(&mut source,
        "        val attempt = updateDataDetailed { data ->\n            before = data\n            val resolution = data.resolveMicroBreaks(now)",
        "        val attempt = updateDataDetailed(timerOnly = true) { data ->\n            before = _appData.value\n            val resolution = data.resolveMicroBreaks(now)")?;
    replace(&mut source,
        "    private suspend fun updateData(\n        expectedWorkspaceKey: String? = null,",
        "    private suspend fun updateData(\n        timerOnly: Boolean = false,\n        expectedWorkspaceKey: String? = null,")?;
    replace(&mut source,
        "        return updateDataDetailed(\n            expectedWorkspaceKey = expectedWorkspaceKey,",
        "        return updateDataDetailed(\n            timerOnly = timerOnly,\n            expectedWorkspaceKey = expectedWorkspaceKey,")?;
    replace(&mut source,
        "    private suspend fun updateDataDetailed(\n        expectedWorkspaceKey: String? = null,",
        "    private suspend fun updateDataDetailed(\n        timerOnly: Boolean = false,\n        expectedWorkspaceKey: String? = null,")?;
    replace(
        &mut source,
        r####"                    val transformed = transform(current)
                    if (transformed === current) {
                        current
                    } else {
                        stampChangedFieldRevisions(
                            previous = current,
                            next = transformed,
                            mutationAt = now()
                        ).sanitized().reuseUnchangedDomains(current)
                    }"####,
        r####"                    val mutationBase = if (timerOnly) current.timerMutationData() else current
                    val transformed = transform(mutationBase)
                    if (transformed === mutationBase) {
                        current
                    } else {
                        val normalized = stampChangedFieldRevisions(
                            previous = mutationBase,
                            next = transformed,
                            mutationAt = now()
                        ).sanitized()
                        if (timerOnly) current.acceptTimerMutation(normalized)
                        else normalized.reuseUnchangedDomains(current)
                    }"####,
    )?;
    replace(&mut source, "    private class RevisionOverflowException(message: String) : IllegalStateException(message)",
        &format!("{TIMER_DOMAIN}    private class RevisionOverflowException(message: String) : IllegalStateException(message)"))?;
    let old = section(
        &source,
        "    private suspend fun persistToWorkspace(",
        "    private fun repairVerifiedRecoveryCandidate(",
    )?
    .to_owned();
    replace(&mut source, &old, PERSIST_WORKSPACE)?;
    replace(
        &mut source,
        "    private fun futureSchemaVersionOnAnySurface(",
        &format!("{PREFLIGHT_CACHE}    private fun futureSchemaVersionOnAnySurface("),
    )?;
    Ok(source)
}

const TIMER_DOMAIN: &str = r####"    // Only these three operations may use this projection: start, pause, phase advance.
    // Keep tombstones to preserve deleted-session and logical revision semantics.
    // The result is NEVER written as a workspace snapshot or sent to sync.
    private fun AppData.timerMutationData(): AppData = AppData(
        schemaVersion = schemaVersion,
        categories = categories,
        slots = slots,
        slotOrder = slotOrder,
        slotOrderUpdatedAtEpochMillis = slotOrderUpdatedAtEpochMillis,
        sessions = sessions,
        tombstones = tombstones
    )

    private fun AppData.acceptTimerMutation(timer: AppData): AppData {
        val nextSlots = timer.slots.reuseIfEqual(slots)
        val nextSessions = timer.sessions.reuseIfEqual(sessions)
        return if (nextSlots === slots && nextSessions === sessions) this
        else copy(slots = nextSlots, sessions = nextSessions)
    }

"####;

const DATABASE_WRITE_GUARD: &str = r####"    data class SnapshotWriteStamp(
        val database: SQLiteDatabase,
        val dataVersion: Long,
        val totalChanges: Long
    )

    // A transaction pins the PRIMARY connection on this thread. Comparing
    // data_version across arbitrary pooled read connections would be incorrect.
    // data_version catches other connections; total_changes catches this one,
    // including imports, quarantines, account writes and rolled-back attempts.
    fun snapshotWriteStamp(): SnapshotWriteStamp {
        val database = writableDatabase
        check(database.inTransaction()) { "A write stamp requires the primary transaction." }
        fun scalar(sql: String): Long = database.rawQuery(sql, null).use { cursor ->
            check(cursor.moveToFirst())
            cursor.getLong(0)
        }
        return SnapshotWriteStamp(database, scalar("PRAGMA data_version"), scalar("SELECT total_changes()"))
    }

    @Synchronized
    fun <T> withSnapshotWriteTransaction(action: () -> T): T {
        val database = writableDatabase
        database.beginTransactionNonExclusive()
        try {
            val result = action()
            database.setTransactionSuccessful()
            return result
        } finally {
            database.endTransaction()
        }
    }

"####;

const PREFLIGHT_CACHE: &str = r####"    private data class PersistFileStamp(
        val identity: String?, val size: Long, val modifiedNanos: Long, val createdNanos: Long
    )
    private data class PersistPreflightStamp(
        val database: AppStateDatabase.SnapshotWriteStamp,
        val primary: PersistFileStamp,
        val backup: PersistFileStamp
    )
    // Accessed only by the repository's serialized workspace writes. Keep a single
    // workspace entry so switching accounts always revalidates the destination.
    private var verifiedWritePreflight: Pair<String, PersistPreflightStamp>? = null

    private fun persistFileStamp(file: File): PersistFileStamp {
        return try {
            val attributes = java.nio.file.Files.readAttributes(
                file.toPath(), java.nio.file.attribute.BasicFileAttributes::class.java
            )
            PersistFileStamp(
                attributes.fileKey()?.toString(), attributes.size(),
                attributes.lastModifiedTime().to(java.util.concurrent.TimeUnit.NANOSECONDS),
                attributes.creationTime().to(java.util.concurrent.TimeUnit.NANOSECONDS)
            )
        } catch (_: java.nio.file.NoSuchFileException) {
            PersistFileStamp(null, -1L, -1L, -1L)
        }
    }

"####;

const PERSIST_WORKSPACE: &str = r####"    private suspend fun persistToWorkspace(workspaceKey: String, appData: AppData) {
        withContext(Dispatchers.IO) {
            val files = workspaceFiles(workspaceKey)
            val cached = verifiedWritePreflight
            verifiedWritePreflight = null
            val encoded = strictPersistedJson.encodeToString(
                appData.copy(schemaVersion = APP_DATA_SCHEMA_VERSION)
            )
            val encodedCandidate = checkNotNull(decodePersistedCandidate("pending-write", encoded)) {
                "Refusing to persist an app-data snapshot that cannot be decoded."
            }
            val committedStamp = stateDatabase.withSnapshotWriteTransaction {
                val stamp = PersistPreflightStamp(
                    stateDatabase.snapshotWriteStamp(), persistFileStamp(files.state), persistFileStamp(files.backup)
                )
                if (cached?.first != workspaceKey || cached?.second != stamp) {
                    val futureSchema = futureSchemaVersionOnAnySurface(workspaceKey)
                    check(futureSchema == null) {
                        "Refusing to overwrite persisted evidence created by future schema $futureSchema."
                    }
                }
                // Preflight and the authoritative write share one primary transaction;
                // another database writer cannot change the evidence between them.
                stateDatabase.writeSnapshot(
                    workspaceKey = workspaceKey,
                    appDataJson = encoded,
                    schemaVersion = APP_DATA_SCHEMA_VERSION,
                    revision = encodedCandidate.revision,
                    itemCount = encodedCandidate.itemCount,
                    now = now()
                )
                stateDatabase.snapshotWriteStamp()
            }
            // Only publish the cache after commit AND successful recovery mirrors.
            // Any failure leaves it invalid, so the next attempt checks every surface.
            runCatching {
                writeWorkspaceInitializationEvidence(workspaceKey)
                val primary = readPersistedCandidate(files.state)
                val backup = readPersistedCandidate(files.backup)
                if (primary != null) {
                    val backupWinner = chooseSaferPersistedCandidate(primary, backup)
                    if (backupWinner === primary && backup?.raw != primary.raw) {
                        AtomicFileStore.writeText(files.backup, primary.raw)
                    }
                }
                AtomicFileStore.writeText(files.state, encoded)
                if (readPersistedCandidate(files.backup) == null) {
                    AtomicFileStore.writeText(files.backup, encoded)
                }
                verifiedWritePreflight = workspaceKey to PersistPreflightStamp(
                    committedStamp, persistFileStamp(files.state), persistFileStamp(files.backup)
                )
            }.onFailure { throwable ->
                Log.w(TAG, "SQLite committed but a recovery mirror refresh failed.", throwable)
                logDiagnosticEvent(
                    category = "repository.persist.recovery_mirror",
                    message = "Primary SQLite snapshot committed; recovery mirror refresh is deferred.",
                    throwable = throwable
                )
            }
        }
    }

"####;

fn screen(base: &str) -> Result<String, String> {
    let mut source = base.to_owned();
    replace(&mut source,
        "                                    topDockPadding = appTopPadding,",
        "                                    tickEnabled = !detailOverlayVisible && !historyVisible,\n                                    topDockPadding = appTopPadding,")?;
    replace(&mut source,
        "private fun TimerHomeScreen(\n    appData: AppData,\n    uiIndex: AppDataUiIndex,",
        "private fun TimerHomeScreen(\n    appData: AppData,\n    uiIndex: AppDataUiIndex,\n    tickEnabled: Boolean,")?;
    let old = section(
        &source,
        "private fun TimerHomeScreen(",
        "private fun TimerTile(",
    )?
    .to_owned();
    let mut new = old.clone();
    replace(&mut new, "    val summaryNow = rememberCurrentTimeMillis(tickMillis = 60_000L)",
        "    val summaryNow = rememberCurrentTimeMillis(tickMillis = 60_000L, enabled = tickEnabled)")?;
    replace(&mut new, "                    slot = slot,\n                    displayIndex = index + 1,",
        "                    slot = slot,\n                    tickEnabled = tickEnabled,\n                    displayIndex = index + 1,")?;
    replace(&mut new, "                slot = draggedSlot,\n                displayIndex = draggedDisplayIndex,",
        "                slot = draggedSlot,\n                tickEnabled = tickEnabled,\n                displayIndex = draggedDisplayIndex,")?;
    replace(&mut source, &old, &new)?;
    replace(&mut source, "private fun TimerTile(\n    modifier: Modifier = Modifier,\n    slot: TimerSlot,",
        "private fun TimerTile(\n    modifier: Modifier = Modifier,\n    slot: TimerSlot,\n    tickEnabled: Boolean = true,")?;
    replace(&mut source, "    val isHiddenPlaceholder = visualAlpha <= 0.01f\n    val elapsedNow = rememberCurrentTimeMillis(tickMillis = 1_000L, enabled = isRunning)",
        "    val isHiddenPlaceholder = visualAlpha <= 0.01f\n    val elapsedNow = rememberCurrentTimeMillis(tickMillis = 1_000L, enabled = isRunning && tickEnabled && !isHiddenPlaceholder)")?;
    let old = section(
        &source,
        "    val latestSession = remember(sessions) {",
        "    val configuration = LocalConfiguration.current",
    )?
    .to_owned();
    replace(&mut source, &old, DETAIL_STATS)?;
    let old = section(
        &source,
        "    val archivedSearchIndex = remember(appData.archivedTasks, appData.categories)",
        "    val filteredSessions = filteredSessionHistory.sessions",
    )?
    .to_owned();
    replace(&mut source, &old, HISTORY_SEARCH)?;
    replace(&mut source, "                HistorySheet(\n                    appData = appData,",
        "                HistorySheet(\n                    workspaceKey = activeWorkspaceKey,\n                    appData = appData,")?;
    replace(
        &mut source,
        "private fun HistorySheet(\n    appData: AppData,",
        "private fun HistorySheet(\n    workspaceKey: String,\n    appData: AppData,",
    )?;
    replace(&mut source, "if (filteredSessions.isEmpty()) {",
        "if (historyLoading) {\n                item(key = \"history_computing\") {\n                    androidx.compose.material3.LinearProgressIndicator(modifier = Modifier.fillMaxWidth())\n                }\n            } else if (filteredSessions.isEmpty()) {")?;
    source.push_str(ASYNC_COMPUTATION);
    Ok(source)
}

const DETAIL_STATS: &str = r####"    val detailRequest = remember(TimerReferenceKey(sessions), currentDayWindow) { Any() }
    val detailStats = rememberTimerComputation(detailRequest, EmptyTimerDetailStats) {
        TimerDetailStats(
            sessions.maxByOrNull(TimerSession::endedAtEpochMillis),
            sessions.maxByOrNull(TimerSession::durationMillis),
            timerSessionWindowSummary(sessions, currentDayWindow.first, currentDayWindow.second),
            sessions.sortedByDescending(TimerSession::endedAtEpochMillis).take(20)
        )
    }
    val latestSession = detailStats.latest
    val longestSession = detailStats.longest
    val todaySummary = detailStats.today
    val todayDuration = todaySummary.totalDurationMillis
    val todaySessionCount = todaySummary.sessionCount
    val recentSessions = detailStats.recent
"####;

const HISTORY_SEARCH: &str = r####"    val historyWorkspaceScope = remember(workspaceKey) { Any() }
    val historyFilterScope = remember(historyWorkspaceScope, filterCategoryId, filterSlotId, trimmedSearchQuery) { Any() }
    val archivedIndexRequest = remember(TimerReferenceKey(appData.archivedTasks), TimerReferenceKey(appData.categories), TimerReferenceKey(uiIndex)) { Any() }
    val sessionIndexRequest = remember(TimerReferenceKey(appData.sessions), TimerReferenceKey(appData.categories), TimerReferenceKey(uiIndex)) { Any() }
    val archivedSearchIndex = rememberTimerComputation(archivedIndexRequest, EmptyArchivedSearchIndex, retentionScope = historyWorkspaceScope) {
        buildArchivedHistorySearchIndex(appData.archivedTasks, uiIndex)
    }
    val sessionSearchIndex = rememberTimerComputation(sessionIndexRequest, EmptySessionSearchIndex, retentionScope = historyWorkspaceScope) {
        buildSessionHistorySearchIndex(appData.sessions, uiIndex)
    }
    val archivedFilterRequest = remember(TimerReferenceKey(archivedSearchIndex), filterCategoryId, filterSlotId, trimmedSearchQuery) { Any() }
    val sessionFilterRequest = remember(TimerReferenceKey(sessionSearchIndex), filterCategoryId, filterSlotId, trimmedSearchQuery, summaryNow) { Any() }
    val filteredArchivedTasks = rememberTimerComputation(
        archivedFilterRequest, EmptyFilteredArchives, if (trimmedSearchQuery.isEmpty()) 0L else 120L,
        retentionScope = historyFilterScope
    ) {
        filterArchivedHistory(archivedSearchIndex, trimmedSearchQuery, filterCategoryId, filterSlotId)
    }
    val filteredSessionHistory = rememberTimerComputation(
        sessionFilterRequest, EmptyFilteredSessionHistory, if (trimmedSearchQuery.isEmpty()) 0L else 120L,
        retentionScope = historyFilterScope
    ) {
        filterSessionHistory(sessionSearchIndex, trimmedSearchQuery, filterCategoryId, filterSlotId, summaryNow)
    }
    val historyLoading = archivedSearchIndex === EmptyArchivedSearchIndex ||
        sessionSearchIndex === EmptySessionSearchIndex || filteredArchivedTasks === EmptyFilteredArchives ||
        filteredSessionHistory === EmptyFilteredSessionHistory
"####;

const ASYNC_COMPUTATION: &str = r####"

private class TimerReferenceKey(private val value: Any) {
    override fun equals(other: Any?): Boolean = other is TimerReferenceKey && value === other.value
    override fun hashCode(): Int = System.identityHashCode(value)
}

private class TimerComputed<T>(val scope: Any, val result: T)

@Composable
private fun <T> rememberTimerComputation(
    request: Any, empty: T, debounceMillis: Long = 0L,
    retentionScope: Any = request, compute: () -> T
): T {
    val result = produceState<TimerComputed<T>?>(initialValue = null, key1 = request, key2 = retentionScope) {
        if (debounceMillis > 0L) delay(debounceMillis)
        value = withContext(Dispatchers.Default) { TimerComputed(retentionScope, compute()) }
    }.value
    // Refreshes keep existing lazy-list keys; a new filter or account clears them immediately.
    return if (result?.scope === retentionScope) result.result else empty
}

private data class TimerDetailStats(
    val latest: TimerSession?, val longest: TimerSession?,
    val today: TimerSessionWindowSummary, val recent: List<TimerSession>
)
private val EmptyTimerDetailStats = TimerDetailStats(
    null, null, TimerSessionWindowSummary(0L, 0, null, 0L, null, null), emptyList()
)
private val EmptyArchivedSearchIndex = HistoryArchivedSearchIndex(emptyList(), emptyArray(), intArrayOf(), emptyArray())
private val EmptySessionSearchIndex = HistorySessionSearchIndex(emptyList(), emptyArray(), intArrayOf(), longArrayOf(), longArrayOf(), emptyArray())
private val EmptyFilteredSessionHistory = FilteredSessionHistory(emptyList(), 0L, 0L)
private val EmptyFilteredArchives: List<ArchivedTask> = ArrayList(0)
"####;

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    pub(crate) fn rendered(path: &str) -> String {
        let base = super::super::kotlin_sources::SOURCES
            .iter()
            .find(|s| s.path == path)
            .unwrap()
            .contents;
        let source = super::super::android_performance_override::render(path, base).unwrap();
        let source = super::super::android_responsiveness::render(path, &source).unwrap();
        let source = super::super::diagnostics_export_backend::render(path, &source).unwrap();
        let source = super::super::diagnostics_collection_backend::render(path, &source).unwrap();
        let source = super::super::diagnostics_export_ui::render(path, &source).unwrap();
        let source = super::super::android_snapshot_memory::render(path, &source).unwrap();
        let source = super::super::android_backup_availability::render(path, &source).unwrap();
        render(path, &source).unwrap()
    }

    #[test]
    fn final_android_pipeline_keeps_durable_commit_and_scopes_only_timer_actions() {
        let source = rendered(REPOSITORY);
        assert_eq!(source.matches("updateData(timerOnly = true)").count(), 2);
        assert_eq!(
            source
                .matches("updateDataDetailed(timerOnly = true)")
                .count(),
            1
        );
        let save = section(
            &source,
            "    private suspend fun persistToWorkspace(",
            "    private fun repairVerifiedRecoveryCandidate(",
        )
        .unwrap();
        assert!(
            save.find("withSnapshotWriteTransaction").unwrap()
                < save.find("futureSchemaVersionOnAnySurface").unwrap()
        );
        assert!(
            save.find("stateDatabase.writeSnapshot").unwrap()
                < save.find("AtomicFileStore.writeText(files.state").unwrap()
        );
        assert!(save.contains("verifiedWritePreflight = null"));
        assert!(
            source.contains("if (!attempt.succeeded || attempt.snapshot === before) return false")
        );
        assert!(TIMER_DOMAIN.contains("else copy(slots = nextSlots, sessions = nextSessions)"));
    }

    #[test]
    fn final_android_database_keeps_recovery_and_checks_current_before_rotation() {
        let source = rendered(DATABASE);
        assert!(source.contains("quarantineDigestMismatches(workspaceKey, includeHistory = true)"));
        assert!(source.contains("SELECT total_changes()"));
        assert!(source.contains("PRAGMA data_version"));
        assert!(source.contains("check(database.inTransaction())"));
        assert!(source
            .contains("quarantineUnverifiedCurrentInTransaction(database, workspaceKey, now)"));
    }

    #[test]
    fn final_android_history_does_not_compute_records_in_composition() {
        let source = rendered(SCREEN);
        assert!(!source.contains("val recentSessions = remember(sessions)"));
        assert!(!source.contains("val sessionSearchIndex = remember(appData.sessions"));
        assert!(source.contains(
            "withContext(Dispatchers.Default) { TimerComputed(retentionScope, compute()) }"
        ));
        assert!(source.contains("result?.scope === retentionScope"));
        assert!(source.contains("tickEnabled = !detailOverlayVisible && !historyVisible"));
        assert!(source.contains("enabled = isRunning && tickEnabled && !isHiddenPlaceholder"));
    }
}
