// v2.22.40 - Keep startup truthful and avoid full historical timer decoding for media retention.
const REPOSITORY: &str = "com/ofairyo/gridtimer/data/TimerRepository.kt";
const SCREEN: &str = "com/ofairyo/gridtimer/ui/GridTimerScreen.kt";
const VIEW_MODEL: &str = "com/ofairyo/gridtimer/ui/TimerViewModel.kt";

fn replace(source: &mut String, old: &str, new: &str) -> Result<(), String> {
    if source.matches(old).count() != 1 {
        return Err(format!("startup anchor is not unique: {old}"));
    }
    *source = source.replacen(old, new, 1);
    Ok(())
}

pub fn render(path: &str, base: &str) -> Result<String, String> {
    let mut source = base.to_owned();
    match path {
        REPOSITORY => {
            replace(&mut source,
                "    private val _appData = MutableStateFlow(AppData.default())",
                "    private val _isInitialized = MutableStateFlow(false)\n    val isInitialized: StateFlow<Boolean> = _isInitialized.asStateFlow()\n    private val _appData = MutableStateFlow(AppData.default())")?;
            replace(&mut source,
                "    private val initializationJob = scope.launch {\n        var sessionRestored = false\n        runCatching {",
                "    private val initializationJob = scope.launch {\n        val initializationStarted = android.os.SystemClock.elapsedRealtime()\n        try {\n        var sessionRestored = false\n        runCatching {")?;
            replace(&mut source,
                "            fallbackToDefaults()\n        }\n    }\n    private val accountBootstrapJob",
                "            fallbackToDefaults()\n        }\n        } finally {\n            _isInitialized.value = true\n            logDiagnosticEvent(\n                category = \"repository.initialized\",\n                message = \"Startup completed elapsedMs=${android.os.SystemClock.elapsedRealtime() - initializationStarted}\"\n            )\n        }\n    }\n    private val accountBootstrapJob")?;
            replace(&mut source,
                "        val storedDatabaseCandidates = runCatching {\n            readDatabaseCandidateSurfaces(workspaceKey)",
                "        val storedDatabaseCandidates = runCatching {\n            readDatabaseCandidateSurfaces(workspaceKey, authoritativeStartup = true)")?;
            replace(&mut source,
                "    private fun readDatabaseCandidateSurfaces(workspaceKey: String): List<PersistedCandidate> {",
                "    private fun readDatabaseCandidateSurfaces(\n        workspaceKey: String,\n        authoritativeStartup: Boolean = false\n    ): List<PersistedCandidate> {")?;
            replace(&mut source,
                "        stateDatabase.forEachSnapshot(workspaceKey) { stored ->\n            decodeStoredCandidate(stored)?.let(surfaces::accept)\n        }",
                r####"        var verifiedCurrentAccepted = false
        stateDatabase.forEachSnapshot(workspaceKey) { stored ->
            // Verification still visits every retained row. A verified current is
            // authoritative; older history needs only the future-schema barrier.
            if (!(authoritativeStartup && verifiedCurrentAccepted &&
                    stored.source.startsWith("sqlite-history") && snapshotFutureSchema(
                        stored.appDataJson,
                        stored.schemaVersion.takeIf { stored.envelopeVerified }
                    ) == null)) {
                decodeStoredCandidate(stored)?.let { candidate ->
                    surfaces.accept(candidate)
                    if (candidate.source == "sqlite-current") verifiedCurrentAccepted = true
                }
            }
        }"####)?;
            replace(&mut source,
                "                        val candidate = decodeStoredCandidate(stored)\n                            ?: error(\"Could not decode a database snapshot while retaining note media.\")\n                        retainCandidate(workspaceKey, candidate)",
                r####"                        val future = snapshotFutureSchema(
                            stored.appDataJson,
                            stored.schemaVersion.takeIf { stored.envelopeVerified }
                        )
                        if (future != null) {
                            protectedWorkspaces += workspaceKey
                        } else {
                            // Only attachment ownership participates in pruning. Avoid
                            // reconstructing thousands of unrelated timer sessions in
                            // each historical snapshot. Malformed references abort all
                            // pruning through the existing failure handler.
                            val retained = referencedAttachmentsByWorkspace.getOrPut(workspaceKey) { linkedMapOf() }
                            snapshotMediaReferences(stored.appDataJson).forEach { attachment ->
                                retained[attachment.fileName] = attachment
                            }
                        }"####)?;
            source.push_str(SNAPSHOT_PROJECTIONS);
        }
        VIEW_MODEL => {
            replace(&mut source, "    val appData = repository.appData",
                "    val isInitialized = repository.isInitialized\n    val appData = repository.appData")?;
        }
        SCREEN => {
            replace(&mut source,
                "    val compositionStartNanos = SystemClock.elapsedRealtimeNanos()\n    val appData by viewModel.appData.collectAsState()",
                r####"    val compositionStartNanos = SystemClock.elapsedRealtimeNanos()
    val initialized by viewModel.isInitialized.collectAsState()
    if (!initialized) {
        SmartisanTheme(preference = ThemeMode.SYSTEM) {
            Surface(modifier = Modifier.fillMaxSize()) {
                Column(
                    modifier = Modifier.fillMaxSize(),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.Center
                ) {
                    androidx.compose.material3.CircularProgressIndicator()
                    Spacer(modifier = Modifier.height(20.dp))
                    Text("正在读取本机数据…")
                }
            }
        }
        return
    }
    val appData by viewModel.appData.collectAsState()"####)?;
        }
        _ => {}
    }
    Ok(source)
}

pub const SNAPSHOT_PROJECTIONS: &str = r####"

@kotlinx.serialization.Serializable
private data class SnapshotSchemaProjection(val schemaVersion: Long? = null)

@kotlinx.serialization.Serializable
private data class SnapshotMediaRevisionProjection(
    val attachments: List<NoteAttachment> = emptyList()
)

@kotlinx.serialization.Serializable
private data class SnapshotMediaNoteProjection(
    val attachments: List<NoteAttachment> = emptyList(),
    val revisions: List<SnapshotMediaRevisionProjection> = emptyList(),
    val versions: List<SnapshotMediaRevisionProjection> = emptyList()
)

@kotlinx.serialization.Serializable
private data class SnapshotMediaProjection(
    val notes: List<SnapshotMediaNoteProjection> = emptyList()
)

private val snapshotProjectionJson = Json {
    ignoreUnknownKeys = true
    coerceInputValues = false
}

internal fun snapshotFutureSchema(raw: String, verifiedDeclaredVersion: Int? = null): Int? {
    val rawVersion = runCatching {
        snapshotProjectionJson.decodeFromString<SnapshotSchemaProjection>(raw).schemaVersion
            ?.coerceIn(Int.MIN_VALUE.toLong(), Int.MAX_VALUE.toLong())?.toInt()
    }.getOrNull()
    return maxOf(rawVersion ?: APP_DATA_SCHEMA_VERSION, verifiedDeclaredVersion ?: APP_DATA_SCHEMA_VERSION)
        .takeIf { it > APP_DATA_SCHEMA_VERSION }
}

internal fun snapshotMediaReferences(raw: String): List<NoteAttachment> {
    // Missing fields retain legacy defaults. Explicit nulls, invalid arrays,
    // malformed JSON and invalid attachment entries must throw, never become empty.
    val snapshot = snapshotProjectionJson.decodeFromString<SnapshotMediaProjection>(raw)
    return buildList {
        snapshot.notes.forEach { note ->
            addAll(note.attachments)
            note.revisions.forEach { addAll(it.attachments) }
            note.versions.forEach { addAll(it.attachments) }
        }
    }
}
"####;

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(path: &str) -> String {
        let base = super::super::android_timer_latency::tests::rendered(path);
        let base = super::super::android_note_latency::render(path, &base).unwrap();
        let base = super::super::android_test_fixes::render(path, &base).unwrap();
        render(path, &base).unwrap()
    }

    #[test]
    fn startup_keeps_verification_and_future_barriers_before_authoritative_selection() {
        let source = rendered(REPOSITORY);
        assert!(source
            .contains("readDatabaseCandidateSurfaces(workspaceKey, authoritativeStartup = true)"));
        assert!(source.contains("stateDatabase.forEachSnapshot(workspaceKey)"));
        assert!(source.contains("stored.schemaVersion.takeIf { stored.envelopeVerified }"));
        assert!(source
            .contains("stored.source.startsWith(\"sqlite-history\") && snapshotFutureSchema("));
        assert!(source.contains(
            "futureCurrentSurface ?: databaseCurrent ?: jsonPrimaryAuthority ?: recovery"
        ));
        assert!(source.contains("Startup completed elapsedMs="));
    }

    #[test]
    fn startup_does_not_publish_default_controls_or_cancel_running_notifications() {
        let source = rendered(SCREEN);
        let gate = source.find("if (!initialized)").unwrap();
        let controls = source
            .find("    val appData by viewModel.appData.collectAsState()")
            .unwrap();
        let notifications = source
            .find("    TimerNotificationEffects(slots = appData.slots)")
            .unwrap();
        assert!(gate < controls && controls < notifications);
        assert!(source[gate..controls].contains("        return"));
        assert!(rendered(VIEW_MODEL).contains("val isInitialized = repository.isInitialized"));
    }

    #[test]
    fn media_projection_retains_versions_and_future_workspace_protection() {
        let source = rendered(REPOSITORY);
        assert!(source.contains("protectedWorkspaces += workspaceKey"));
        assert!(source.contains("snapshotMediaReferences(stored.appDataJson)"));
        assert!(SNAPSHOT_PROJECTIONS.contains("note.revisions.forEach { addAll(it.attachments) }"));
        assert!(SNAPSHOT_PROJECTIONS.contains("note.versions.forEach { addAll(it.attachments) }"));
        assert!(SNAPSHOT_PROJECTIONS.contains("coerceInputValues = false"));
    }
}
