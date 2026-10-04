// v2.22.49.8 - Authenticate compact canonical history without expanding it at launch.
const DATABASE: &str = "com/ofairyo/gridtimer/data/AppStateDatabase.kt";
const DIAGNOSTICS: &str = "com/ofairyo/gridtimer/diagnostics/DiagnosticsExporter.kt";

fn replace(source: &mut String, old: &str, new: &str) -> Result<(), String> {
    if source.matches(old).count() != 1 {
        return Err(format!("history storage anchor is not unique: {old}"));
    }
    *source = source.replacen(old, new, 1);
    Ok(())
}

pub fn render(path: &str, base: &str) -> Result<String, String> {
    let mut source = base.to_owned();
    if path == DIAGNOSTICS {
        replace(
            &mut source,
            "val raw = cursor.getString(4)",
            "val raw = com.ofairyo.gridtimer.data.SnapshotPayload.decode(cursor.getString(4))",
        )?;
    }
    if path == "com/ofairyo/gridtimer/ui/GridTimerScreen.kt" {
        replace(
            &mut source,
            "    androidx.activity.compose.ReportDrawnWhen { initialized }",
            r#"    androidx.activity.compose.ReportDrawnWhen { initialized }
    val loadingGateStarted = androidx.compose.runtime.remember {
        if (initialized) null else android.os.SystemClock.elapsedRealtime()
    }
    androidx.compose.runtime.LaunchedEffect(initialized) {
        if (initialized) {
            androidx.compose.runtime.withFrameNanos { }
            val elapsed = loadingGateStarted?.let { android.os.SystemClock.elapsedRealtime() - it } ?: 0L
            android.util.Log.i("StartupLatency", "loadingGate elapsedMs=$elapsed")
        }
    }"#,
        )?;
    }
    if path != DATABASE {
        return Ok(source);
    }
    replace(
        &mut source,
        "const val DATABASE_VERSION = 4",
        "const val DATABASE_VERSION = 5",
    )?;
    replace(&mut source, "        setWriteAheadLoggingEnabled(true)",
        "        NativeSnapshotVerifier.initialize(context.applicationContext)\n        setWriteAheadLoggingEnabled(true)")?;
    // The extra column is a migration queue, never an integrity receipt. Every
    // actual payload (including rows marked complete) is still verified at launch.
    replace(&mut source, "        createSnapshotQuarantineSchema(database)\n    }\n\n    override fun onUpgrade",
        "        createSnapshotQuarantineSchema(database)\n        createHistoryEncodingSchema(database)\n    }\n\n    override fun onUpgrade")?;
    replace(&mut source, "    private fun createSnapshotQuarantineSchema(database: SQLiteDatabase) {",
        "    private fun createHistoryEncodingSchema(database: SQLiteDatabase) {\n        database.execSQL(\"ALTER TABLE $HISTORY_TABLE ADD COLUMN storage_encoding INTEGER NOT NULL DEFAULT 0\")\n        database.execSQL(\"CREATE INDEX history_encoding_queue ON $HISTORY_TABLE(workspace_key, storage_encoding, id)\")\n        database.execSQL(\"CREATE TABLE history_codec_key_state (id INTEGER PRIMARY KEY, key_id TEXT NOT NULL)\")\n    }\n\n    private fun createSnapshotQuarantineSchema(database: SQLiteDatabase) {")?;
    replace(&mut source, "                rebuildVersionTwoQuarantine(database)\n            }\n        }\n    }",
        "                rebuildVersionTwoQuarantine(database)\n            }\n        }\n        if (oldVersion < 5) createHistoryEncodingSchema(database)\n    }")?;
    replace(&mut source, "                database.execSQL(\"PRAGMA app_state_backup.user_version=$DATABASE_VERSION\")",
        "                database.execSQL(\"ALTER TABLE app_state_backup.snapshot_history ADD COLUMN storage_encoding INTEGER NOT NULL DEFAULT 0\")\n                database.execSQL(\"CREATE INDEX app_state_backup.history_encoding_queue ON snapshot_history(workspace_key, storage_encoding, id)\")\n                database.execSQL(\"CREATE TABLE app_state_backup.history_codec_key_state (id INTEGER PRIMARY KEY, key_id TEXT NOT NULL)\")\n                database.execSQL(\"PRAGMA app_state_backup.user_version=$DATABASE_VERSION\")")?;
    replace(&mut source, "        val freshTransaction = !database.inTransaction()\n        return withVerifiedSnapshotReads",
        "        val freshTransaction = !database.inTransaction()\n        if (freshTransaction) runCatching { compactHistory(workspaceKey) }.onFailure {\n            android.util.Log.w(\"StartupLatency\", \"historyCompact incomplete; retaining original rows\", it)\n        }\n        return withVerifiedSnapshotReads")?;
    replace(&mut source, "    private val snapshotReadWindow = ThreadLocal<SnapshotVerificationWindow>()",
        &format!("{MIGRATION}\n    private val snapshotReadWindow = ThreadLocal<SnapshotVerificationWindow>()"))?;
    replace(&mut source, "ContentValues().apply { put(\"envelope_sha256\", historyEnvelope) }",
        "ContentValues().apply {\n                        put(\"envelope_sha256\", historyEnvelope)\n                        val packed = packHistory(current.appDataJson, historyId.toString(), workspaceKey, current.schemaVersion, current.revision, current.itemCount, now, currentDigest, historyEnvelope)\n                        put(\"app_data_json\", packed)\n                        put(\"storage_encoding\", if (SnapshotPayload.isPacked(packed)) 1 else 0)\n                    }")?;
    let history_start = source
        .find("            historyIds.forEach { id ->")
        .ok_or("missing history rewrite")?;
    let history_end = history_start
        + source[history_start..]
            .find("            val quarantineIds")
            .ok_or("missing quarantine rewrite")?;
    let old_history = source[history_start..history_end].to_owned();
    let mut new_history = old_history.clone();
    replace(&mut new_history, "cursor.getString(0).also { raw -> requirePrivacySnapshotSchema(raw, cursor.getInt(1)) }",
        "SnapshotPayload.decode(cursor.getString(0)).also { raw -> requirePrivacySnapshotSchema(raw, cursor.getInt(1)) }")?;
    replace(&mut source, &old_history, &new_history)?;
    replace(&mut source, "                        timestampColumn = \"created_at_epoch_millis\"\n                    ),\n                    \"id = ?\"",
        "                        timestampColumn = \"created_at_epoch_millis\"\n                    ).apply {\n                        val packed = packHistory(rewritten.appDataJson, id.toString(), workspaceKey, rewritten.schemaVersion, rewritten.revision, rewritten.itemCount, timestamp, digest, envelope)\n                        put(\"app_data_json\", packed)\n                        put(\"storage_encoding\", if (SnapshotPayload.isPacked(packed)) 1 else 0)\n                    },\n                    \"id = ?\"")?;
    replace(&mut source, "val raw = cursor.getString(0) ?: error(\"Backup contains an empty snapshot.\")",
        "val raw = SnapshotPayload.decode(cursor.getString(0) ?: error(\"Backup contains an empty snapshot.\"))")?;
    replace(
        &mut source,
        "appDataJson = cursor.getString(6),",
        "appDataJson = SnapshotPayload.decode(cursor.getString(6)),",
    )?;
    replace(&mut source, "val raw = cursor.getString(cursor.getColumnIndexOrThrow(\"app_data_json\"))\n                        ?: error(\"History snapshot JSON is null.\")",
        "val raw = SnapshotPayload.decode(cursor.getString(cursor.getColumnIndexOrThrow(\"app_data_json\"))\n                        ?: error(\"History snapshot JSON is null.\"))")?;
    replace(&mut source, "val raw = getString(getColumnIndexOrThrow(\"app_data_json\")) ?: return null",
        "val raw = SnapshotPayload.decode(getString(getColumnIndexOrThrow(\"app_data_json\")) ?: return null)")?;
    replace(
        &mut source,
        "private object NativeSnapshotVerifier {",
        &format!("private object NativeSnapshotVerifier {{\n{BRIDGE}"),
    )?;
    replace(&mut source, "nativeVerify(raw, length, prefix, digest, envelope, schema)",
        "if (raw.isNotEmpty() && raw[0] == 33.toByte()) nativeVerifyPacked(raw, length, prefix, digest, envelope, schema, key)\n            else nativeVerify(raw, length, prefix, digest, envelope, schema)")?;
    source.push_str(PAYLOAD);
    Ok(source)
}

const MIGRATION: &str = r####"
    private fun historyPrefix(id: String, owner: String, schema: Int, revision: Long, count: Int, time: Long): String =
        listOf("gridtimer-snapshot-envelope-v1", HISTORY_TABLE, id, owner, schema.toString(),
            revision.toString(), count.toString(), time.toString()).joinToString("\u0000", postfix = "\u0000")

    private fun packHistory(raw: String, id: String, owner: String, schema: Int, revision: Long,
        count: Int, time: Long, digest: String, envelope: String): String =
        NativeSnapshotVerifier.seal(raw, historyPrefix(id, owner, schema, revision, count, time), digest, envelope) ?: raw

    @Synchronized
    private fun compactHistory(workspaceKey: String) {
        if (!NativeSnapshotVerifier.available || NativeSnapshotVerifier.key.size != 32) return
        val database = writableDatabase
        check(!database.inTransaction())
        val keyId = sha256(android.util.Base64.encodeToString(NativeSnapshotVerifier.key, android.util.Base64.NO_WRAP))
        withSnapshotWriteTransaction {
            val previousKey = database.rawQuery("SELECT key_id FROM history_codec_key_state WHERE id = 1", null).use {
                if (it.moveToFirst()) it.getString(0) else null
            }
            if (previousKey != keyId) {
                // Backup restore / key replacement pays full validation once and
                // reseals each canonical payload before its next fast startup.
                database.execSQL("UPDATE $HISTORY_TABLE SET storage_encoding = 0 WHERE storage_encoding != 0")
                database.execSQL("INSERT OR REPLACE INTO history_codec_key_state(id, key_id) VALUES(1, ?)", arrayOf(keyId))
            }
        }
        var after = 0L
        var changed = 0
        var beforeBytes = 0L
        var afterBytes = 0L
        val started = android.os.SystemClock.elapsedRealtime()
        while (true) {
            val ids = database.rawQuery("SELECT id FROM $HISTORY_TABLE WHERE workspace_key = ? AND storage_encoding = 0 AND id > ? ORDER BY id LIMIT 16",
                arrayOf(workspaceKey, after.toString())).use { cursor ->
                buildList { while (cursor.moveToNext()) add(cursor.getLong(0)) }
            }
            if (ids.isEmpty()) break
            withSnapshotWriteTransaction {
                for (id in ids) {
                    after = id
                    database.query(HISTORY_TABLE, HISTORY_QUERY_COLUMNS, "id = ? AND workspace_key = ?",
                        arrayOf(id.toString(), workspaceKey), null, null, null, "1").use { cursor ->
                        if (cursor is android.database.sqlite.SQLiteCursor && android.os.Build.VERSION.SDK_INT >= 28)
                            cursor.window = android.database.CursorWindow("history-pack", 16L * 1024 * 1024)
                        check(cursor.moveToFirst())
                        val stored = cursor.getString(cursor.getColumnIndexOrThrow("app_data_json"))
                        val raw = SnapshotPayload.decode(stored)
                        val prefix = historyPrefix(id.toString(), workspaceKey,
                            cursor.getInt(cursor.getColumnIndexOrThrow("schema_version")),
                            cursor.getLong(cursor.getColumnIndexOrThrow("revision")),
                            cursor.getInt(cursor.getColumnIndexOrThrow("item_count")),
                            cursor.getLong(cursor.getColumnIndexOrThrow("created_at_epoch_millis")))
                        val digest = cursor.getString(cursor.getColumnIndexOrThrow("sha256"))
                        val envelope = cursor.getString(cursor.getColumnIndexOrThrow("envelope_sha256"))
                        // A future, corrupt or unrepresentable row stays untouched and
                        // is handled by the original recovery / read-only barriers.
                        val packed = NativeSnapshotVerifier.seal(raw, prefix, digest, envelope) ?: return@use
                        check(database.update(HISTORY_TABLE, ContentValues().apply {
                            put("app_data_json", packed)
                            put("storage_encoding", 1)
                        }, "id = ? AND workspace_key = ?", arrayOf(id.toString(), workspaceKey)) == 1)
                        val roundTrip = database.rawQuery("SELECT app_data_json FROM $HISTORY_TABLE WHERE id = ?", arrayOf(id.toString())).use {
                            if (it is android.database.sqlite.SQLiteCursor && android.os.Build.VERSION.SDK_INT >= 28)
                                it.window = android.database.CursorWindow("history-pack-check", 16L * 1024 * 1024)
                            check(it.moveToFirst()); it.getString(0)
                        }
                        check(roundTrip == packed) { "Compressed history write did not round-trip" }
                        changed++
                        beforeBytes += raw.toByteArray(Charsets.UTF_8).size
                        afterBytes += packed.length
                    }
                }
            }
            if (changed > 0 && changed % 128 == 0) android.util.Log.i("StartupLatency", "historyCompact progress=$changed")
        }
        if (changed > 0) android.util.Log.i("StartupLatency", "historyCompact rows=$changed rawBytes=$beforeBytes packedBytes=$afterBytes elapsedMs=${android.os.SystemClock.elapsedRealtime() - started}")
    }
"####;

const BRIDGE: &str = r####"
    @Volatile var key: ByteArray = ByteArray(0)
        private set

    @Synchronized fun initialize(context: Context) {
        if (key.isNotEmpty() || !available) return
        val file = File(context.noBackupFilesDir, "history_codec_key_v1")
        key = runCatching {
            val existing = runCatching {
                android.util.Base64.decode(file.readText(), android.util.Base64.NO_WRAP).also { check(it.size == 32) }
            }.getOrNull()
            existing ?: ByteArray(32).also { bytes ->
                java.security.SecureRandom().nextBytes(bytes)
                AtomicFileStore.writeText(file, android.util.Base64.encodeToString(bytes, android.util.Base64.NO_WRAP))
            }
        }.getOrDefault(ByteArray(0))
        // Missing/unreadable keys use full decoding and original digest checks.
        // They never discard histories or make a cached proof authoritative.
    }

    fun seal(raw: String, prefix: String, digest: String, envelope: String): String? =
        if (!available || key.size != 32) null else runCatching {
            nativeSeal(raw, prefix, digest, envelope, APP_DATA_SCHEMA_VERSION, key)
        }.getOrNull()

    fun decode(packed: String): String? = if (!available) null else runCatching {
        nativeDecode(packed, APP_DATA_SCHEMA_VERSION, key)
    }.getOrNull()

    @JvmStatic private external fun nativeSeal(raw: String, prefix: String, expected: String, envelope: String, supported: Int, key: ByteArray): String?
    @JvmStatic private external fun nativeDecode(packed: String, supported: Int, key: ByteArray): String?
    @JvmStatic private external fun nativeVerifyPacked(raw: ByteArray, length: Int, prefix: String, expected: String, envelope: String, supported: Int, key: ByteArray): Boolean
"####;

const PAYLOAD: &str = r####"

internal object SnapshotPayload {
    fun isPacked(raw: String): Boolean = raw.startsWith("!gridtimer-history-v1:")
    fun decode(raw: String): String = if (!isPacked(raw)) raw else
        checkNotNull(NativeSnapshotVerifier.decode(raw)) { "History payload cannot be verified; original bytes retained" }
}
"####;
