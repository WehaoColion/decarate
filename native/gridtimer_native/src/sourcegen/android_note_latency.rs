// v2.22.37 - Limit note edits to one note and avoid decoding history during save preflight.
// Android implementation is authored here in Rust and emitted by sourcegen.

const REPOSITORY: &str = "com/ofairyo/gridtimer/data/TimerRepository.kt";

pub fn render(path: &str, base: &str) -> Result<String, String> {
    if path != REPOSITORY {
        return Ok(base.to_owned());
    }
    let mut source = base.to_owned();
    for name in ["updateData", "updateDataDetailed"] {
        replace(&mut source,
            &format!("    private suspend fun {name}(\n        timerOnly: Boolean = false,"),
            &format!("    private suspend fun {name}(\n        noteMutationId: String? = null,\n        timerOnly: Boolean = false,"))?;
    }
    replace(&mut source,
        "        return updateDataDetailed(\n            timerOnly = timerOnly,",
        "        return updateDataDetailed(\n            noteMutationId = noteMutationId,\n            timerOnly = timerOnly,")?;
    replace(&mut source,
        "        return updateData(expectedWorkspaceKey = expectedWorkspaceKey) { data ->\n            upsertNoteInData",
        "        return updateData(noteMutationId = note.id, expectedWorkspaceKey = expectedWorkspaceKey) { data ->\n            upsertNoteInData")?;
    let old = section(
        &source,
        "    suspend fun upsertNoteDurablyResult(",
        "    suspend fun createNextNoteVersionDurably(",
    )?
    .to_owned();
    let mut new = old.clone();
    replace(
        &mut new,
        "            updateDataDetailed(\n",
        "            updateDataDetailed(\n                noteMutationId = note.id,\n",
    )?;
    replace(&mut source, &old, &new)?;
    replace(&mut source,
        "                    val mutationBase = if (timerOnly) current.timerMutationData() else current",
        "                    check(!timerOnly || noteMutationId == null)\n                    val mutationBase = when {\n                        noteMutationId != null -> current.noteMutationData(noteMutationId)\n                        timerOnly -> current.timerMutationData()\n                        else -> current\n                    }")?;
    replace(&mut source,
        "                        if (timerOnly) current.acceptTimerMutation(normalized)\n                        else normalized.reuseUnchangedDomains(current)",
        "                        when {\n                            noteMutationId != null -> current.acceptNoteMutation(normalized, noteMutationId)\n                            timerOnly -> current.acceptTimerMutation(normalized)\n                            else -> normalized.reuseUnchangedDomains(current)\n                        }")?;
    replace(&mut source,
        "    private class RevisionOverflowException(message: String) : IllegalStateException(message)",
        &format!("{NOTE_DOMAIN}    private class RevisionOverflowException(message: String) : IllegalStateException(message)"))?;

    // Explicit product versions use their existing CAS/commit path, with the same
    // projection and rejoin as ordinary saves. Nothing partial reaches persistence.
    let old = section(
        &source,
        "    suspend fun createNextNoteVersionDurably(",
        "    private fun redactEncryptedNoteRecoveryCopies(",
    )?
    .to_owned();
    let mut new = old.clone();
    replace(
        &mut new,
        "        withContext(NonCancellable) {",
        "        withContext(NonCancellable + Dispatchers.Default) {",
    )?;
    replace(&mut new, "                val current = _appData.value", "                val workspace = _appData.value\n                val current = workspace.noteMutationData(note.id)")?;
    replace(&mut new, "                    ).sanitized(timestamp)", "                    ).sanitized(timestamp).let { workspace.acceptNoteMutation(it, note.id) }")?;
    replace(&mut source, &old, &new)?;

    let old = section(
        &source,
        "    private fun futureSchemaVersionOnAnySurface(",
        "    private fun migrateLegacyStateOnce(",
    )?
    .to_owned();
    replace(&mut source, &old, SCHEMA_PREFLIGHT)?;

    // A mirror already validated at the start of this block does not need another
    // full decode after writing the primary file. Track the successful backup write.
    replace(&mut source,
        "                val backup = readPersistedCandidate(files.backup)\n                if (primary != null)",
        "                val backup = readPersistedCandidate(files.backup)\n                var backupReady = backup != null\n                if (primary != null)")?;
    replace(&mut source,
        "                        AtomicFileStore.writeText(files.backup, primary.raw)\n",
        "                        AtomicFileStore.writeText(files.backup, primary.raw)\n                        backupReady = true\n")?;
    replace(
        &mut source,
        "                if (readPersistedCandidate(files.backup) == null) {",
        "                if (!backupReady) {",
    )?;
    Ok(source)
}

fn replace(source: &mut String, old: &str, new: &str) -> Result<(), String> {
    if source.matches(old).count() != 1 {
        return Err(format!("note latency anchor is not unique: {old}"));
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

const NOTE_DOMAIN: &str = r####"    // Only upsert and explicit version creation may use this projection. Folder
    // membership, deletion and attachment tombstones remain inputs to the original
    // Rust engine. Other notes/history never enter its JSON or normalization passes.
    private fun AppData.noteMutationData(noteId: String): AppData = AppData(
        schemaVersion = schemaVersion,
        noteFolders = noteFolders,
        notes = notes.filter { it.id == noteId },
        tombstones = tombstones
    )

    private fun AppData.acceptNoteMutation(mutation: AppData, noteId: String): AppData {
        check(mutation.notes.all { it.id == noteId }) { "Unexpected note in a scoped mutation." }
        val nextNotes = (mutation.notes + notes.filterNot { it.id == noteId }).reuseIfEqual(notes)
        return if (nextNotes === notes) this else copy(notes = nextNotes)
    }

"####;

const SCHEMA_PREFLIGHT: &str = r####"    private fun futureSchemaVersionOnAnySurface(
        workspaceKey: String,
        includeUnmigratedLegacy: Boolean = false
    ): Int? {
        var future: Int? = null
        fun observe(raw: String, declared: Int? = null) {
            val version = maxOf(rawSchemaVersion(raw) ?: APP_DATA_SCHEMA_VERSION, declared ?: APP_DATA_SCHEMA_VERSION)
            if (version > APP_DATA_SCHEMA_VERSION) future = maxOf(future ?: version, version)
        }
        // Keep digest/envelope verification and all future-version evidence. Save
        // preflight needs only the schema header, never every note in every snapshot.
        stateDatabase.forEachSnapshot(workspaceKey) { observe(it.appDataJson, it.schemaVersion) }
        stateDatabase.forEachQuarantinedCurrentSnapshot(workspaceKey) { observe(it.appDataJson, it.schemaVersion) }
        val files = workspaceFiles(workspaceKey)
        fun observeFile(file: File) {
            if (file.isFile) observe(file.readText())
        }
        observeFile(files.state)
        observeFile(files.backup)
        if (workspaceKey == GUEST_WORKSPACE_KEY && includeUnmigratedLegacy) {
            observeFile(legacyStateFile)
            observeFile(legacyBackupFile)
            observeFile(legacyTempFile)
        }
        return future
    }

"####;

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered() -> String {
        let source = super::super::android_timer_latency::tests::rendered(REPOSITORY);
        render(REPOSITORY, &source).unwrap()
    }

    #[test]
    fn scoped_saves_rejoin_before_durable_publication() {
        let source = rendered();
        let update = section(
            &source,
            "    private suspend fun updateDataDetailed(",
            "    // Sanitization remains authoritative.",
        )
        .unwrap();
        assert!(
            update.find("current.acceptNoteMutation").unwrap()
                < update.find("persistSafelyDetailed(updated)").unwrap()
        );
        assert!(
            update.find("persistSafelyDetailed(updated)").unwrap()
                < update.find("_appData.value = updated").unwrap()
        );
        assert!(source
            .contains("noteMutationId = note.id, expectedWorkspaceKey = expectedWorkspaceKey"));
        assert!(source.contains(
            "noteMutationId = note.id,\n                expectedWorkspaceKey = workspaceKey"
        ));
        assert!(NOTE_DOMAIN.contains("else copy(notes = nextNotes)"));
        assert!(!NOTE_DOMAIN.contains("persist"));
    }

    #[test]
    fn versions_preserve_cas_workspace_lock_and_full_snapshot_commit() {
        let source = rendered();
        let version = section(
            &source,
            "    suspend fun createNextNoteVersionDurably(",
            "    private fun redactEncryptedNoteRecoveryCopies(",
        )
        .unwrap();
        for guard in [
            "activeWorkspaceKey != workspaceKey",
            "writeMutex.withLock",
            "expectedLatestVersionId = expectedLatestVersionId",
            "workspace.acceptNoteMutation(it, note.id)",
            "persistSafely(updated)",
        ] {
            assert!(version.contains(guard), "missing {guard}");
        }
        assert!(version.contains("withContext(NonCancellable + Dispatchers.Default)"));
    }

    #[test]
    fn preflight_visits_verified_history_quarantine_and_legacy_without_full_decoding() {
        let source = rendered();
        let preflight = section(
            &source,
            "    private fun futureSchemaVersionOnAnySurface(",
            "    private fun migrateLegacyStateOnce(",
        )
        .unwrap();
        for evidence in [
            "forEachSnapshot",
            "forEachQuarantinedCurrentSnapshot",
            "it.schemaVersion",
            "rawSchemaVersion(raw)",
            "observeFile(files.backup)",
            "observeFile(legacyTempFile)",
        ] {
            assert!(preflight.contains(evidence));
        }
        assert!(!preflight.contains("decodePersistedCandidate"));
        assert!(!preflight.contains("readDatabaseCandidateSurfaces"));
        assert!(source.contains("var backupReady = backup != null"));
    }

    fn project(data: &serde_json::Value, note_id: &str) -> serde_json::Value {
        serde_json::json!({
            "schemaVersion": data["schemaVersion"],
            "noteFolders": data["noteFolders"],
            "notes": data["notes"].as_array().unwrap().iter().filter(|n| n["id"] == note_id).collect::<Vec<_>>(),
            "tombstones": data["tombstones"]
        })
    }

    fn target(data: &serde_json::Value) -> &serde_json::Value {
        data["notes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["id"] == "edited")
            .unwrap()
    }

    #[test]
    fn scoped_native_note_operations_match_full_workspace_results() {
        use gridtimer_native::app_data;
        use serde_json::{json, Value};
        const NOW: i64 = 1_788_000_000_000;
        let mut seed: Value = serde_json::from_str(&app_data::default_app_data_json(NOW)).unwrap();
        seed["noteFolders"] = json!([{"id":"folder","name":"Work","createdAtEpochMillis":10,"updatedAtEpochMillis":10}]);
        seed["notes"] = json!([
            {"id":"edited","title":"First","content":"Original","folderId":"folder","createdAtEpochMillis":10,"updatedAtEpochMillis":10},
            {"id":"retained","title":"Keep","content":"Untouched".repeat(4000),"createdAtEpochMillis":10,"updatedAtEpochMillis":10}
        ]);
        seed["sessions"] = json!((0..1500).map(|i| json!({"id":format!("s-{i}"),"slotId":1,"slotTitle":"Focus","startedAtEpochMillis":100+i*1000,"endedAtEpochMillis":600+i*1000,"durationMillis":500,"updatedAtEpochMillis":600+i*1000})).collect::<Vec<_>>());
        let raw = app_data::sanitize_app_data_json(&seed.to_string(), NOW).unwrap();
        let base: Value = serde_json::from_str(&raw).unwrap();
        let baseline_projection = project(&base, "edited").to_string();
        let mut cases = 0;
        for scenario in 0..12 {
            let mut data = base.clone();
            let mut incoming = target(&data).clone();
            match scenario {
                0 => incoming["title"] = json!("Renamed"),
                1 => {
                    incoming["content"] = json!("Body\n第二行");
                    incoming["document"] = Value::Null;
                }
                2 => incoming["pinned"] = json!(true),
                3 => incoming["folderId"] = Value::Null,
                4 => incoming["accentSeed"] = json!(25),
                5 => {
                    incoming["title"] = json!("");
                    incoming["content"] = json!("");
                    incoming["document"] = Value::Null;
                }
                6 => incoming["updatedAtEpochMillis"] = json!(NOW + 100_000),
                7 => {
                    incoming["id"] = json!("new-note");
                    incoming["versions"] = json!([]);
                    incoming["latestVersionId"] = json!("");
                }
                8 => {
                    data["tombstones"] = json!([{"entityType":"NOTE","entityId":"edited","deletedAtEpochMillis":NOW+1}])
                }
                9 => {
                    data["tombstones"] = json!([{"entityType":"NOTE_FOLDER","entityId":"folder","deletedAtEpochMillis":NOW+1}])
                }
                10 => incoming["protectionStateRevision"] = json!(99),
                _ => {
                    incoming["document"] = json!({"blocks":[{"id":"p","type":"RICH_TEXT","text":"Bold","html":"<p><b>Bold</b></p>"}]})
                }
            }
            let id = incoming["id"].as_str().unwrap();
            let full = app_data::upsert_note_app_data_json(
                &data.to_string(),
                &incoming.to_string(),
                NOW + 2,
            );
            let scoped = app_data::upsert_note_app_data_json(
                &project(&data, id).to_string(),
                &incoming.to_string(),
                NOW + 2,
            );
            assert_eq!(
                full.is_some(),
                scoped.is_some(),
                "rejection mismatch in {scenario}"
            );
            if let (Some(full), Some(scoped)) = (full, scoped) {
                let full: Value = serde_json::from_str(&full).unwrap();
                let scoped: Value = serde_json::from_str(&scoped).unwrap();
                let full_note = full["notes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|n| n["id"] == id);
                let scoped_note = scoped["notes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|n| n["id"] == id);
                assert_eq!(full_note, scoped_note, "note mismatch in {scenario}");
            }
            cases += 1;
        }
        let note = target(&base);
        let version = note["latestVersionId"].as_str().unwrap();
        for expected in [version, "stale-version"] {
            let full = app_data::create_note_version_app_data_json(
                &raw,
                &note.to_string(),
                version,
                expected,
                "request-next",
                NOW + 5,
            );
            let scoped = app_data::create_note_version_app_data_json(
                &baseline_projection,
                &note.to_string(),
                version,
                expected,
                "request-next",
                NOW + 5,
            );
            assert_eq!(full.is_some(), scoped.is_some());
            if let (Some(full), Some(scoped)) = (full, scoped) {
                assert_eq!(
                    target(&serde_json::from_str(&full).unwrap()),
                    target(&serde_json::from_str(&scoped).unwrap())
                );
            }
            cases += 1;
        }
        let start = std::time::Instant::now();
        for _ in 0..5 {
            assert!(
                app_data::upsert_note_app_data_json(&raw, &note.to_string(), NOW + 10).is_some()
            );
        }
        let full_millis = start.elapsed().as_millis();
        let start = std::time::Instant::now();
        for _ in 0..5 {
            assert!(app_data::upsert_note_app_data_json(
                &baseline_projection,
                &note.to_string(),
                NOW + 10
            )
            .is_some());
        }
        println!("note_projection cases={cases} full_bytes={} scoped_bytes={} full_5x_ms={full_millis} scoped_5x_ms={} result=PASS", raw.len(), baseline_projection.len(), start.elapsed().as_millis());
    }
}
