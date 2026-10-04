// v2.22.45 - Reuse verified startup state and keep global media pruning off launch.
const REPOSITORY: &str = "com/ofairyo/gridtimer/data/TimerRepository.kt";
const DATABASE: &str = "com/ofairyo/gridtimer/data/AppStateDatabase.kt";

fn replace(source: &mut String, old: &str, new: &str) -> Result<(), String> {
    if source.matches(old).count() != 1 {
        return Err(format!("local startup anchor is not unique: {old}"));
    }
    *source = source.replacen(old, new, 1);
    Ok(())
}

pub fn render(path: &str, base: &str) -> Result<String, String> {
    let mut source = base.to_owned();
    match path {
        REPOSITORY => {
            replace(&mut source,
                "    private data class WorkspaceReadResult(\n        val data: AppData?,",
                "    private data class WorkspaceReadResult(\n        val data: AppData?,\n        val verifiedCurrentSnapshot: AppData? = null,")?;
            replace(&mut source,
                "    private fun loadPersistedDataForWorkspace(workspaceKey: String): WorkspaceReadResult {",
                WORKSPACE_READ)?;
            replace(&mut source,
                "            data = selected.data,\n            futureSchemaVersion = detectedFutureSchemaVersion,",
                "            data = selected.data,\n            // Only an authoritative current with healthy recovery mirrors can skip\n            // a save. Missing, stale or unreadable mirrors still get repaired.\n            verifiedCurrentSnapshot = selected.data.takeIf {\n                selected === databaseCurrent && detectedFutureSchemaVersion == null &&\n                    !recoveryOnly && blockedReason == null &&\n                    primary?.raw == selected.raw && backup != null\n            },\n            futureSchemaVersion = detectedFutureSchemaVersion,")?;
            replace(&mut source,
                "        persistenceWriteBlockedReason = result.persistenceWriteBlockedReason\n        return result.data",
                "        persistenceWriteBlockedReason = result.persistenceWriteBlockedReason\n        lastPersistedSnapshot = result.verifiedCurrentSnapshot\n        return result.data")?;
            // Keep draft/import recovery, required commits, journal cleanup, alarm
            // restoration and the ready gate. Deletion/import paths still prune.
            replace(&mut source,
                "                pruneNoteMedia(data)\n            }\n            scheduleEncryptedSnapshotRedactionScan(activeWorkspaceKey, data)",
                "                // Global retained-media pruning belongs to media mutations.\n                // Reading local state must not scan every account's entire history.\n            }\n            scheduleEncryptedSnapshotRedactionScan(activeWorkspaceKey, data)")?;
        }
        DATABASE => {
            replace(
                &mut source,
                "    @Synchronized\n    private fun quarantineDigestMismatches(",
                &format!(
                    "{READ_WINDOW}    @Synchronized\n    private fun quarantineDigestMismatches("
                ),
            )?;
            replace(&mut source,
                "        try {\n            quarantineDigestMismatchesInTransaction(\n                database = database,\n                workspaceKey = workspaceKey,\n                now = now,\n                includeHistory = includeHistory\n            )\n            database.setTransactionSuccessful()",
                "        try {\n            val window = snapshotReadWindow.get()\n            if (window == null || !window.covers(workspaceKey, includeHistory, snapshotWriteStamp())) {\n                quarantineDigestMismatchesInTransaction(\n                    database = database,\n                    workspaceKey = workspaceKey,\n                    now = now,\n                    includeHistory = includeHistory\n                )\n                window?.record(workspaceKey, includeHistory, snapshotWriteStamp())\n            }\n            database.setTransactionSuccessful()")?;
            replace(&mut source,
                "        if (!sha256(raw).equals(expectedDigest, ignoreCase = true)) {\n            return null\n        }\n        val timestampColumn",
                "        val alreadyVerified = sourceRowId != null &&\n            (sourceTable == CURRENT_TABLE || sourceTable == HISTORY_TABLE) &&\n            snapshotReadWindow.get()?.covers(\n                getString(getColumnIndexOrThrow(\"workspace_key\")),\n                sourceTable == HISTORY_TABLE, snapshotWriteStamp()\n            ) == true\n        if (!alreadyVerified && !sha256(raw).equals(expectedDigest, ignoreCase = true)) {\n            return null\n        }\n        val timestampColumn")?;
            replace(&mut source,
                "            false\n        } else {\n            val workspaceKey = getString(getColumnIndexOrThrow(\"workspace_key\")) ?: return null",
                "            false\n        } else if (alreadyVerified) {\n            true\n        } else {\n            val workspaceKey = getString(getColumnIndexOrThrow(\"workspace_key\")) ?: return null")?;
            replace(&mut source,
                "        val resolvedSource = when {\n            envelopeVerified -> source",
                "        snapshotReadWindow.get()?.observeEnvelope(envelopeVerified)\n        val resolvedSource = when {\n            envelopeVerified -> source")?;
            source.push_str(VERIFICATION_WINDOW);
        }
        _ => {}
    }
    Ok(source)
}

const WORKSPACE_READ: &str = r####"    private fun loadPersistedDataForWorkspace(workspaceKey: String): WorkspaceReadResult {
        val files = workspaceFiles(workspaceKey)
        verifiedWritePreflight = null
        var verifiedPreflight: PersistPreflightStamp? = null
        val result = stateDatabase.withVerifiedSnapshotReads {
            val before = PersistPreflightStamp(
                stateDatabase.snapshotWriteStamp(), persistFileStamp(files.state), persistFileStamp(files.backup)
            )
            val loaded = loadVerifiedWorkspace(workspaceKey)
            val after = PersistPreflightStamp(
                stateDatabase.snapshotWriteStamp(), persistFileStamp(files.state), persistFileStamp(files.backup)
            )
            if (loaded.verifiedCurrentSnapshot != null && before == after &&
                stateDatabase.verifiedReadHasOnlySealedSnapshots()) {
                verifiedPreflight = after
            }
            loaded
        }
        // Publish only after the pinned transaction closes successfully. Normal
        // saves compare this stamp again, so startup cannot move its full-history
        // scan onto the first timer tap or reuse it after any database/file change.
        verifiedPreflight?.let { verifiedWritePreflight = workspaceKey to it }
        return result
    }

    private fun loadVerifiedWorkspace(workspaceKey: String): WorkspaceReadResult {
"####;

const READ_WINDOW: &str = r####"    private val snapshotReadWindow = ThreadLocal<SnapshotVerificationWindow>()

    fun verifiedReadHasOnlySealedSnapshots(): Boolean =
        snapshotReadWindow.get()?.allSnapshotsSealed == true

    fun <T> withVerifiedSnapshotReads(action: () -> T): T = withSnapshotWriteTransaction {
        // This synchronous scope pins the primary SQLite connection and its row set.
        // A receipt never survives commit/rollback or crosses a thread/account.
        if (snapshotReadWindow.get() != null) {
            action()
        } else {
            snapshotReadWindow.set(SnapshotVerificationWindow())
            try {
                action()
            } finally {
                snapshotReadWindow.remove()
            }
        }
    }

"####;

pub const VERIFICATION_WINDOW: &str = r####"

// Retains only scope metadata, never large JSON payloads or decoded histories.
internal class SnapshotVerificationWindow {
    private var verifiedStamp: Any? = null
    private val currentWorkspaces = mutableSetOf<String?>()
    private val historyWorkspaces = mutableSetOf<String?>()
    var allSnapshotsSealed: Boolean = true
        private set

    fun observeEnvelope(verified: Boolean) {
        // Unsealed metadata still needs the stricter normal-save preflight.
        // Later sealed rows cannot erase that evidence within this read.
        if (!verified) allSnapshotsSealed = false
    }

    fun covers(workspaceKey: String?, includeHistory: Boolean, stamp: Any): Boolean {
        if (verifiedStamp != stamp) return false
        val scopes = if (includeHistory) historyWorkspaces else currentWorkspaces
        return null in scopes || workspaceKey in scopes
    }

    fun record(workspaceKey: String?, includeHistory: Boolean, stamp: Any) {
        if (verifiedStamp != stamp) {
            currentWorkspaces.clear()
            historyWorkspaces.clear()
            verifiedStamp = stamp
        }
        currentWorkspaces += workspaceKey
        if (includeHistory) historyWorkspaces += workspaceKey
    }
}
"####;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn rendered(path: &str) -> String {
        let base = super::super::android_timer_latency::tests::rendered(path);
        let base = super::super::android_note_latency::render(path, &base).unwrap();
        let base = super::super::android_test_fixes::render(path, &base).unwrap();
        let base = super::super::android_startup_loading::render(path, &base).unwrap();
        let base = super::super::android_privacy_generation::render(path, &base).unwrap();
        render(path, &base).unwrap()
    }

    #[test]
    fn launch_keeps_recovery_and_required_saves_without_global_media_pruning() {
        let source = rendered(REPOSITORY);
        let start = source
            .find("    private suspend fun loadFromDisk()")
            .unwrap();
        let end = source[start..]
            .find("    private suspend fun updateData(")
            .unwrap()
            + start;
        let load = &source[start..end];
        assert!(!load.contains("pruneNoteMedia("));
        for guard in [
            "recoverJournaledNoteDrafts(",
            "recoverPendingNoteMediaImports(",
            "flushPersistLocked(resolved)",
            "if (durableForMediaPrune)",
            "NoteDraftJournal.removeIfCommitted(",
            "syncMicroBreakAlarm(data)",
        ] {
            assert!(load.contains(guard), "missing {guard}");
        }
        assert!(
            source.contains("selected === databaseCurrent && detectedFutureSchemaVersion == null")
        );
        assert!(source.contains("!recoveryOnly && blockedReason == null"));
        assert!(source.contains("primary?.raw == selected.raw && backup != null"));
        assert!(source.contains("lastPersistedSnapshot = result.verifiedCurrentSnapshot"));
        assert!(source.contains("if (appData == lastPersistedSnapshot && dirtySnapshot == null)"));
        assert!(source.contains("loaded.verifiedCurrentSnapshot != null && before == after"));
        assert!(source.contains("stateDatabase.verifiedReadHasOnlySealedSnapshots()"));
        assert!(source
            .contains("verifiedPreflight?.let { verifiedWritePreflight = workspaceKey to it }"));
        assert!(source.contains("if (cached?.first != workspaceKey || cached?.second != stamp)"));
        assert!(source.contains("pruneNoteMedia(snapshot)"));
    }

    #[test]
    fn receipts_are_transaction_scoped_and_never_seal_unverified_envelopes() {
        let source = rendered(DATABASE);
        assert!(source.contains(
            "withVerifiedSnapshotReads(action: () -> T): T = withSnapshotWriteTransaction"
        ));
        assert!(source.contains("finally {\n                snapshotReadWindow.remove()"));
        assert!(
            source.contains("window?.record(workspaceKey, includeHistory, snapshotWriteStamp())")
        );
        assert!(source.contains("sourceTable == CURRENT_TABLE || sourceTable == HISTORY_TABLE"));
        assert!(source.contains("snapshotReadWindow.get()?.observeEnvelope(envelopeVerified)"));
        assert!(source
            .contains("storedEnvelope.isBlank() || sourceTable == null || sourceRowId == null"));
        assert!(source.contains("if (!alreadyVerified && !sha256(raw).equals(expectedDigest"));
        assert!(
            rendered(REPOSITORY).contains("val result = stateDatabase.withVerifiedSnapshotReads {")
        );
    }
}
