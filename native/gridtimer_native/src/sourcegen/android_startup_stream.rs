// v2.22.49 - Stream the pinned SQLite statement through reusable verification buffers.
const REPOSITORY: &str = "com/ofairyo/gridtimer/data/TimerRepository.kt";
const DATABASE: &str = "com/ofairyo/gridtimer/data/AppStateDatabase.kt";
const SCREEN: &str = "com/ofairyo/gridtimer/ui/GridTimerScreen.kt";

fn replace(source: &mut String, old: &str, new: &str) -> Result<(), String> {
    if source.matches(old).count() != 1 {
        return Err(format!("startup stream anchor is not unique: {old}"));
    }
    *source = source.replacen(old, new, 1);
    Ok(())
}

pub fn render(path: &str, base: &str) -> Result<String, String> {
    let mut source = base.to_owned();
    match path {
        SCREEN => {
            replace(&mut source,
                "    val initialized by viewModel.isInitialized.collectAsState()\n    if (!initialized)",
                "    val initialized by viewModel.isInitialized.collectAsState()\n    androidx.activity.compose.ReportDrawnWhen { initialized }\n    if (!initialized)")?;
        }
        REPOSITORY => {
            replace(
                &mut source,
                "val result = stateDatabase.withVerifiedSnapshotReads {",
                "val result = stateDatabase.withVerifiedWorkspaceReads(workspaceKey) {",
            )?;
            replace(&mut source,
                "        var verifiedCurrentAccepted = false\n        stateDatabase.forEachSnapshot(workspaceKey) { stored ->",
                "        var verifiedCurrentAccepted = false\n        stateDatabase.forEachSnapshot(\n            workspaceKey,\n            skipVerifiedHistory = { authoritativeStartup && verifiedCurrentAccepted }\n        ) { stored ->")?;
            replace(&mut source,
                "            _isInitialized.value = true\n            logDiagnosticEvent(",
                "            _isInitialized.value = true\n            Log.i(\"StartupLatency\", \"ready elapsedMs=${android.os.SystemClock.elapsedRealtime() - initializationStarted}\")\n            logDiagnosticEvent(")?;
        }
        DATABASE => {
            replace(&mut source,
                "    fun forEachSnapshot(workspaceKey: String, action: (StoredStateSnapshot) -> Unit) {",
                "    fun forEachSnapshot(\n        workspaceKey: String,\n        skipVerifiedHistory: (() -> Boolean)? = null,\n        action: (StoredStateSnapshot) -> Unit\n    ) {")?;
            let start = source
                .find("    fun forEachSnapshot(")
                .ok_or("missing snapshot visitor")?;
            let end = source[start..]
                .find("    // Compatibility for bounded test fixtures")
                .ok_or("missing visitor end")?
                + start;
            let old = source[start..end].to_owned();
            let mut new = old.clone();
            replace(&mut new,
                "        database.visitSnapshotRows(\n            HISTORY_TABLE,",
                "        // The current must have decoded successfully in the repository.\n        // A missing/corrupt/future current still takes the complete recovery path.\n        if (snapshotReadWindow.get()?.canSkipVerifiedHistory(\n                workspaceKey, snapshotWriteStamp(), skipVerifiedHistory?.invoke() == true\n            ) == true) return\n        database.visitSnapshotRows(\n            HISTORY_TABLE,")?;
            replace(&mut source, &old, &new)?;
            replace(&mut source,
                "    private val snapshotReadWindow = ThreadLocal<SnapshotVerificationWindow>()",
                &format!("{WORKSPACE_READ}\n    private val snapshotReadWindow = ThreadLocal<SnapshotVerificationWindow>()"))?;
            replace(&mut source,
                "internal class SnapshotVerificationWindow {",
                "internal class SnapshotVerificationWindow {\n    private var nativeHistoryReceipt: Pair<String, Any>? = null\n\n    fun recordNativeHistory(workspace: String, stamp: Any) {\n        record(workspace, true, stamp)\n        nativeHistoryReceipt = workspace to stamp\n    }\n\n    fun canSkipVerifiedHistory(workspace: String, stamp: Any, acceptedCurrent: Boolean): Boolean =\n        acceptedCurrent && nativeHistoryReceipt == (workspace to stamp) &&\n            covers(workspace, true, stamp)\n")?;
            source.push_str(NATIVE_BRIDGE);
            source.push_str(VERIFICATION_PIPELINE);
        }
        _ => {}
    }
    Ok(source)
}

const WORKSPACE_READ: &str = r####"    fun <T> withVerifiedWorkspaceReads(workspaceKey: String, action: () -> T): T {
        val database = writableDatabase
        // All rows are read on Android's pinned primary connection. The Rust
        // verifier only receives this cursor's bytes, never opens the database.
        // Nested callers retain the existing verification/recovery behavior.
        val freshTransaction = !database.inTransaction()
        return withVerifiedSnapshotReads {
            if (freshTransaction) {
                val before = snapshotWriteStamp()
                val started = android.os.SystemClock.elapsedRealtime()
                val verifiedRows = runCatching { verifyNativeSnapshotRows(workspaceKey) }
                    .onFailure { android.util.Log.w("StartupLatency", "snapshotStream fallback: ${it.javaClass.simpleName}") }
                    .getOrDefault(-1L)
                val after = snapshotWriteStamp()
                if (verifiedRows >= 0 && before == after) {
                    snapshotReadWindow.get()?.recordNativeHistory(workspaceKey, after)
                }
                android.util.Log.i("StartupLatency",
                    "snapshotStream rows=$verifiedRows elapsedMs=${android.os.SystemClock.elapsedRealtime() - started}")
            }
            action()
        }
    }

    private fun verifyNativeSnapshotRows(workspaceKey: String): Long {
        if (!NativeSnapshotVerifier.available) return -1L
        val database = writableDatabase
        check(database.inTransaction())
        val expectedRows = arrayOf(CURRENT_TABLE, HISTORY_TABLE).sumOf { table ->
            database.rawQuery("SELECT COUNT(*) FROM $table WHERE workspace_key = ?",
                arrayOf(workspaceKey)).use { cursor ->
                check(cursor.moveToFirst())
                cursor.getLong(0)
            }
        }
        val workers = Runtime.getRuntime().availableProcessors().coerceIn(1, 4)
        val nativeNanos = java.util.concurrent.atomic.AtomicLong()
        val buffers = SnapshotBufferPool(workers * 2 + 1)
        var payloadBytes = 0L
        val started = android.os.SystemClock.elapsedRealtime()
        val verified = verifySnapshotPipeline(workers, expectedRows) { submit ->
            for (table in arrayOf(CURRENT_TABLE, HISTORY_TABLE)) {
                if (android.os.Build.VERSION.SDK_INT >= 35) {
                    payloadBytes += visitRawSnapshots(database, table, workspaceKey, buffers, submit, nativeNanos)
                    continue
                }
                // CursorWindow.getBlob(TEXT) includes the internal string terminator.
                // CAST returns the exact UTF-8 payload as BLOB, so SHA-256 continues
                // to cover the persisted JSON bytes without trimming/changing data.
                val columns = (if (table == CURRENT_TABLE) SNAPSHOT_COLUMNS else HISTORY_QUERY_COLUMNS)
                    .map { column -> if (column == "app_data_json")
                        "CAST(app_data_json AS BLOB) AS app_data_json" else column }
                    .toTypedArray()
                visitNativeSnapshotBatches(database, table, columns, workspaceKey) { cursor ->
                    val owner = cursor.getString(cursor.getColumnIndexOrThrow("workspace_key"))
                    val rowId = if (table == CURRENT_TABLE) owner
                        else cursor.getLong(cursor.getColumnIndexOrThrow("id")).toString()
                    val timestamp = if (table == CURRENT_TABLE) "updated_at_epoch_millis"
                        else "created_at_epoch_millis"
                    val prefix = listOf(
                        "gridtimer-snapshot-envelope-v1", table, rowId, owner,
                        cursor.getInt(cursor.getColumnIndexOrThrow("schema_version")).toString(),
                        cursor.getLong(cursor.getColumnIndexOrThrow("revision")).toString(),
                        cursor.getInt(cursor.getColumnIndexOrThrow("item_count")).toString(),
                        cursor.getLong(cursor.getColumnIndexOrThrow(timestamp)).toString()
                    ).joinToString(separator = "\u0000", postfix = "\u0000")
                    check(owner == workspaceKey) { "Snapshot workspace changed during its read" }
                    val raw = cursor.getBlob(cursor.getColumnIndexOrThrow("app_data_json"))
                    val digest = cursor.getString(cursor.getColumnIndexOrThrow("sha256"))
                    val envelope = cursor.getString(cursor.getColumnIndexOrThrow("envelope_sha256"))
                    payloadBytes += raw?.size ?: 0
                    // These immutable values own their bytes; no worker touches
                    // Android's transaction, cursor or mutable receipt window.
                    submit {
                        val began = android.os.Debug.threadCpuTimeNanos()
                        try { NativeSnapshotVerifier.verify(raw, raw?.size ?: 0, prefix, digest, envelope, APP_DATA_SCHEMA_VERSION) }
                        finally { nativeNanos.addAndGet(android.os.Debug.threadCpuTimeNanos() - began) }
                    }
                }
            }
        }
        android.util.Log.i("StartupLatency", "snapshotPipeline workers=$workers bytes=$payloadBytes nativeCpuMs=${nativeNanos.get() / 1_000_000} elapsedMs=${android.os.SystemClock.elapsedRealtime() - started}")
        return verified
    }

    @androidx.annotation.RequiresApi(35)
    private fun visitRawSnapshots(
        database: SQLiteDatabase, table: String, workspaceKey: String,
        buffers: SnapshotBufferPool, submit: (() -> Boolean) -> Unit,
        nativeNanos: java.util.concurrent.atomic.AtomicLong
    ): Long {
        check(database.inTransaction())
        val timestamp = if (table == CURRENT_TABLE) "updated_at_epoch_millis" else "created_at_epoch_millis"
        val identity = if (table == CURRENT_TABLE) "workspace_key" else "id"
        val sql = "SELECT $identity, workspace_key, schema_version, revision, item_count, $timestamp, " +
            "CAST(app_data_json AS BLOB) AS app_data_json, sha256, envelope_sha256 " +
            "FROM $table NOT INDEXED WHERE workspace_key = ? ORDER BY rowid"
        var bytes = 0L
        // This public API uses the same thread's already-pinned transaction.
        // It avoids CursorWindow copies and never opens another SQLite connection.
        database.createRawStatement(sql).use { row ->
            row.bindText(1, workspaceKey)
            while (row.step()) {
                val owner = row.getColumnText(1)
                check(owner == workspaceKey) { "Snapshot workspace changed during its read" }
                val rowId = if (table == CURRENT_TABLE) owner else row.getColumnLong(0).toString()
                val prefix = listOf("gridtimer-snapshot-envelope-v1", table, rowId, owner,
                    row.getColumnInt(2).toString(), row.getColumnLong(3).toString(),
                    row.getColumnInt(4).toString(), row.getColumnLong(5).toString())
                    .joinToString(separator = "\u0000", postfix = "\u0000")
                val digest = row.getColumnText(7)
                val envelope = row.getColumnText(8)
                val length = row.getColumnLength(6)
                bytes += length
                buffers.submit(length,
                    read = { buffer -> row.readColumnBlob(6, buffer, 0, length, 0) },
                    submit = submit,
                    verify = { buffer, size ->
                        val began = android.os.Debug.threadCpuTimeNanos()
                        try { NativeSnapshotVerifier.verify(buffer, size, prefix, digest, envelope, APP_DATA_SCHEMA_VERSION) }
                        finally { nativeNanos.addAndGet(android.os.Debug.threadCpuTimeNanos() - began) }
                    })
            }
        }
        return bytes
    }

    private fun visitNativeSnapshotBatches(
        database: SQLiteDatabase, table: String, columns: Array<String>,
        workspaceKey: String, action: (Cursor) -> Unit
    ) {
        check(database.inTransaction())
        val sizedWindow = android.os.Build.VERSION.SDK_INT >= 28
        val batchSize = if (sizedWindow) 16 else 1
        var afterRow: Long? = null
        do {
            val selection = "workspace_key = ?" + if (afterRow == null) "" else " AND rowid > ?"
            val arguments = if (afterRow == null) arrayOf(workspaceKey)
                else arrayOf(workspaceKey, afterRow.toString())
            var rows = 0
            // The existing workspace/time index would sort the remaining payloads
            // again for every page. Force an advancing primary-key scan instead.
            database.query("$table NOT INDEXED", arrayOf("rowid AS scan_row_id", *columns), selection,
                arguments, null, null, "rowid ASC", batchSize.toString()).use { cursor ->
                if (sizedWindow && cursor is android.database.sqlite.SQLiteCursor) {
                    cursor.window = android.database.CursorWindow("startup-batch", 16L * 1024 * 1024)
                    cursor.setFillWindowForwardOnly(true)
                }
                while (cursor.moveToNext()) {
                    val id = cursor.getLong(cursor.getColumnIndexOrThrow("scan_row_id"))
                    check(afterRow == null || id > afterRow!!) { "Snapshot scan failed to advance" }
                    afterRow = id
                    action(cursor)
                    rows++
                }
            }
        } while (rows == batchSize)
    }
"####;

const VERIFICATION_PIPELINE: &str = r####"

// A buffer stays exclusively owned by its verifier until that call returns.
// Reuse removes one large Java allocation per history row; the explicit length
// excludes bytes left over from a previous, larger snapshot.
internal class SnapshotBufferPool(slots: Int) {
    private val free = java.util.concurrent.ArrayBlockingQueue<ByteArray>(slots).apply {
        repeat(slots) { add(ByteArray(0)) }
    }

    fun submit(
        length: Int, read: (ByteArray) -> Int,
        submit: (() -> Boolean) -> Unit, verify: (ByteArray, Int) -> Boolean
    ) {
        require(length > 0)
        var buffer = free.take()
        var handedOff = false
        try {
            if (buffer.size < length) {
                val capacity = minOf(Int.MAX_VALUE.toLong(),
                    maxOf(length.toLong(), buffer.size.toLong() * 2)).toInt()
                buffer = ByteArray(capacity)
            }
            check(read(buffer) == length) { "Snapshot payload was not read completely" }
            val owned = buffer
            submit {
                try { verify(owned, length) }
                finally { free.add(owned) }
            }
            handedOff = true
        } finally {
            if (!handedOff) free.add(buffer)
        }
    }
}

// At most two jobs per worker are pending. Cursor reading stays on the primary
// transaction thread; only immutable snapshot bytes cross to verification workers.
internal fun verifySnapshotPipeline(
    workers: Int, expectedRows: Long, produce: ((() -> Boolean) -> Unit) -> Unit
): Long {
    require(workers in 1..4)
    require(expectedRows >= 0)
    val executor = java.util.concurrent.Executors.newFixedThreadPool(workers)
    val pending = java.util.ArrayDeque<java.util.concurrent.Future<Boolean>>()
    var verified = 0L
    fun awaitOne() {
        check(pending.removeFirst().get()) { "Snapshot verification rejected a row" }
        verified++
    }
    try {
        produce { verify ->
            while (pending.size >= workers * 2) awaitOne()
            pending.addLast(executor.submit(java.util.concurrent.Callable { verify() }))
        }
        while (pending.isNotEmpty()) awaitOne()
        check(verified == expectedRows) { "Snapshot scan did not cover the complete transaction" }
        return verified
    } finally {
        // A rejected or interrupted scan cannot outlive its transaction. Always
        // join already submitted workers before recovery or the receipt can run.
        executor.shutdown()
        var interrupted = false
        while (!executor.isTerminated) {
            try { executor.awaitTermination(1, java.util.concurrent.TimeUnit.SECONDS) }
            catch (_: InterruptedException) { interrupted = true }
        }
        if (interrupted) Thread.currentThread().interrupt()
    }
}
"####;

const NATIVE_BRIDGE: &str = r####"

private object NativeSnapshotVerifier {
    val available: Boolean = runCatching {
        System.loadLibrary("gridtimer_native")
        true
    }.onFailure { android.util.Log.w("StartupLatency", "nativeLoad fallback: ${it.javaClass.simpleName}") }
        .getOrDefault(false)

    fun verify(raw: ByteArray?, length: Int, prefix: String, digest: String?, envelope: String?, schema: Int): Boolean =
        available && raw != null && digest != null && envelope != null &&
            runCatching { nativeVerify(raw, length, prefix, digest, envelope, schema) }
                .onFailure { android.util.Log.w("StartupLatency", "nativeVerify fallback: ${it.javaClass.simpleName}") }
                .getOrDefault(false)

    @JvmStatic private external fun nativeVerify(
        raw: ByteArray, length: Int, prefix: String, digest: String, envelope: String, schema: Int
    ): Boolean
}
"####;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_cannot_escape_the_existing_transaction_or_replace_recovery_selection() {
        let base = super::super::android_local_startup::tests::rendered(DATABASE);
        let base = super::super::android_timer_action::render(DATABASE, &base).unwrap();
        let source = render(DATABASE, &base).unwrap();
        assert!(source.contains("val freshTransaction = !database.inTransaction()"));
        assert!(source.contains("if (freshTransaction)"));
        assert!(source.contains("verifiedRows >= 0 && before == after"));
        assert!(source.contains("CAST(app_data_json AS BLOB) AS app_data_json"));
        assert!(source.contains("finally {\n                snapshotReadWindow.remove()"));
        let base = super::super::android_local_startup::tests::rendered(REPOSITORY);
        let source = render(REPOSITORY, &base).unwrap();
        assert!(source
            .contains("skipVerifiedHistory = { authoritativeStartup && verifiedCurrentAccepted }"));
        assert!(source.contains(
            "if (candidate.source == \"sqlite-current\") verifiedCurrentAccepted = true"
        ));
        assert!(source.contains(
            "futureCurrentSurface ?: databaseCurrent ?: jsonPrimaryAuthority ?: recovery"
        ));
    }
}
