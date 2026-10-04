// v2.22.41 - Preserve newer generations and unrelated data during serialized redaction.

const REPOSITORY: &str = "com/ofairyo/gridtimer/data/TimerRepository.kt";
const DATABASE: &str = "com/ofairyo/gridtimer/data/AppStateDatabase.kt";

fn replace(source: &mut String, old: &str, new: &str) -> Result<(), String> {
    if source.matches(old).count() != 1 {
        return Err(format!("privacy generation anchor changed: {old}"));
    }
    *source = source.replacen(old, new, 1);
    Ok(())
}

pub fn render(path: &str, base: &str) -> Result<String, String> {
    let mut source = base.to_owned();
    match path {
        REPOSITORY => {
            replace(
                &mut source,
                "    private fun redactEncryptedNoteRecoveryCopies(\n",
                r####"    private suspend fun redactEncryptedNoteRecoveryCopies(
        workspaceKey: String,
        protectedNote: NoteEntry,
        draftCheckpoint: NoteDraftJournalCheckpoint?
    ): Boolean = writeMutex.withLock {
        // The current snapshot, history, JSON mirrors and normal saves share
        // one writer. Do not allow a read/replace mirror race after DB commit.
        withContext(NonCancellable + Dispatchers.IO) {
            verifiedWritePreflight = null
            redactEncryptedNoteRecoveryCopiesLocked(workspaceKey, protectedNote, draftCheckpoint)
        }
    }

    private fun redactEncryptedNoteRecoveryCopiesLocked(
"####,
            )?;
            replace(&mut source,
                "        val candidate = decodePersistedCandidate(\"encrypted-note-privacy-redaction\", raw)",
                "        requirePrivacySnapshotSchema(raw)\n        val candidate = decodePersistedCandidate(\"encrypted-note-privacy-redaction\", raw)")?;
            replace(&mut source,
                "        if (existingIndex < 0 || candidate.data.notes[existingIndex].encryption != null) {",
                r####"        val existing = candidate.data.notes.getOrNull(existingIndex)
        val newerPlaintext = existing?.takeIf { it.encryption == null }?.let {
            val revision = it.effectiveProtectionStateRevision()
            val protectedRevision = protectedNote.effectiveProtectionStateRevision()
            check(revision != protectedRevision) {
                "Conflicting plaintext and encrypted notes share one protection generation."
            }
            revision > protectedRevision
        } == true
        // A later disable-encryption operation is intentional. Its plaintext
        // must survive an old queued cleanup, regardless of wall-clock order.
        if (existing == null || existing.encryption != null || newerPlaintext) {"####)?;
            replace(&mut source,
                "        ).sanitized()\n        val encoded = strictPersistedJson.encodeToString(redactedData)",
                "        )\n        val encoded = strictPersistedJson.encodeToString(redactedData)")?;
            source.push_str(SCHEMA_GUARD);
        }
        DATABASE => {
            let from = source
                .find("    fun redactEncryptedNoteFromRetainedSnapshots(")
                .ok_or("privacy database method missing")?;
            let to = source[from..]
                .find("    @Synchronized\n")
                .map(|offset| from + offset)
                .ok_or("privacy database method end missing")?;
            let original = source[from..to].to_owned();
            let mut method = original.clone();
            replace(&mut method,
                "                    val raw = cursor.getString(cursor.getColumnIndexOrThrow(\"app_data_json\"))\n                    val rewritten = rewrite(raw)",
                "                    val raw = cursor.getString(cursor.getColumnIndexOrThrow(\"app_data_json\"))\n                    requirePrivacySnapshotSchema(raw, cursor.getInt(cursor.getColumnIndexOrThrow(\"schema_version\")))\n                    val rewritten = rewrite(raw)")?;
            for table in ["HISTORY_TABLE", "QUARANTINE_TABLE"] {
                replace(&mut method,
                    &format!("                    {table}, arrayOf(\"app_data_json\"),"),
                    &format!("                    {table}, arrayOf(\"app_data_json\", \"schema_version\"),"))?;
            }
            let row = "                    check(cursor.moveToFirst()) { \"Snapshot disappeared during privacy redaction.\" }\n                    cursor.getString(0)";
            if method.matches(row).count() != 2 {
                return Err("privacy retained row guards changed".to_owned());
            }
            method = method.replace(row,
                "                    check(cursor.moveToFirst()) { \"Snapshot disappeared during privacy redaction.\" }\n                    cursor.getString(0).also { raw -> requirePrivacySnapshotSchema(raw, cursor.getInt(1)) }");
            replace(&mut source, &original, &method)?;
        }
        _ => {}
    }
    Ok(source)
}

pub const SCHEMA_GUARD: &str = r####"

internal fun requirePrivacySnapshotSchema(raw: String, declaredVersion: Int? = null) {
    check(snapshotFutureSchema(raw, declaredVersion) == null) {
        "Privacy cleanup cannot rewrite a snapshot created by a newer schema."
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
        let base = super::super::android_startup_loading::render(path, &base).unwrap();
        render(path, &base).unwrap()
    }

    #[test]
    fn both_privacy_callers_share_the_normal_writer_mutex() {
        let source = rendered(REPOSITORY);
        let wrapper = source
            .split("    private suspend fun redactEncryptedNoteRecoveryCopies(")
            .nth(1)
            .unwrap()
            .split("    private fun redactEncryptedNoteRecoveryCopiesLocked(")
            .next()
            .unwrap();
        assert!(wrapper.contains(": Boolean = writeMutex.withLock {"));
        assert!(wrapper.contains("withContext(NonCancellable + Dispatchers.IO)"));
        assert!(wrapper.contains("verifiedWritePreflight = null"));
        assert_eq!(
            source
                .matches("redactEncryptedNoteRecoveryCopiesLocked(")
                .count(),
            2
        );
        assert!(source.contains("privacyRedactionCompleted = redactEncryptedNoteRecoveryCopies("));
        assert!(source.contains("if (redactEncryptedNoteRecoveryCopies("));
    }

    #[test]
    fn all_database_surfaces_check_declared_schema_inside_the_original_transaction() {
        let source = rendered(DATABASE);
        let method = source
            .split("    fun redactEncryptedNoteFromRetainedSnapshots(")
            .nth(1)
            .unwrap()
            .split("    @Synchronized\n")
            .next()
            .unwrap();
        assert_eq!(
            method.matches("requirePrivacySnapshotSchema(raw,").count(),
            3
        );
        assert!(
            method.find("database.beginTransaction()").unwrap()
                < method.find("requirePrivacySnapshotSchema").unwrap()
        );
        assert!(method.contains("database.setTransactionSuccessful()"));
        assert!(method.contains("database.endTransaction()"));
        assert!(!method.contains("mutableListOf<Pair<Long, String>>()"));
    }

    #[test]
    fn every_json_rewrite_checks_schema_before_decoding_or_removing_evidence() {
        let source = rendered(REPOSITORY);
        assert!(source.contains(
            "requirePrivacySnapshotSchema(raw)\n        val candidate = decodePersistedCandidate"
        ));
        assert!(source.contains("revision > protectedRevision"));
        assert!(source.contains("check(revision != protectedRevision)"));
        assert!(
            source.contains("existing == null || existing.encryption != null || newerPlaintext")
        );
    }
}
