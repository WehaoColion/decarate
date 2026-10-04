// v2.22.32 - Include persistent collection snapshots in bounded diagnostic exports.

const EXPORTER_PATH: &str = "com/ofairyo/gridtimer/diagnostics/DiagnosticsExporter.kt";
const SESSION_PATH: &str = "com/ofairyo/gridtimer/diagnostics/DiagnosticRecordingSessionStore.kt";
const DATABASE_PATH: &str = "com/ofairyo/gridtimer/data/AppStateDatabase.kt";

pub fn render(path: &str, base: &str) -> Result<String, String> {
    match path {
        EXPORTER_PATH => render_exporter(base),
        SESSION_PATH => {
            let mut rendered = base.to_owned();
            replace(
                &mut rendered,
                "        CrashLogStore.clear(appContext)\n",
                "        // Keep the previous crash available when a new recording starts.\n",
                "preserve the previous crash report",
            )?;
            Ok(rendered)
        }
        DATABASE_PATH => {
            let mut rendered = base.to_owned();
            replace(
                &mut rendered,
                "            database.execSQL(\"PRAGMA app_state_backup.journal_mode=DELETE\")",
                r#"            database.rawQuery("PRAGMA app_state_backup.journal_mode=DELETE", null).use { cursor ->
                check(cursor.moveToFirst() && cursor.getString(0).equals("delete", ignoreCase = true)) {
                    "Could not select a standalone journal mode for the database backup."
                }
            }"#,
                "consume the result row from backup journal_mode",
            )?;
            Ok(rendered)
        }
        _ => Ok(base.to_owned()),
    }
}

fn replace(target: &mut String, old: &str, new: &str, label: &str) -> Result<(), String> {
    let count = target.match_indices(old).count();
    if count != 1 {
        return Err(format!("expected one {label} fragment, found {count}"));
    }
    *target = target.replacen(old, new, 1);
    Ok(())
}

fn replace_section(
    target: &mut String,
    start: &str,
    end: &str,
    replacement: &str,
) -> Result<(), String> {
    let starts = target.match_indices(start).collect::<Vec<_>>();
    if starts.len() != 1 {
        return Err(format!("expected one section start {start:?}"));
    }
    let first = starts[0].0;
    let last = target[first..]
        .find(end)
        .map(|offset| first + offset)
        .ok_or_else(|| format!("missing section end {end:?}"))?;
    target.replace_range(first..last, replacement);
    Ok(())
}

fn render_exporter(base: &str) -> Result<String, String> {
    let mut rendered = base.to_owned();
    replace(
        &mut rendered,
        "import java.io.File\n",
        "import java.io.File\nimport java.io.ByteArrayOutputStream\nimport java.io.RandomAccessFile\nimport java.nio.ByteBuffer\nimport java.nio.CharBuffer\nimport java.nio.charset.CodingErrorAction\nimport java.util.concurrent.TimeUnit\nimport java.util.concurrent.atomic.AtomicReference\nimport java.util.zip.CRC32\n",
        "bounded diagnostic export imports",
    )?;
    replace_section(
        &mut rendered,
        "    fun recordDiagnosticLog(",
        "    private fun buildSummaryBlock(",
        DIAGNOSTIC_EXPORT,
    )?;
    replace_section(
        &mut rendered,
        "    private fun dumpLogcatForCurrentProcess(): String {",
        "    private fun readRecentProcessExitReasons(",
        LOGCAT_EXPORT,
    )?;
    replace_section(
        &mut rendered,
        "    private fun verifyExportZip(file: File, expectedActiveAppDataRaw: String) {",
        "    private fun sha256(bytes: ByteArray): String {",
        STREAMING_ZIP_VERIFICATION,
    )?;
    replace(
        &mut rendered,
        "        return File(context.cacheDir, \"shared_exports\").apply { mkdirs() }",
        "        return File(context.cacheDir, \"shared_exports\").apply {\n            check(isDirectory || mkdirs()) { \"Could not create the export directory.\" }\n        }",
        "export directory creation failure",
    )?;
    // Optional diagnostics must not prevent a complete, verified data backup.
    replace(
        &mut rendered,
        "                val eventLog = DiagnosticLogStore.snapshotFile(appContext)?.readText()",
        "                val eventLog = collectDiagnosticSection(\"app_event_log\") { readBoundedEventLog(appContext) }",
        "isolate optional backup event log collection",
    )?;
    replace(
        &mut rendered,
        "                        content = buildSummaryBlock(appContext, appData, now, performanceSummary)",
        "                        content = collectDiagnosticSection(\"export_summary\") {\n                            buildSummaryBlock(appContext, appData, now, performanceSummary)\n                        }",
        "isolate optional backup summary collection",
    )?;
    replace(
        &mut rendered,
        "            appendLine(\"app_event_log_exists=${DiagnosticLogStore.snapshotFile(context) != null}\")",
        "            appendLine(\"app_event_log_exists=${File(context.filesDir, \"diagnostic_events.log\").isFile}\")",
        "inspect event log metadata without creating another snapshot",
    )?;
    replace_section(
        &mut rendered,
        "                    DiagnosticLogStore.snapshotFile(appContext)?.let { fileOnDisk ->",
        "                }\n                verifyExportZip(",
        r#"                    writeZipTextEntry(zipStream, "app_event_log.txt", eventLog)
                    writeZipTextEntry(
                        zipStream,
                        LAST_CRASH_FILE_NAME,
                        collectDiagnosticSection("last_crash") {
                            readBoundedFile(File(appContext.filesDir, LAST_CRASH_FILE_NAME), tail = false)
                        }
                    )
"#,
    )?;
    replace(
        &mut rendered,
        "                        appendLine(\"description=${reason.description ?: \"\"}\")",
        "                        appendLine(\"description=${reason.description.orEmpty().take(4_096)}\")",
        "bound system exit descriptions",
    )?;
    // A diagnostic-event write cannot turn an already published export into a failure.
    rendered = rendered.replace(
        "            DiagnosticLogStore.record(",
        "            recordExportEvent(",
    );
    Ok(rendered)
}

const DIAGNOSTIC_EXPORT: &str = r####"    private const val DIAGNOSTIC_FILE_BYTES = 256 * 1024
    private const val DIAGNOSTIC_EVENT_LINES = 1_200
    private const val LOGCAT_BYTES = 256 * 1024
    private const val LOGCAT_TIMEOUT_MILLIS = 2_000L

    fun recordDiagnosticLog(
        context: Context,
        appData: AppData,
        recordingSession: DiagnosticRecordingSession? = null
    ): File {
        val appContext = context.applicationContext
        val now = Instant.now()
        return runCatching {
            val directory = exportDirectory(appContext)
            val file = File(
                directory,
                "grid_timer_log_${fileTimestampFormatter.format(now)}_${UUID.randomUUID()}.txt"
            )
            check(!file.exists()) { "Diagnostic export destination already exists." }
            val temporary = File.createTempFile(".${file.name}.", ".tmp", directory)
            try {
                // This report is available independently of recording and database persistence.
                val eventLog = collectDiagnosticSection("app_event_log") { readBoundedEventLog(appContext) }
                val collectionSnapshot = runCatching { DiagnosticRecordingSessionStore.snapshotForExport(appContext) }
                temporary.bufferedWriter(Charsets.UTF_8).use { writer ->
                    fun section(name: String, content: () -> String) {
                        writer.appendLine("[$name]")
                        writer.appendLine(collectDiagnosticSection(name, content))
                        writer.appendLine()
                    }
                    section("meta") {
                        buildString {
                            appendLine("captured_at=${textTimestampFormatter.format(now)}")
                            appendLine("package_name=${BuildConfig.APPLICATION_ID}")
                            appendLine("version_name=${BuildConfig.VERSION_NAME}")
                            appendLine("version_code=${BuildConfig.VERSION_CODE}")
                            appendLine("android_sdk=${Build.VERSION.SDK_INT}")
                            appendLine("android_release=${Build.VERSION.RELEASE.orEmpty()}")
                            appendLine("brand=${Build.BRAND.orEmpty()}")
                            appendLine("manufacturer=${Build.MANUFACTURER.orEmpty()}")
                            appendLine("model=${Build.MODEL.orEmpty()}")
                            appendLine("device=${Build.DEVICE.orEmpty()}")
                            appendLine("process_id=${Process.myPid()}")
                            appendLine("diagnostic_format=2")
                            appendLine("app_data_payload=included_counts_only")
                            appendLine("file_section_byte_limit=$DIAGNOSTIC_FILE_BYTES")
                            appendLine("logcat_byte_limit=$LOGCAT_BYTES")
                            appendLine("logcat_timeout_millis=$LOGCAT_TIMEOUT_MILLIS")
                        }
                    }
                    section("summary") {
                        buildString {
                            appendLine("schema_version=${appData.schemaVersion}")
                            appendLine("category_count=${appData.categories.size}")
                            appendLine("slot_count=${appData.slots.size}")
                            appendLine("running_slot_count=${appData.slots.count { it.runningSinceEpochMillis != null }}")
                            appendLine("session_count=${appData.sessions.size}")
                            appendLine("archived_task_count=${appData.archivedTasks.size}")
                            appendLine("note_count=${appData.notes.size}")
                        }
                    }
                    section("recording_session") {
                        val collected = collectionSnapshot.getOrThrow().session
                        val recorded = collected.takeIf { it.hasCollection } ?: recordingSession ?: collected
                        buildString {
                            appendLine("active=${recorded.isActive}")
                            appendLine("session_id=${recorded.sessionId.orEmpty()}")
                            appendLine("has_collection=${recorded.hasCollection}")
                            appendLine("started_at_epoch_millis=${recorded.startedAtEpochMillis ?: 0L}")
                            appendLine("stopped_at_epoch_millis=${recorded.stoppedAtEpochMillis ?: 0L}")
                            appendLine("event_count=${recorded.eventCount}")
                            appendLine("collected_bytes=${recorded.collectedBytes}")
                            appendLine("dropped_event_count=${recorded.droppedEventCount}")
                            appendLine("limit_reached=${recorded.limitReached}")
                            appendLine("interrupted=${recorded.interrupted}")
                            appendLine("collection_error=${recorded.lastError.orEmpty()}")
                            appendLine("evidence_scope=latest_available_including_previous_process")
                        }
                    }
                    section("collected_events") { collectionSnapshot.getOrThrow().events }
                    section("app_event_log") { eventLog }
                    section("performance_summary") { buildPerformanceSummary(eventLog) }
                    section("last_crash") {
                        readBoundedFile(File(appContext.filesDir, LAST_CRASH_FILE_NAME), tail = false)
                    }
                    section("recent_process_exit_reasons") {
                        readRecentProcessExitReasons(appContext, sinceEpochMillis = null)
                    }
                    section("logcat_current_process") { dumpLogcatForCurrentProcess() }
                    section("logcat_recent_global") { dumpRecentGlobalLogcat() }
                }
                check(temporary.length() > 0L) { "Diagnostic export is empty." }
                check(!file.exists()) { "Diagnostic export destination appeared before publication." }
                AtomicFileStore.replaceWithSyncedFile(file, temporary)
                recordExportEvent(appContext, "export.log", "Created diagnostic log ${file.name} (${file.length()} bytes).")
                file
            } finally {
                temporary.delete()
            }
        }.getOrElse { throwable ->
            recordExportEvent(appContext, "export.log", "Failed to create diagnostic log.", throwable)
            throw throwable
        }
    }

    private fun collectDiagnosticSection(name: String, collect: () -> String): String {
        return try {
            collect().ifBlank { "status=empty" }
        } catch (throwable: Exception) {
            "status=collection_failed\nsection=$name\nerror=${throwable.javaClass.simpleName}\n" +
                "message=${throwable.message.orEmpty().take(1_024)}"
        }
    }

    private fun recordExportEvent(context: Context, category: String, message: String, throwable: Throwable? = null) {
        runCatching { DiagnosticLogStore.record(context, category, message, throwable) }
    }

    private fun readBoundedFile(file: File, tail: Boolean): String {
        if (!file.exists()) return "status=not_available"
        RandomAccessFile(file, "r").use { input ->
            val size = input.length()
            val length = minOf(size, DIAGNOSTIC_FILE_BYTES.toLong()).toInt()
            val offset = if (tail) (size - length).coerceAtLeast(0L) else 0L
            input.seek(offset)
            val bytes = ByteArray(length)
            var count = 0
            while (count < bytes.size) {
                val read = input.read(bytes, count, bytes.size - count)
                if (read < 0) break
                count += read
            }
            return buildString {
                appendLine("source=${file.name}")
                appendLine("source_bytes=$size captured_bytes=$count truncated=${size > length}")
                append(String(bytes, 0, count, Charsets.UTF_8))
            }
        }
    }

    private fun readBoundedEventLog(context: Context): String {
        // The writer flush has its own one-second limit; read persisted files without its lock.
        val flush = collectDiagnosticSection("event_flush") {
            DiagnosticLogStore.flushPendingWrites()
            "flush_requested=true"
        }
        return buildString {
            appendLine(flush)
            for (name in listOf("diagnostic_events.1.log", "diagnostic_events.log")) {
                appendLine(collectDiagnosticSection(name) {
                    val content = readBoundedFile(File(context.filesDir, name), tail = true)
                    val lines = content.lineSequence().toList()
                    if (lines.size <= DIAGNOSTIC_EVENT_LINES) content
                    else "source=$name\nline_limit=$DIAGNOSTIC_EVENT_LINES truncated=true\n" +
                        lines.takeLast(DIAGNOSTIC_EVENT_LINES).joinToString("\n")
                })
            }
        }
    }

"####;

const LOGCAT_EXPORT: &str = r####"    private fun dumpLogcatForCurrentProcess(): String {
        return dumpLogcat(
            command = listOf("logcat", "-d", "-v", "threadtime", "--pid=${Process.myPid()}", "-t", "900"),
            maxLines = 900
        )
    }

    private fun dumpRecentGlobalLogcat(): String {
        return dumpLogcat(
            command = listOf("logcat", "-d", "-v", "threadtime", "-t", "1200"),
            maxLines = 1_200
        )
    }

    private data class LogcatCapture(val text: String, val truncated: Boolean, val failure: String?)

    private fun dumpLogcat(command: List<String>, maxLines: Int): String {
        val process = ProcessBuilder(command).redirectErrorStream(true).start()
        val captured = AtomicReference<LogcatCapture?>(null)
        val reader = Thread({
            val output = ByteArrayOutputStream()
            var truncated = false
            var failure: String? = null
            try {
                process.inputStream.use { input ->
                    val buffer = ByteArray(4_096)
                    while (true) {
                        val remaining = LOGCAT_BYTES - output.size()
                        val read = input.read(buffer, 0, minOf(buffer.size, remaining + 1))
                        if (read < 0) break
                        val retained = minOf(read, remaining)
                        if (retained > 0) output.write(buffer, 0, retained)
                        if (read > retained) {
                            truncated = true
                            break
                        }
                    }
                }
            } catch (throwable: Exception) {
                failure = "${throwable.javaClass.simpleName}: ${throwable.message.orEmpty().take(512)}"
            } finally {
                captured.set(LogcatCapture(output.toString(Charsets.UTF_8.name()), truncated, failure))
                if (truncated) runCatching { process.destroy() }
            }
        }, "diagnostic-logcat-reader").apply { isDaemon = true }
        try {
            reader.start()
            val exited = process.waitFor(LOGCAT_TIMEOUT_MILLIS, TimeUnit.MILLISECONDS)
            if (!exited) {
                process.destroy()
                if (!process.waitFor(150L, TimeUnit.MILLISECONDS)) process.destroyForcibly()
            }
            reader.join(500L)
            val result = captured.get()
            val lines = result?.text.orEmpty().lineSequence().toList()
            return buildString {
                appendLine("status=${if (!exited) "timeout" else if (result == null) "reader_timeout" else "collected"}")
                appendLine("byte_limit=$LOGCAT_BYTES line_limit=$maxLines")
                appendLine("truncated=${result?.truncated == true || lines.size > maxLines}")
                if (exited) appendLine("exit_code=${process.exitValue()}")
                result?.failure?.let { appendLine("read_error=$it") }
                append(lines.takeLast(maxLines).joinToString("\n"))
            }
        } finally {
            runCatching { process.destroy() }
            runCatching { if (process.isAlive) process.destroyForcibly() }
            runCatching { process.outputStream.close() }
            // The daemon owns and closes stdout. The caller never blocks on a pipe read/close.
        }
    }

"####;

const STREAMING_ZIP_VERIFICATION: &str = r####"    private fun verifyExportZip(file: File, expectedActiveAppDataRaw: String) {
        check(file.isFile && file.length() > 0L) { "Data export ZIP is empty." }
        ZipFile(file).use { archive ->
            val seenNames = linkedSetOf<String>()
            val requiredEntries = mutableSetOf(
                "database/app_state.db",
                "app_data.json",
                STATE_FILE_NAME,
                "note_media/manifest.tsv"
            )
            val entries = archive.entries()
            while (entries.hasMoreElements()) {
                val entry = entries.nextElement()
                requireSafeZipEntryName(entry.name)
                check(seenNames.add(entry.name)) { "Data export ZIP contains duplicate entries." }
                requiredEntries.remove(entry.name)
                val expectedDigest = when {
                    entry.name.startsWith("note_media/pending_journals/") -> {
                        check(PENDING_JOURNAL_ZIP_ENTRY_REGEX.matches(entry.name)) {
                            "Data export pending-journal entry has an unsafe path."
                        }
                        entry.name.substringBeforeLast(".journal").substringAfterLast('_')
                    }
                    entry.name.startsWith("note_media/blobs/") -> entry.name.substringAfterLast('/')
                    else -> null
                }
                val expectedText = expectedActiveAppDataRaw.takeIf {
                    entry.name == "app_data.json" || entry.name == STATE_FILE_NAME
                }
                archive.getInputStream(entry).buffered().use { input ->
                    verifyExportEntry(input, entry, expectedText, expectedDigest)
                }
            }
            check(requiredEntries.isEmpty()) {
                "Data export ZIP is missing required entries: ${requiredEntries.joinToString()}"
            }
        }
    }

    internal fun verifyExportEntry(
        input: java.io.InputStream,
        entry: ZipEntry,
        expectedText: String?,
        expectedDigest: String?
    ) {
        // Encode the expected snapshot incrementally; compare the actual bytes, not a parsed copy.
        val characters = expectedText?.let(CharBuffer::wrap)
        val encoder = Charsets.UTF_8.newEncoder()
            .onMalformedInput(CodingErrorAction.REPLACE)
            .onUnmappableCharacter(CodingErrorAction.REPLACE)
        val expectedBytes = ByteBuffer.allocate(DEFAULT_BUFFER_SIZE).apply { limit(0) }
        fun nextExpectedByte(): Int {
            if (characters == null) return -1
            if (!expectedBytes.hasRemaining()) {
                expectedBytes.clear()
                val result = encoder.encode(characters, expectedBytes, true)
                if (result.isError) result.throwException()
                expectedBytes.flip()
                if (!expectedBytes.hasRemaining()) return -1
            }
            return expectedBytes.get().toInt() and 0xff
        }
        val digest = MessageDigest.getInstance("SHA-256")
        val crc = CRC32()
        val buffer = ByteArray(DEFAULT_BUFFER_SIZE)
        var observedSize = 0L
        while (true) {
            val read = input.read(buffer)
            if (read < 0) break
            observedSize += read.toLong()
            digest.update(buffer, 0, read)
            crc.update(buffer, 0, read)
            if (expectedText != null) {
                for (index in 0 until read) {
                    check((buffer[index].toInt() and 0xff) == nextExpectedByte()) {
                        "Data export active snapshot changed while packaging."
                    }
                }
            }
        }
        check(expectedText == null || nextExpectedByte() == -1) {
            "Data export active snapshot was truncated while packaging."
        }
        check(entry.size < 0L || observedSize == entry.size) {
            "Data export ZIP entry size mismatch: ${entry.name}"
        }
        check(entry.crc < 0L || crc.value == entry.crc) {
            "Data export ZIP entry CRC mismatch: ${entry.name}"
        }
        if (expectedDigest != null) {
            check(digest.digest().toHexString().equals(expectedDigest, ignoreCase = true)) {
                "Data export evidence digest mismatch: ${entry.name}"
            }
        }
    }

"####;

#[cfg(test)]
mod tests {
    use super::*;

    fn source(path: &str) -> &'static str {
        crate::kotlin_sources::SOURCES
            .iter()
            .find(|source| source.path == path)
            .expect("canonical generated source")
            .contents
    }

    #[test]
    fn backup_journal_mode_uses_result_cursor_before_transaction() {
        let rendered = render(DATABASE_PATH, source(DATABASE_PATH)).unwrap();
        let start = rendered
            .find("private fun copyDatabaseToAttachedBackup(")
            .unwrap();
        let body = &rendered[start..];
        let query = body
            .find("database.rawQuery(\"PRAGMA app_state_backup.journal_mode=DELETE\", null).use")
            .unwrap();
        let check = body
            .find("cursor.getString(0).equals(\"delete\", ignoreCase = true)")
            .unwrap();
        let transaction = body.find("database.beginTransaction()").unwrap();
        assert!(query < check && check < transaction);
        assert!(!rendered.contains("execSQL(\"PRAGMA app_state_backup.journal_mode=DELETE\")"));
        assert!(rendered.contains("verifyBackupDatabase(temporary)"));
        assert!(rendered.contains("PRAGMA quick_check(1)"));
    }

    #[test]
    fn diagnostic_export_is_independent_bounded_and_preserves_previous_process_evidence() {
        let rendered = render(EXPORTER_PATH, source(EXPORTER_PATH)).unwrap();
        let body = rendered
            .split("    fun recordDiagnosticLog(")
            .nth(1)
            .unwrap()
            .split("    private fun buildSummaryBlock(")
            .next()
            .unwrap();
        assert!(!body.contains("json.encodeToString(appData)"));
        assert!(!body.contains("AppStateDatabase"));
        assert!(!body.contains("persistNow"));
        assert!(!body.contains("snapshotFile("));
        assert!(body.contains("readRecentProcessExitReasons(appContext, sinceEpochMillis = null)"));
        for section in [
            "last_crash",
            "app_event_log",
            "recent_process_exit_reasons",
            "logcat_current_process",
            "logcat_recent_global",
        ] {
            assert!(
                body.contains(&format!("section(\"{section}\")")),
                "{section}"
            );
        }
        assert!(body.contains("status=collection_failed"));
        assert!(body.contains("minOf(size, DIAGNOSTIC_FILE_BYTES.toLong())"));
        let session = render(SESSION_PATH, source(SESSION_PATH)).unwrap();
        assert!(!session.contains("CrashLogStore.clear"));
        assert!(session.contains(".putBoolean(KEY_IS_ACTIVE, true)"));
        assert!(session.contains("fun finish(context: Context)"));
    }

    #[test]
    fn diagnostic_file_is_unique_closed_and_atomically_published() {
        let rendered = render(EXPORTER_PATH, source(EXPORTER_PATH)).unwrap();
        let body = rendered
            .split("    fun recordDiagnosticLog(")
            .nth(1)
            .unwrap()
            .split("    private fun collectDiagnosticSection(")
            .next()
            .unwrap();
        assert!(body.contains(
            "grid_timer_log_${fileTimestampFormatter.format(now)}_${UUID.randomUUID()}.txt"
        ));
        assert!(body.contains("File.createTempFile(\".${file.name}.\", \".tmp\", directory)"));
        let write = body
            .find("temporary.bufferedWriter(Charsets.UTF_8).use")
            .unwrap();
        let validate = body.find("check(temporary.length() > 0L)").unwrap();
        let publish = body
            .find("AtomicFileStore.replaceWithSyncedFile(file, temporary)")
            .unwrap();
        assert!(write < validate && validate < publish);
        assert!(body[publish..].contains("finally {\n                temporary.delete()"));
        assert!(!body.contains("file.writeText("));
    }

    #[test]
    fn logcat_reader_has_byte_budget_and_process_deadline_with_cleanup() {
        let rendered = render(EXPORTER_PATH, source(EXPORTER_PATH)).unwrap();
        let body = rendered
            .split("    private fun dumpLogcat(command:")
            .nth(1)
            .unwrap()
            .split("    private fun readRecentProcessExitReasons(")
            .next()
            .unwrap();
        assert!(!body.contains("readText()"));
        assert!(!body.contains("readBytes()"));
        assert!(body.contains("LOGCAT_BYTES - output.size()"));
        assert!(body.contains("minOf(buffer.size, remaining + 1)"));
        assert!(body.contains("reader.start()"));
        assert!(body.contains("process.waitFor(LOGCAT_TIMEOUT_MILLIS, TimeUnit.MILLISECONDS)"));
        assert!(body.contains("reader.join(500L)"));
        let cleanup = body.rsplit("} finally {").next().unwrap();
        assert!(cleanup.contains("process.destroy()"));
        assert!(cleanup.contains("process.destroyForcibly()"));
        assert!(body.contains("lines.takeLast(maxLines)"));
    }

    #[test]
    fn streamed_zip_validation_keeps_exact_snapshot_digest_crc_and_required_entries() {
        let rendered = render(EXPORTER_PATH, source(EXPORTER_PATH)).unwrap();
        let body = rendered
            .split("    private fun verifyExportZip(")
            .nth(1)
            .unwrap()
            .split("    private fun sha256(bytes:")
            .next()
            .unwrap();
        assert!(!body.contains("readBytes()"));
        assert!(!body.contains("expectedActiveAppDataRaw.toByteArray"));
        for check in [
            "seenNames.add(entry.name)",
            "requiredEntries.isEmpty()",
            "PENDING_JOURNAL_ZIP_ENTRY_REGEX.matches",
            "observedSize == entry.size",
            "crc.value == entry.crc",
            "equals(expectedDigest, ignoreCase = true)",
            "nextExpectedByte() == -1",
            "== nextExpectedByte()",
        ] {
            assert!(body.contains(check), "missing {check}");
        }
        assert!(body.contains("ByteBuffer.allocate(DEFAULT_BUFFER_SIZE)"));
        assert!(body.contains("expectedText?.let(CharBuffer::wrap)"));
        assert!(rendered.contains("createVerifiedBackup(databaseBackup)"));
        assert!(rendered.contains("Frozen database snapshot envelope mismatch"));
        assert!(rendered.contains("AtomicFileStore.replaceWithSyncedFile(file, temporaryZip)"));
    }
}
