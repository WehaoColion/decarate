// v2.22.39 - Redact encrypted-note recovery copies one payload at a time.
// v2.22.33 - Stream recovery snapshots instead of retaining every decoded history.

const DATABASE_PATH: &str = "com/ofairyo/gridtimer/data/AppStateDatabase.kt";
const REPOSITORY_PATH: &str = "com/ofairyo/gridtimer/data/TimerRepository.kt";

pub fn render(path: &str, base: &str) -> Result<String, String> {
    match path {
        DATABASE_PATH => render_database(base),
        REPOSITORY_PATH => render_repository(base),
        _ => Ok(base.to_owned()),
    }
}

fn replace(source: &mut String, old: &str, new: &str) -> Result<(), String> {
    if source.matches(old).count() != 1 {
        return Err(format!("snapshot memory anchor changed: {old}"));
    }
    *source = source.replacen(old, new, 1);
    Ok(())
}

fn section(source: &str, start: &str, end: &str) -> Result<String, String> {
    let from = source
        .find(start)
        .ok_or_else(|| format!("missing {start}"))?;
    let to = source[from..]
        .find(end)
        .ok_or_else(|| format!("missing {end}"))?
        + from;
    Ok(source[from..to].to_owned())
}

fn render_database(base: &str) -> Result<String, String> {
    let mut source = base.to_owned();
    let original = section(
        base,
        "    fun readSnapshots(",
        "    fun forEachRetainedSnapshot(",
    )?;
    let mut streamed = original.clone();
    replace(
        &mut streamed,
        "    fun readSnapshots(workspaceKey: String): List<StoredStateSnapshot> {",
        "    fun forEachSnapshot(workspaceKey: String, action: (StoredStateSnapshot) -> Unit) {",
    )?;
    replace(
        &mut streamed,
        "        val snapshots = mutableListOf<StoredStateSnapshot>()\n",
        "",
    )?;
    streamed = streamed.replace("?.let(snapshots::add)", "?.let(action)");
    replace(&mut streamed, "        return snapshots\n", "")?;
    replace(&mut source, &original, &format!("{streamed}    // Compatibility for bounded test fixtures; production recovery uses the visitor.\n    fun readSnapshots(workspaceKey: String): List<StoredStateSnapshot> = buildList {{\n        forEachSnapshot(workspaceKey) {{ add(it) }}\n    }}\n\n"))?;

    let original = section(
        base,
        "    fun readQuarantinedCurrentSnapshots(",
        "    fun hasCurrentSnapshotRow(",
    )?;
    let mut streamed = original.clone();
    replace(&mut streamed,
        "    fun readQuarantinedCurrentSnapshots(workspaceKey: String): List<StoredStateSnapshot> {",
        "    fun forEachQuarantinedCurrentSnapshot(workspaceKey: String, action: (StoredStateSnapshot) -> Unit) {")?;
    replace(
        &mut streamed,
        "        return readableDatabase.rawQuery(",
        "        readableDatabase.rawQuery(",
    )?;
    replace(&mut streamed,
        "            buildList {\n                while (cursor.moveToNext()) {\n                    val id = cursor.getLong(cursor.getColumnIndexOrThrow(\"id\"))\n                    cursor.toStoredSnapshot(\"sqlite-quarantine-current:$id\")?.let(::add)\n                }\n            }",
        "            while (cursor.moveToNext()) {\n                val id = cursor.getLong(cursor.getColumnIndexOrThrow(\"id\"))\n                cursor.toStoredSnapshot(\"sqlite-quarantine-current:$id\")?.let(action)\n            }")?;
    replace(&mut source, &original, &streamed)?;
    for (table, name) in [
        ("HISTORY_TABLE", "history"),
        ("QUARANTINE_TABLE", "quarantine"),
    ] {
        let old = format!(
            "            val {name}Rows = mutableListOf<Pair<Long, String>>()\n            database.query(\n                {table},\n                arrayOf(\"id\", \"app_data_json\"),\n                \"workspace_key = ?\",\n                arrayOf(workspaceKey),\n                null,\n                null,\n                null\n            ).use {{ cursor ->\n                while (cursor.moveToNext()) {{\n                    {name}Rows += cursor.getLong(0) to cursor.getString(1)\n                }}\n            }}\n            {name}Rows.forEach {{ (id, raw) ->"
        );
        let new = format!(
            "            // Keep only row identifiers across iterations. Each potentially large\n            // JSON payload is released before the next row is read. The enclosing\n            // transaction retains the existing all-or-nothing privacy guarantee.\n            val {name}Ids = mutableListOf<Long>()\n            database.query(\n                {table},\n                arrayOf(\"id\"),\n                \"workspace_key = ?\",\n                arrayOf(workspaceKey),\n                null,\n                null,\n                \"id ASC\"\n            ).use {{ cursor ->\n                while (cursor.moveToNext()) {{\n                    {name}Ids += cursor.getLong(0)\n                }}\n            }}\n            {name}Ids.forEach {{ id ->\n                val raw = database.query(\n                    {table}, arrayOf(\"app_data_json\"),\n                    \"id = ? AND workspace_key = ?\",\n                    arrayOf(id.toString(), workspaceKey),\n                    null, null, null, \"1\"\n                ).use {{ cursor ->\n                    check(cursor.moveToFirst()) {{ \"Snapshot disappeared during privacy redaction.\" }}\n                    cursor.getString(0)\n                }}"
        );
        replace(&mut source, &old, &new)?;
    }
    Ok(source)
}

fn render_repository(base: &str) -> Result<String, String> {
    let mut source = base.to_owned();
    replace(&mut source,
        "        val databaseCandidates = stateDatabase.readSnapshots(workspaceKey)\n            .mapNotNull(::decodeStoredCandidate)\n        val quarantinedCandidates = stateDatabase.readQuarantinedCurrentSnapshots(workspaceKey)\n            .mapNotNull(::decodeStoredCandidate)",
        "        val databaseCandidates = readDatabaseCandidateSurfaces(workspaceKey)\n        val quarantinedCandidates = readQuarantinedFutureCandidates(workspaceKey)")?;
    replace(&mut source,
        "        val databaseCandidates = runCatching {\n            stateDatabase.readSnapshots(migrationWorkspaceKey)\n        }.getOrDefault(emptyList())\n            .mapNotNull(::decodeStoredCandidate)",
        "        val databaseCandidates = runCatching {\n            readDatabaseCandidateSurfaces(migrationWorkspaceKey)\n        }.getOrDefault(emptyList())")?;
    replace(&mut source,
        "        val targetHasAnyState = stateDatabase.readSnapshots(targetWorkspaceKey).isNotEmpty() ||",
        "        val targetHasAnyState = stateDatabase.hasAnySnapshotRows(targetWorkspaceKey) ||")?;
    replace(&mut source,
        "            stateDatabase.readSnapshots(GUEST_WORKSPACE_KEY)\n                .mapNotNull(::decodeStoredCandidate) +",
        "            readDatabaseCandidateSurfaces(GUEST_WORKSPACE_KEY) +")?;
    replace(&mut source,
        "                    stateDatabase.readSnapshots(expectedWorkspaceKey)\n                        .firstOrNull { it.source == \"sqlite-current\" }\n                        ?.appDataJson == expectedJson",
        "                    stateDatabase.currentSnapshotMatches(\n                        workspaceKey = expectedWorkspaceKey,\n                        appDataJson = expectedJson,\n                        schemaVersion = APP_DATA_SCHEMA_VERSION,\n                        revision = appDataRevisionMillis(updated),\n                        itemCount = persistedItemCount(updated)\n                    )")?;
    replace(&mut source,
        "        val storedDatabaseSnapshots = runCatching {\n            stateDatabase.readSnapshots(workspaceKey)",
        "        val storedDatabaseCandidates = runCatching {\n            readDatabaseCandidateSurfaces(workspaceKey)")?;
    replace(&mut source,
        "        val quarantinedCurrentSnapshots = runCatching {\n            stateDatabase.readQuarantinedCurrentSnapshots(workspaceKey)",
        "        val quarantinedCurrentCandidates = runCatching {\n            readQuarantinedFutureCandidates(workspaceKey)")?;
    replace(&mut source,
        "        var databaseCandidates = storedDatabaseSnapshots.mapNotNull(::decodeStoredCandidate)",
        "        var databaseCandidates = storedDatabaseCandidates")?;
    replace(&mut source,
        "        val quarantinedFutureCurrent = quarantinedCurrentSnapshots\n            .mapNotNull(::decodeStoredCandidate)",
        "        val quarantinedFutureCurrent = quarantinedCurrentCandidates")?;

    let old_schema = section(
        base,
        "    private fun rawSchemaVersion(",
        "    private fun decodePersistedCandidate(",
    )?;
    replace(&mut source, &old_schema, SCHEMA_HEADER)?;
    replace(&mut source, "    private fun decodeStoredCandidate(stored: StoredStateSnapshot): PersistedCandidate? {", &format!("{CANDIDATE_SURFACES}    private fun decodeStoredCandidate(stored: StoredStateSnapshot): PersistedCandidate? {{"))?;
    if source.contains("stateDatabase.readSnapshots(")
        || source.contains("readQuarantinedCurrentSnapshots(")
    {
        return Err("an eager production recovery snapshot reader remains".into());
    }
    Ok(source)
}

const SCHEMA_HEADER: &str = r####"    @kotlinx.serialization.Serializable
    private data class PersistedSchemaHeader(val schemaVersion: Long? = null)

    private fun rawSchemaVersion(raw: String): Int? {
        // Skip unknown payload fields with the streaming decoder. A full JsonElement
        // tree duplicates every note/history object just to inspect this one number.
        return runCatching {
            json.decodeFromString<PersistedSchemaHeader>(raw).schemaVersion
                ?.coerceIn(Int.MIN_VALUE.toLong(), Int.MAX_VALUE.toLong())
                ?.toInt()
        }.getOrNull()
    }

"####;

const CANDIDATE_SURFACES: &str = r####"    private inner class PersistedCandidateSurfaces {
        private var current: PersistedCandidate? = null
        private var unverifiedCurrent: PersistedCandidate? = null
        private var recovery: PersistedCandidate? = null
        private var future: PersistedCandidate? = null

        fun accept(candidate: PersistedCandidate) {
            when (candidate.source) {
                "sqlite-current" -> if (current == null) current = candidate
                "sqlite-current-unverified" -> if (unverifiedCurrent == null) unverifiedCurrent = candidate
                else -> recovery = chooseSaferPersistedCandidate(recovery, candidate)
            }
            val version = candidate.futureSchemaVersion
            if (version != null && (future == null || version > future!!.futureSchemaVersion!!)) {
                future = candidate
            }
        }

        fun retained(): List<PersistedCandidate> = buildList {
            for (candidate in arrayOf(current, unverifiedCurrent, recovery, future)) {
                if (candidate != null && none { it === candidate }) add(candidate)
            }
        }
    }

    private fun readDatabaseCandidateSurfaces(workspaceKey: String): List<PersistedCandidate> {
        val surfaces = PersistedCandidateSurfaces()
        // The cursor and this accumulator retain at most one incoming payload and
        // four authoritative/recovery candidates, independently of history length.
        stateDatabase.forEachSnapshot(workspaceKey) { stored ->
            decodeStoredCandidate(stored)?.let(surfaces::accept)
        }
        return surfaces.retained()
    }

    private fun readQuarantinedFutureCandidates(workspaceKey: String): List<PersistedCandidate> {
        var future: PersistedCandidate? = null
        stateDatabase.forEachQuarantinedCurrentSnapshot(workspaceKey) { stored ->
            val candidate = decodeStoredCandidate(stored)
            val version = candidate?.futureSchemaVersion
            if (version != null && (future == null || version > future!!.futureSchemaVersion!!)) {
                future = candidate
            }
        }
        return listOfNotNull(future)
    }

"####;

#[cfg(test)]
mod tests {
    use super::*;

    fn original(path: &str) -> &'static str {
        super::super::kotlin_sources::SOURCES
            .iter()
            .find(|source| source.path == path)
            .unwrap()
            .contents
    }

    #[test]
    fn repository_retains_bounded_candidates_without_materializing_history_lists() {
        let source = render(REPOSITORY_PATH, original(REPOSITORY_PATH)).unwrap();
        assert!(!source.contains("stateDatabase.readSnapshots("));
        assert!(!source.contains("storedDatabaseSnapshots.mapNotNull"));
        assert!(source.contains("stateDatabase.forEachSnapshot(workspaceKey)"));
        assert!(source.contains("chooseSaferPersistedCandidate(recovery, candidate)"));
        assert!(source.contains("version > future!!.futureSchemaVersion!!"));
        assert!(source.contains(
            "futureCurrentSurface ?: databaseCurrent ?: jsonPrimaryAuthority ?: recovery"
        ));
    }

    #[test]
    fn database_visitors_keep_digests_order_and_all_existing_history_candidates() {
        let source = render(DATABASE_PATH, original(DATABASE_PATH)).unwrap();
        let visitor = section(&source, "    fun forEachSnapshot(", "    // Compatibility").unwrap();
        assert!(visitor.contains("quarantineDigestMismatches(workspaceKey, includeHistory = true)"));
        assert!(visitor.contains("MAX_HISTORY_READ.toString()"));
        assert!(visitor.contains("created_at_epoch_millis DESC, id DESC"));
        assert_eq!(visitor.matches("?.let(action)").count(), 2);
        assert!(!visitor.contains("mutableListOf"));
        assert!(!source.contains("fun readQuarantinedCurrentSnapshots("));
    }

    #[test]
    fn privacy_redaction_releases_each_payload_and_retains_transaction_guarantees() {
        let source = render(DATABASE_PATH, original(DATABASE_PATH)).unwrap();
        let redaction = section(
            &source,
            "    fun redactEncryptedNoteFromRetainedSnapshots(",
            "    /**\n     * Database v2",
        )
        .unwrap();
        assert!(!redaction.contains("Pair<Long, String>"));
        assert_eq!(redaction.matches("mutableListOf<Long>()").count(), 2);
        assert_eq!(redaction.matches("id = ? AND workspace_key = ?").count(), 2);
        for invariant in [
            "database.beginTransaction()",
            "database.setTransactionSuccessful()",
            "database.endTransaction()",
            "PRAGMA wal_checkpoint(TRUNCATE)",
            "snapshotEnvelopeSha256(",
            "!raw.contains(noteId)",
        ] {
            assert!(
                redaction.contains(invariant),
                "missing privacy invariant {invariant}"
            );
        }
    }

    #[test]
    fn schema_header_does_not_construct_the_payload_tree() {
        let source = render(REPOSITORY_PATH, original(REPOSITORY_PATH)).unwrap();
        let header = section(
            &source,
            "    private fun rawSchemaVersion(",
            "    private fun decodePersistedCandidate(",
        )
        .unwrap();
        assert!(header.contains("decodeFromString<PersistedSchemaHeader>"));
        assert!(!header.contains("parseToJsonElement"));
        assert!(source.contains("strictPersistedJson else json"));
    }
}
