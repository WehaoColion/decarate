// v0.0.5 - Keep legacy fixtures at their declared format after the media-reader upgrade.
// v0.0.4 - Reject privacy changes through the reference-only metadata writer.
// v0.0.3 - Changed policies and bodies must invalidate unchanged projection reuse.
// v0.0.2 - Reject malformed note payloads and unknown versions in legacy projections.
#[test]
fn server_privacy_projection_reuse_cannot_bypass_changed_policy_or_body() {
    let raw = server_privacy_audit_plain();
    let other = raw.replace("private-note", "cache-other-note");
    let policy_for = |body: &str, id: &str| {
        let deleted =
            crate::app_data::delete_note_permanently_app_data_json(body, id, 200).unwrap();
        crate::desktop_state_store::DesktopPrivacyPolicy::default()
            .including_snapshot(&deleted)
            .unwrap()
    };
    let original_policy = policy_for(&other, "cache-other-note");
    for _ in 0..2 {
        assert!(note_privacy::project(&original_policy, &raw)
            .unwrap()
            .is_none());
    }
    let changed_policy = policy_for(&raw, "private-note");
    for (policy, body) in [(&changed_policy, &raw), (&original_policy, &other)] {
        let cleaned = note_privacy::project(policy, body).unwrap().unwrap();
        assert!(!cleaned.contains("SERVER_PRIVATE_MARKER_49371"));
        let value: serde_json::Value = serde_json::from_str(&cleaned).unwrap();
        assert!(value["notes"].as_array().unwrap().is_empty());
    }
    let mut malformed: serde_json::Value = serde_json::from_str(&raw).unwrap();
    malformed["notes"][0]["encryption"] = serde_json::json!({"cipherSuite":"unknown"});
    assert!(note_privacy::project(&original_policy, &malformed.to_string()).is_err());
}

#[test]
fn server_privacy_legacy_non_note_fallback_keeps_note_validation() {
    let raw = server_privacy_audit_plain();
    let deleted =
        crate::app_data::delete_note_permanently_app_data_json(&raw, "private-note", 200).unwrap();
    let policy = crate::desktop_state_store::DesktopPrivacyPolicy::default()
        .including_snapshot(&deleted)
        .unwrap();
    let mut legacy: serde_json::Value = serde_json::from_str(&raw).unwrap();
    legacy["schemaVersion"] = serde_json::json!(10);
    legacy["slots"] = serde_json::json!({"legacyOnly":true});
    let cleaned = note_privacy::project(&policy, &legacy.to_string())
        .unwrap()
        .unwrap();
    assert!(!cleaned.contains("SERVER_PRIVATE_MARKER_49371"));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&cleaned).unwrap()["slots"],
        legacy["slots"]
    );
    let mut malformed = legacy.clone();
    malformed["notes"][0]["encryption"] = serde_json::json!({"cipherSuite":"unknown"});
    assert!(note_privacy::project(&policy, &malformed.to_string()).is_err());
    for schema in [
        serde_json::json!(APP_DATA_SCHEMA_VERSION),
        serde_json::json!(APP_DATA_SCHEMA_VERSION + 1),
        serde_json::json!(-1),
        serde_json::json!("10"),
    ] {
        let mut rejected = legacy.clone();
        rejected["schemaVersion"] = schema;
        assert!(note_privacy::project(&policy, &rejected.to_string()).is_err());
    }
}

fn server_privacy_audit_plain() -> String {
    crate::app_data::upsert_note_app_data_json(&crate::app_data::default_app_data_json(100),
        r#"{"id":"private-note","title":"Private","content":"SERVER_PRIVATE_MARKER_49371","updatedAtEpochMillis":100}"#, 100).unwrap()
}

fn server_privacy_audit_sealed(raw: &str) -> String {
    let value: serde_json::Value = serde_json::from_str(raw).unwrap();
    let (sealed, session) =
        crate::encrypt_desktop_note_json(&value["notes"][0].to_string(), "audit-only-password")
            .unwrap();
    crate::close_desktop_note_session(&session);
    crate::app_data::upsert_note_app_data_json(raw, &sealed, 200).unwrap()
}

#[test]
fn server_privacy_audit_encryption_removes_plaintext_history() {
    let directory = TestDirectory::new("privacy-encryption-history");
    let store = open_empty(&directory);
    store
        .create_user(new_user("user-a", "a@example.test"))
        .unwrap();
    let raw = server_privacy_audit_plain();
    store
        .compare_and_swap_account("user-a", 0, &raw, 100)
        .unwrap();
    let sealed = server_privacy_audit_sealed(&raw);
    store
        .compare_and_swap_account("user-a", 1, &sealed, 200)
        .unwrap();
    for snapshot in store.list_snapshot_history("user-a", 100).unwrap() {
        assert!(
            !snapshot
                .app_data_json
                .contains("SERVER_PRIVATE_MARKER_49371"),
            "server history retained plaintext at revision {}",
            snapshot.revision
        );
    }
}

#[test]
fn server_privacy_audit_permanent_deletion_cannot_be_restored() {
    let directory = TestDirectory::new("privacy-deletion-history");
    let store = open_empty(&directory);
    store
        .create_user(new_user("user-a", "a@example.test"))
        .unwrap();
    let raw = server_privacy_audit_plain();
    store
        .compare_and_swap_account("user-a", 0, &raw, 100)
        .unwrap();
    let deleted =
        crate::app_data::delete_note_permanently_app_data_json(&raw, "private-note", 200).unwrap();
    store
        .compare_and_swap_account("user-a", 1, &deleted, 200)
        .unwrap();
    let restored = store.restore_account_snapshot("user-a", 1, 2, 300).unwrap();
    assert!(
        !restored
            .app_data_json
            .contains("SERVER_PRIVATE_MARKER_49371"),
        "account restore revived permanently deleted content"
    );
}

#[test]
fn server_privacy_audit_new_backup_does_not_contain_plaintext_history() {
    let directory = TestDirectory::new("privacy-backup-history");
    let store = open_empty(&directory);
    store
        .create_user(new_user("user-a", "a@example.test"))
        .unwrap();
    let raw = server_privacy_audit_plain();
    store
        .compare_and_swap_account("user-a", 0, &raw, 100)
        .unwrap();
    let sealed = server_privacy_audit_sealed(&raw);
    store
        .compare_and_swap_account("user-a", 1, &sealed, 200)
        .unwrap();
    let backup = directory.0.join("privacy-backup.sqlite3");
    store.create_verified_backup(&backup, 300).unwrap();
    let restored_store = SqliteServerStore::open(&backup, None).unwrap();
    for snapshot in restored_store.list_snapshot_history("user-a", 100).unwrap() {
        assert!(
            !snapshot
                .app_data_json
                .contains("SERVER_PRIVATE_MARKER_49371"),
            "verified server backup retained decrypted history content"
        );
    }
}

#[test]
fn server_privacy_audit_receipts_remain_spent_without_replaying_plaintext() {
    let directory = TestDirectory::new("privacy-dedup");
    let store = open_empty(&directory);
    store
        .create_user(new_user("user-a", "a@example.test"))
        .unwrap();
    let raw = server_privacy_audit_plain();
    let response = serde_json::json!({"ok":true, "appDataJson":raw}).to_string();
    store
        .apply_sync_request(
            "user-a",
            "privacy-request",
            r#"{"operation":1}"#,
            0,
            &raw,
            &response,
            100,
        )
        .unwrap();
    let sealed = server_privacy_audit_sealed(&raw);
    store
        .compare_and_swap_account("user-a", 1, &sealed, 200)
        .unwrap();
    let replay = store
        .apply_sync_request(
            "user-a",
            "privacy-request",
            r#"{"operation":1}"#,
            0,
            &raw,
            &response,
            300,
        )
        .unwrap();
    match replay {
        SyncRequestOutcome::Replayed(receipt) => {
            assert!(!receipt
                .response_json
                .contains("SERVER_PRIVATE_MARKER_49371"));
            assert_eq!(receipt.account_revision, 1);
            let response: serde_json::Value = serde_json::from_str(&receipt.response_json).unwrap();
            let payload: serde_json::Value =
                serde_json::from_str(response["appDataJson"].as_str().unwrap()).unwrap();
            assert!(payload["notes"][0]["encryption"].is_object());
        }
        _ => panic!("privacy cleanup must not make a receipt reusable"),
    }
    assert_eq!(store.read_account("user-a").unwrap().revision, 2);
    assert!(store
        .apply_sync_request(
            "user-a",
            "privacy-request",
            r#"{"operation":2}"#,
            2,
            &sealed,
            &response,
            400
        )
        .is_err());
    store.validate_integrity().unwrap();
}

#[test]
fn server_privacy_audit_failure_rolls_back_account_history_and_receipts() {
    let directory = TestDirectory::new("privacy-rollback");
    let store = open_empty(&directory);
    store
        .create_user(new_user("user-a", "a@example.test"))
        .unwrap();
    let raw = server_privacy_audit_plain();
    let response = serde_json::json!({"appDataJson":raw}).to_string();
    store
        .apply_sync_request("user-a", "privacy-request", "{}", 0, &raw, &response, 100)
        .unwrap();
    let before = store.read_account("user-a").unwrap();
    let history = store.list_snapshot_history("user-a", 100).unwrap();
    let connection = store.open_connection(false).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER privacy_test_fail BEFORE INSERT ON account_note_privacy
        BEGIN SELECT RAISE(ABORT, 'injected privacy failure'); END;",
        )
        .unwrap();
    let sealed = server_privacy_audit_sealed(&raw);
    assert!(store
        .compare_and_swap_account("user-a", 1, &sealed, 200)
        .is_err());
    assert_eq!(before, store.read_account("user-a").unwrap());
    assert_eq!(history, store.list_snapshot_history("user-a", 100).unwrap());
    let stored: String = connection
        .query_row(
            "SELECT response_json FROM request_dedup WHERE request_id='privacy-request'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored, response);
    store.validate_integrity().unwrap();
}

#[test]
fn server_privacy_audit_restart_rejects_stale_writes_and_legacy_imports() {
    let directory = TestDirectory::new("privacy-restart");
    let store = open_empty(&directory);
    store
        .create_user(new_user("user-a", "a@example.test"))
        .unwrap();
    let raw = server_privacy_audit_plain();
    store
        .compare_and_swap_account("user-a", 0, &raw, 100)
        .unwrap();
    let deleted =
        crate::app_data::delete_note_permanently_app_data_json(&raw, "private-note", 200).unwrap();
    store
        .compare_and_swap_account("user-a", 1, &deleted, 200)
        .unwrap();
    let path = store.database_path.clone();
    drop(store);
    let store = SqliteServerStore::open(&path, None).unwrap();
    assert!(store
        .compare_and_swap_account("user-a", 2, &raw, 300)
        .is_err());
    let mut connection = store.open_connection(false).unwrap();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert!(import_account_snapshot_in_transaction(
        &transaction,
        "user-a",
        &raw,
        100,
        400,
        &mut |_, _, incoming, _, _| Ok(incoming.to_owned())
    )
    .is_err());
    drop(transaction);
    assert_eq!(store.read_account("user-a").unwrap().app_data_json, deleted);
    store.finish_note_privacy_cleanup().unwrap();
    drop(connection);
    drop(store);
    assert!(!fs::read(&path)
        .unwrap()
        .windows(b"SERVER_PRIVATE_MARKER_49371".len())
        .any(|bytes| bytes == b"SERVER_PRIVATE_MARKER_49371"));
}

#[test]
fn server_privacy_audit_shared_content_keeps_other_account_and_unrelated_notes() {
    let directory = TestDirectory::new("privacy-account-isolation");
    let store = open_empty(&directory);
    let raw = crate::app_data::upsert_note_app_data_json(&server_privacy_audit_plain(),
        r#"{"id":"ordinary","title":"Unrelated","content":"KEEP_OTHER_NOTE","updatedAtEpochMillis":100}"#, 100).unwrap();
    for (id, email) in [("user-a", "a@example.test"), ("user-b", "b@example.test")] {
        let mut user = new_user(id, email);
        user.app_data_json = raw.clone();
        store.create_user(user).unwrap();
    }
    let other_before = store.list_snapshot_history("user-b", 100).unwrap();
    let deleted =
        crate::app_data::delete_note_permanently_app_data_json(&raw, "private-note", 200).unwrap();
    store
        .compare_and_swap_account("user-a", 0, &deleted, 200)
        .unwrap();
    assert_eq!(
        other_before,
        store.list_snapshot_history("user-b", 100).unwrap()
    );
    for history in store.list_snapshot_history("user-a", 100).unwrap() {
        assert!(history.app_data_json.contains("KEEP_OTHER_NOTE"));
        assert!(!history
            .app_data_json
            .contains("SERVER_PRIVATE_MARKER_49371"));
    }
    assert_eq!(store.read_account("user-b").unwrap().app_data_json, raw);
    store.validate_integrity().unwrap();
}

#[test]
fn server_privacy_audit_upgrade_seeds_existing_seal_without_losing_identity() {
    let directory = TestDirectory::new("privacy-schema14");
    let store = open_empty(&directory);
    store
        .create_user(new_user("user-a", "a@example.test"))
        .unwrap();
    let raw = server_privacy_audit_plain();
    store
        .compare_and_swap_account("user-a", 0, &raw, 100)
        .unwrap();
    store
        .compare_and_swap_account("user-a", 1, &raw, 200)
        .unwrap();
    let connection = store.open_connection(false).unwrap();
    let identity: String = connection
        .query_row("SELECT server_instance_id FROM server_identity", [], |r| {
            r.get(0)
        })
        .unwrap();
    let sealed = server_privacy_audit_sealed(&raw);
    connection.execute("UPDATE account_snapshots SET app_data_json=?1, content_sha256=?2, envelope_sha256=?3 WHERE user_id='user-a'",
        params![sealed, sha256_hex(sealed.as_bytes()), account_snapshot_envelope_sha256("user-a", &sealed, 2, 200, 0)]).unwrap();
    connection.execute_batch("DROP TABLE account_note_privacy; DROP TABLE IF EXISTS note_privacy_commit_witnesses; DELETE FROM schema_migrations WHERE version>=15; PRAGMA user_version=14;").unwrap();
    let path = store.database_path.clone();
    drop(connection);
    drop(store);
    let opened = SqliteServerStore::open_with_options(ServerStoreOpenOptions {
        database_path: path,
        legacy_json_path: None,
        now_epoch_millis: 300,
        legacy_token_ttl_millis: DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
    })
    .unwrap();
    let backup = opened.pre_schema_migration_backup.unwrap();
    assert!(backup.destination.exists());
    let verified = SqliteServerStore::verify_existing_backup(&backup.destination, 300).unwrap();
    assert_eq!(verified.sha256, backup.sha256);
    assert_eq!(verified.server_instance_id, identity);
    let connection = opened.store.open_connection(false).unwrap();
    assert_eq!(
        identity,
        connection
            .query_row("SELECT server_instance_id FROM server_identity", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap()
    );
    assert_eq!(
        opened.store.read_account("user-a").unwrap().app_data_json,
        sealed
    );
    for history in opened.store.list_snapshot_history("user-a", 100).unwrap() {
        assert!(!history
            .app_data_json
            .contains("SERVER_PRIVATE_MARKER_49371"));
    }
    opened.store.validate_integrity().unwrap();
}

#[test]
fn server_privacy_audit_tampered_policy_and_future_backup_fail_closed() {
    let directory = TestDirectory::new("privacy-tampering");
    let store = open_empty(&directory);
    store
        .create_user(new_user("user-a", "a@example.test"))
        .unwrap();
    let raw = server_privacy_audit_plain();
    store
        .compare_and_swap_account("user-a", 0, &raw, 100)
        .unwrap();
    let sealed = server_privacy_audit_sealed(&raw);
    store
        .compare_and_swap_account("user-a", 1, &sealed, 200)
        .unwrap();
    let backup = directory.0.join("future.sqlite3");
    store.create_verified_backup(&backup, 300).unwrap();
    let backup_connection = Connection::open(&backup).unwrap();
    backup_connection
        .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
        .unwrap();
    drop(backup_connection);
    let backup_hash = sha256_file(&backup).unwrap();
    assert!(SqliteServerStore::verify_existing_backup(&backup, 300).is_err());
    assert_eq!(sha256_file(&backup).unwrap(), backup_hash);
    let backup_connection = Connection::open(&backup).unwrap();
    backup_connection.execute("INSERT INTO schema_migrations(version, description, applied_at_epoch_millis) VALUES(?1,'future layout',400)",
        params![SCHEMA_VERSION + 1]).unwrap();
    drop(backup_connection);
    assert!(SqliteServerStore::verify_existing_backup(&backup, 300).is_err());
    let connection = store.open_connection(false).unwrap();
    connection
        .execute(
            "UPDATE account_note_privacy SET policy_json='{}' WHERE user_id='user-a'",
            [],
        )
        .unwrap();
    assert!(store.validate_integrity().is_err());
    assert!(store
        .compare_and_swap_account("user-a", 2, &raw, 400)
        .is_err());
    assert_eq!(store.read_account("user-a").unwrap().app_data_json, sealed);
}

#[test]
fn server_privacy_audit_history_redaction_preserves_unrelated_media_recovery() {
    let directory = TestDirectory::new("privacy-media-recovery");
    let store = open_empty(&directory);
    store
        .create_user(new_user("user-a", "a@example.test"))
        .unwrap();
    let mut raw = crate::app_data::default_app_data_json(100);
    for (note_id, media_id, content) in [
        ("private-note", "private-media", b"private image".as_slice()),
        ("ordinary", "other-media", b"keep image".as_slice()),
    ] {
        let digest = sha256_hex(content);
        store
            .upsert_media(
                "user-a",
                media_id,
                &digest,
                "image/png",
                content.len() as i64,
                content,
                100,
            )
            .unwrap();
        let note = serde_json::json!({"id":note_id,"title":note_id,"content":note_id,"updatedAtEpochMillis":100,
            "attachments":[{"id":media_id,"sha256":digest,"mimeType":"image/png","sizeBytes":content.len(),"updatedAtEpochMillis":100}]});
        raw = crate::app_data::upsert_note_app_data_json(&raw, &note.to_string(), 100).unwrap();
    }
    store
        .compare_and_swap_account("user-a", 0, &raw, 100)
        .unwrap();
    let deleted =
        crate::app_data::delete_note_permanently_app_data_json(&raw, "private-note", 200).unwrap();
    store
        .compare_and_swap_account("user-a", 1, &deleted, 200)
        .unwrap();
    let connection = store.open_connection(false).unwrap();
    let ids = connection.prepare("SELECT attachment_id FROM account_snapshot_media_history WHERE user_id='user-a' AND account_revision=1")
        .unwrap().query_map([], |r| r.get::<_, String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    assert_eq!(ids, vec!["other-media"]);
    let restored = store.restore_account_snapshot("user-a", 1, 2, 300).unwrap();
    assert!(!restored.app_data_json.contains("private-media"));
    let media = store.read_media("user-a", "other-media").unwrap().unwrap();
    assert_eq!(media.content, b"keep image");
    store.validate_integrity().unwrap();
}

#[test]
fn server_privacy_audit_busy_checkpoint_keeps_cleanup_pending_for_retry() {
    let directory = TestDirectory::new("privacy-checkpoint-retry");
    let store = open_empty(&directory);
    store
        .create_user(new_user("user-a", "a@example.test"))
        .unwrap();
    let raw = server_privacy_audit_plain();
    store
        .compare_and_swap_account("user-a", 0, &raw, 100)
        .unwrap();
    let reader = store.open_connection(false).unwrap();
    reader.execute_batch("BEGIN;").unwrap();
    assert_eq!(
        reader
            .query_row(
                "SELECT app_data_json FROM account_snapshots WHERE user_id='user-a'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        raw
    );
    let sealed = server_privacy_audit_sealed(&raw);
    store
        .compare_and_swap_account("user-a", 1, &sealed, 200)
        .unwrap();
    assert!(store.finish_note_privacy_cleanup().is_err());
    let connection = store.open_connection(false).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT cleanup_pending FROM account_note_privacy WHERE user_id='user-a'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    reader.execute_batch("ROLLBACK;").unwrap();
    drop(reader);
    store.finish_note_privacy_cleanup().unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT cleanup_pending FROM account_note_privacy WHERE user_id='user-a'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    store.validate_integrity().unwrap();
}

// v0.0.1 - Reproduce permanent-deletion media retention while proving shared references survive.
#[test]
fn server_privacy_media_permanent_delete_purges_only_unreferenced_account_blobs() {
    let directory = TestDirectory::new("permanent-delete-media-audit");
    let store = open_empty(&directory);
    for (user, email) in [("user-a", "a@example.test"), ("user-b", "b@example.test")] {
        store.create_user(new_user(user, email)).unwrap();
    }
    let media: [(&str, &[u8]); 4] = [
        ("private-only", b"PERMANENT_PRIVATE_MEDIA_83664"),
        ("shared-current", b"KEEP_CURRENT_MEDIA_56301"),
        ("shared-version", b"KEEP_VERSION_MEDIA_11209"),
        ("shared-account", b"KEEP_OTHER_ACCOUNT_MEDIA_33781"),
    ];
    let attachment = |index: usize| {
        let (id, content) = media[index];
        serde_json::json!({"id":id,"sha256":sha256_hex(content),"mimeType":"application/octet-stream",
            "sizeBytes":content.len(),"createdAtEpochMillis":100,"updatedAtEpochMillis":100})
    };
    let mut document: serde_json::Value =
        serde_json::from_str(&crate::app_data::default_app_data_json(100)).unwrap();
    document["notes"] = serde_json::json!([
        {"id":"private-note","content":"private synthetic page","updatedAtEpochMillis":100,
            "attachments":[attachment(0),attachment(1),attachment(2),attachment(3)]},
        {"id":"ordinary-note","content":"ordinary synthetic page","updatedAtEpochMillis":100,
            "attachments":[attachment(1)],
            "versions":[{"id":"ordinary-version","noteId":"ordinary-note","sequence":1,
                "content":"ordinary previous version","attachments":[attachment(2)],
                "attachmentIds":["shared-version"],"createdAtEpochMillis":90,"updatedAtEpochMillis":100},
                {"id":"ordinary-current-version","noteId":"ordinary-note","sequence":2,
                "content":"ordinary synthetic page","attachments":[attachment(1)],
                "createdAtEpochMillis":100,"updatedAtEpochMillis":100}]}
    ]);
    let plain = crate::app_data::sanitize_app_data_json(&document.to_string(), 100).unwrap();
    let mut other: serde_json::Value =
        serde_json::from_str(&crate::app_data::default_app_data_json(100)).unwrap();
    other["notes"] = serde_json::json!([{"id":"other-note","content":"other account","updatedAtEpochMillis":100,"attachments":[attachment(3)]}]);
    let other = crate::app_data::sanitize_app_data_json(&other.to_string(), 100).unwrap();
    store
        .compare_and_swap_account("user-a", 0, &plain, 100)
        .unwrap();
    store
        .compare_and_swap_account("user-b", 0, &other, 100)
        .unwrap();
    for (id, content) in media {
        store
            .upsert_media(
                "user-a",
                id,
                &sha256_hex(content),
                "application/octet-stream",
                content.len() as i64,
                content,
                100,
            )
            .unwrap();
    }
    store
        .upsert_media(
            "user-b",
            media[3].0,
            &sha256_hex(media[3].1),
            "application/octet-stream",
            media[3].1.len() as i64,
            media[3].1,
            100,
        )
        .unwrap();
    store
        .compare_and_swap_account("user-b", 1, &other, 101)
        .unwrap();
    let other_history = store.list_snapshot_history("user-b", 100).unwrap();
    let deleted =
        crate::app_data::delete_note_permanently_app_data_json(&plain, "private-note", 200)
            .unwrap();
    let retained = referenced_attachment_ids(&deleted).unwrap();
    assert!(retained.contains("shared-current") && retained.contains("shared-version"));
    assert!(!retained.contains("private-only") && !retained.contains("shared-account"));
    store
        .compare_and_swap_account("user-a", 1, &deleted, 200)
        .unwrap();
    // Complete both client requests; this is not an interrupted media sync.
    for id in ["private-only", "shared-account"] {
        store.delete_media("user-a", id, 200, 200).unwrap();
        assert!(store.read_media("user-a", id).unwrap().is_none());
    }
    store.finish_note_privacy_cleanup().unwrap();
    for (user, index) in [("user-a", 1), ("user-a", 2), ("user-b", 3)] {
        assert_eq!(
            media[index].1,
            store
                .read_media(user, media[index].0)
                .unwrap()
                .unwrap()
                .content
                .as_slice()
        );
    }
    assert_eq!(
        other_history,
        store.list_snapshot_history("user-b", 100).unwrap()
    );
    store.validate_integrity().unwrap();
    let connection = store.open_connection(false).unwrap();
    let historical_private: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM media_snapshot_contents WHERE sha256=?1",
            [sha256_hex(media[0].1)],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(0, historical_private, "private history bytes remain");
    let other_shared: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM media_snapshot_contents WHERE sha256=?1",
            [sha256_hex(media[3].1)],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(1, other_shared, "other account lost shared content");
    let obsolete: i64 = connection.query_row("SELECT COUNT(*) FROM note_media WHERE user_id='user-a' AND attachment_id IN ('private-only','shared-account')",[],|r|r.get(0)).unwrap();
    assert_eq!(
        0, obsolete,
        "permanently deleted unreferenced attachment bytes remain in the live store"
    );
}

// v0.0.1 - Exercise permanent deletion, retention, restart repair and rollback boundaries.
fn media_privacy_document(ids: &[&str]) -> String {
    let mut value: serde_json::Value =
        serde_json::from_str(&crate::app_data::default_app_data_json(100)).unwrap();
    let attachments = ids.iter().map(|id| serde_json::json!({"id":id,"sha256":sha256_hex(id.as_bytes()),
        "mimeType":"application/octet-stream","sizeBytes":id.len(),"createdAtEpochMillis":100,"updatedAtEpochMillis":100})).collect::<Vec<_>>();
    value["notes"] = serde_json::json!([{"id":"private-note","content":"private media note","attachments":attachments,"updatedAtEpochMillis":100}]);
    crate::app_data::sanitize_app_data_json(&value.to_string(), 100).unwrap()
}

fn media_privacy_upload(store: &SqliteServerStore, id: &str, at: i64) -> MediaUpsertOutcome {
    store
        .upsert_media(
            "user-a",
            id,
            &sha256_hex(id.as_bytes()),
            "application/octet-stream",
            id.len() as i64,
            id.as_bytes(),
            at,
        )
        .unwrap()
}

fn media_privacy_live_count(store: &SqliteServerStore, id: &str) -> i64 {
    store
        .open_connection(false)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM note_media WHERE user_id='user-a' AND attachment_id=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn server_privacy_media_soft_delete_undo_pending_and_explicit_restore_survive() {
    let directory = TestDirectory::new("media-privacy-undo");
    let store = open_empty(&directory);
    store
        .create_user(new_user("user-a", "a@example.test"))
        .unwrap();
    let plain = media_privacy_document(&["owned-media"]);
    store
        .compare_and_swap_account("user-a", 0, &plain, 100)
        .unwrap();
    media_privacy_upload(&store, "owned-media", 100);
    media_privacy_upload(&store, "unassociated-upload", 150);
    let trashed = crate::app_data::delete_note_app_data_json(&plain, "private-note", 200).unwrap();
    store
        .compare_and_swap_account("user-a", 1, &trashed, 200)
        .unwrap();
    store.finish_note_privacy_cleanup().unwrap();
    assert_eq!(
        b"owned-media",
        store
            .read_media("user-a", "owned-media")
            .unwrap()
            .unwrap()
            .content
            .as_slice()
    );
    let restored =
        crate::app_data::restore_note_app_data_json(&trashed, "private-note", 250).unwrap();
    store
        .compare_and_swap_account("user-a", 2, &restored, 250)
        .unwrap();
    let deleted =
        crate::app_data::delete_note_permanently_app_data_json(&restored, "private-note", 300)
            .unwrap();
    store
        .compare_and_swap_account("user-a", 3, &deleted, 300)
        .unwrap();
    assert_eq!(
        0,
        media_privacy_live_count(&store, "owned-media"),
        "permanent deletion retained a live BLOB before the follow-up media request"
    );
    assert!(
        matches!(
            media_privacy_upload(&store, "owned-media", 10_000),
            MediaUpsertOutcome::RejectedByTombstone(_)
        ),
        "fast-clock stale upload revived permanently deleted bytes"
    );
    assert_eq!(
        1,
        media_privacy_live_count(&store, "unassociated-upload"),
        "unknown pending upload was purged"
    );
    assert!(matches!(
        store
            .upsert_media_with_restore(
                "user-a",
                "owned-media",
                &sha256_hex(b"owned-media"),
                "application/octet-stream",
                11,
                b"owned-media",
                301,
                true
            )
            .unwrap(),
        MediaUpsertOutcome::Stored(_)
    ));
    store.finish_note_privacy_cleanup().unwrap();
    assert_eq!(
        1,
        media_privacy_live_count(&store, "owned-media"),
        "newer explicit restoration was purged by an older deletion"
    );
    store.validate_integrity().unwrap();
}

#[test]
fn server_privacy_media_current_conflicts_and_retained_history_prevent_purge() {
    let directory = TestDirectory::new("media-privacy-retained-references");
    let store = open_empty(&directory);
    store
        .create_user(new_user("user-a", "a@example.test"))
        .unwrap();
    let plain = media_privacy_document(&["history-shared", "conflict-shared", "private-only"]);
    let mut value: serde_json::Value = serde_json::from_str(&plain).unwrap();
    let old_note = serde_json::json!({"id":"ordinary-note","content":"old reference","attachments":[value["notes"][0]["attachments"][0].clone()],"updatedAtEpochMillis":100});
    value["notes"].as_array_mut().unwrap().push(old_note);
    let old = crate::app_data::sanitize_app_data_json(&value.to_string(), 100).unwrap();
    store
        .compare_and_swap_account("user-a", 0, &old, 100)
        .unwrap();
    for id in ["history-shared", "conflict-shared", "private-only"] {
        media_privacy_upload(&store, id, 100);
    }
    value["notes"][1]["attachments"] = serde_json::json!([]);
    value["notes"][1]["updatedAtEpochMillis"] = serde_json::json!(150);
    value["syncConflictHistory"] = serde_json::json!([{"id":"keep-conflict","entityType":"note","entityId":"conflict-note","losingRevisionEpochMillis":100,"capturedAtEpochMillis":150,
        "payload":{"id":"conflict-note","content":"retained conflict","updatedAtEpochMillis":100,
        "attachments":[value["notes"][0]["attachments"][1].clone()],"document":{"blocks":[{"attachmentId":"conflict-shared"}]}}}]);
    let next = crate::app_data::sanitize_app_data_json(&value.to_string(), 150).unwrap();
    assert!(
        next.contains("keep-conflict"),
        "fixture lost its conflict record"
    );
    store
        .compare_and_swap_account("user-a", 1, &next, 150)
        .unwrap();
    let deleted =
        crate::app_data::delete_note_permanently_app_data_json(&next, "private-note", 200).unwrap();
    store
        .compare_and_swap_account("user-a", 2, &deleted, 200)
        .unwrap();
    store.finish_note_privacy_cleanup().unwrap();
    for id in ["history-shared", "conflict-shared"] {
        assert_eq!(
            1,
            media_privacy_live_count(&store, id),
            "retained account reference lost {id}"
        );
    }
    assert_eq!(0, media_privacy_live_count(&store, "private-only"));
    store.validate_integrity().unwrap();
}

#[test]
fn server_privacy_media_restart_repairs_already_redacted_rows_and_erases_sqlite_bytes() {
    let directory = TestDirectory::new("media-privacy-old-cleanup");
    let store = open_empty(&directory);
    store
        .create_user(new_user("user-a", "a@example.test"))
        .unwrap();
    let id = "restart-media";
    let marker = "PRIVATE_MEDIA_RESTART_MARKER_507637204";
    let mut value: serde_json::Value =
        serde_json::from_str(&media_privacy_document(&[id])).unwrap();
    value["notes"][0]["attachments"][0]["sha256"] =
        serde_json::json!(sha256_hex(marker.as_bytes()));
    value["notes"][0]["attachments"][0]["sizeBytes"] = serde_json::json!(marker.len());
    let plain = crate::app_data::sanitize_app_data_json(&value.to_string(), 100).unwrap();
    store
        .compare_and_swap_account("user-a", 0, &plain, 100)
        .unwrap();
    store
        .upsert_media(
            "user-a",
            id,
            &sha256_hex(marker.as_bytes()),
            "application/octet-stream",
            marker.len() as i64,
            marker.as_bytes(),
            100,
        )
        .unwrap();
    let deleted =
        crate::app_data::delete_note_permanently_app_data_json(&plain, "private-note", 200)
            .unwrap();
    store
        .compare_and_swap_account("user-a", 1, &deleted, 200)
        .unwrap();
    store.finish_note_privacy_cleanup().unwrap();
    let connection = store.open_connection(false).unwrap();
    let strip_media_fence = |connection: &Connection, table: &str| {
        let raw: String = connection
            .query_row(
                &format!("SELECT policy_json FROM {table} WHERE user_id='user-a'"),
                [],
                |r| r.get(0),
            )
            .unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        value.as_object_mut().unwrap().remove("media_deletions");
        let json = value.to_string();
        connection
            .execute(
                &format!(
                    "UPDATE {table} SET policy_json=?1,binding_sha256=?2 WHERE user_id='user-a'"
                ),
                params![json, note_privacy::binding("user-a", &json)],
            )
            .unwrap();
    };
    strip_media_fence(&connection, "account_note_privacy");
    let journal = Connection::open(privacy_journal::journal_path(store.database_path())).unwrap();
    strip_media_fence(&journal, "privacy_journal_policies");
    drop(journal);
    // Earlier releases cleaned JSON and history but left this deleted live row.
    connection.execute("INSERT INTO note_media VALUES('user-a',?1,?2,'application/octet-stream',?3,?4,100,200)",params![id,sha256_hex(marker.as_bytes()),marker.len() as i64,marker.as_bytes()]).unwrap();
    connection
        .execute("UPDATE account_note_privacy SET cleanup_pending=0", [])
        .unwrap();
    drop(connection);
    let path = store.database_path().to_path_buf();
    drop(store);
    let reopened = SqliteServerStore::open_with_options(ServerStoreOpenOptions {
        database_path: path.clone(),
        legacy_json_path: None,
        now_epoch_millis: 250,
        legacy_token_ttl_millis: DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
    })
    .unwrap()
    .store;
    reopened.finish_note_privacy_cleanup().unwrap();
    assert_eq!(
        0,
        media_privacy_live_count(&reopened, id),
        "restart trusted the old text-only cleanup state"
    );
    let connection = reopened.open_connection(false).unwrap();
    assert_eq!(
        Some(&200),
        note_privacy::read_policy(&connection, "user-a")
            .unwrap()
            .media_deletions()
            .get(id),
        "upgrade did not preserve the newly reconstructed media fence"
    );
    drop(connection);
    reopened.validate_integrity().unwrap();
    for file in [path.clone(), sqlite_sidecar_path(&path, "-wal")] {
        if file.exists() {
            let bytes = fs::read(file).unwrap();
            assert!(
                !bytes
                    .windows(marker.len())
                    .any(|window| window == marker.as_bytes()),
                "private BLOB marker remained in SQLite or WAL after cleanup"
            );
        }
    }
}

#[test]
fn server_privacy_media_encryption_alone_does_not_authorize_external_blob_deletion() {
    let directory = TestDirectory::new("media-privacy-sealed-attachment");
    let store = open_empty(&directory);
    store
        .create_user(new_user("user-a", "a@example.test"))
        .unwrap();
    let plain = media_privacy_document(&["opaque-attachment"]);
    store
        .compare_and_swap_account("user-a", 0, &plain, 100)
        .unwrap();
    media_privacy_upload(&store, "opaque-attachment", 100);
    let sealed = server_privacy_audit_sealed(&plain);
    store
        .compare_and_swap_account("user-a", 1, &sealed, 200)
        .unwrap();
    store.finish_note_privacy_cleanup().unwrap();
    assert_eq!(
        b"opaque-attachment",
        store
            .read_media("user-a", "opaque-attachment")
            .unwrap()
            .unwrap()
            .content
            .as_slice(),
        "sealing note metadata deleted an external file it did not encrypt"
    );
    store.validate_integrity().unwrap();
}

#[test]
fn server_privacy_media_interruption_rolls_back_account_and_blob_together() {
    for checkpoint in [1, 2, 3] {
        let directory = TestDirectory::new(&format!("media-privacy-interrupt-{checkpoint}"));
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        let plain = media_privacy_document(&["rollback-media"]);
        store
            .compare_and_swap_account("user-a", 0, &plain, 100)
            .unwrap();
        media_privacy_upload(&store, "rollback-media", 100);
        let deleted =
            crate::app_data::delete_note_permanently_app_data_json(&plain, "private-note", 200)
                .unwrap();
        privacy_journal::COMMIT_INTERRUPTION.with(|value| value.set(checkpoint));
        let result = store.compare_and_swap_account("user-a", 1, &deleted, 200);
        privacy_journal::COMMIT_INTERRUPTION.with(|value| value.set(0));
        assert!(result.is_err(), "fault injection did not execute");
        if checkpoint == 1 {
            assert_eq!(1, media_privacy_live_count(&store, "rollback-media"));
            assert_eq!(plain, store.read_account("user-a").unwrap().app_data_json);
            store
                .compare_and_swap_account("user-a", 1, &deleted, 200)
                .unwrap();
        }
        store.finish_note_privacy_cleanup().unwrap();
        assert_eq!(0, media_privacy_live_count(&store, "rollback-media"));
        store.validate_integrity().unwrap();
    }
}

#[test]
fn server_privacy_media_independent_fence_cleans_an_already_redacted_backup() {
    let directory = TestDirectory::new("media-privacy-independent-backup-fence");
    let store = open_empty(&directory);
    store
        .create_user(new_user("user-a", "a@example.test"))
        .unwrap();
    let id = "old-private-media";
    let plain = media_privacy_document(&[id]);
    store
        .compare_and_swap_account("user-a", 0, &plain, 100)
        .unwrap();
    media_privacy_upload(&store, id, 100);
    let deleted =
        crate::app_data::delete_note_permanently_app_data_json(&plain, "private-note", 200)
            .unwrap();
    store
        .compare_and_swap_account("user-a", 1, &deleted, 200)
        .unwrap();
    store.finish_note_privacy_cleanup().unwrap();
    let backup = directory.0.join("older-redacted-backup.sqlite3");
    store.create_verified_backup(&backup, 250).unwrap();
    // Model a previously cleaned archive: note content and its associations
    // are gone, but the former live BLOB remains with no media deletion fence.
    let copy = Connection::open(&backup).unwrap();
    copy.execute(
        "INSERT INTO note_media VALUES('user-a',?1,?2,'application/octet-stream',?3,?4,100,NULL)",
        params![
            id,
            sha256_hex(id.as_bytes()),
            id.len() as i64,
            id.as_bytes()
        ],
    )
    .unwrap();
    copy.execute("DELETE FROM note_media_tombstones", [])
        .unwrap();
    let mut data: serde_json::Value = serde_json::from_str(&deleted).unwrap();
    data["tombstones"]
        .as_array_mut()
        .unwrap()
        .retain(|item| item["entityType"] == "note");
    let redacted = data.to_string();
    copy.execute("UPDATE account_snapshots SET app_data_json=?1,content_sha256=?2,envelope_sha256=?3 WHERE user_id='user-a'",params![redacted,sha256_hex(redacted.as_bytes()),account_snapshot_envelope_sha256("user-a",&redacted,2,200,0)]).unwrap();
    let mut policy: serde_json::Value = serde_json::from_str(
        &copy
            .query_row(
                "SELECT policy_json FROM account_note_privacy WHERE user_id='user-a'",
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap(),
    )
    .unwrap();
    policy.as_object_mut().unwrap().remove("media_deletions");
    let policy = policy.to_string();
    copy.execute(
        "UPDATE account_note_privacy SET policy_json=?1,binding_sha256=?2 WHERE user_id='user-a'",
        params![policy, note_privacy::binding("user-a", &policy)],
    )
    .unwrap();
    drop(copy);
    SqliteServerStore::verify_existing_backup(&backup, 300).unwrap();
    SqliteServerStore::apply_external_privacy_to_backup_copy(&backup, store.database_path(), 350)
        .unwrap();
    let copy = Connection::open(&backup).unwrap();
    assert_eq!(
        0,
        copy.query_row(
            "SELECT COUNT(*) FROM note_media WHERE user_id='user-a' AND attachment_id=?1",
            [id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        "already-redacted backup forgot the independent attachment deletion fence"
    );
}

#[test]
fn server_privacy_media_supported_archive_schemas_keep_other_accounts_and_format() {
    let directory = TestDirectory::new("media-privacy-archive-schemas");
    let store = open_empty(&directory);
    for (user, email) in [("user-a", "a@example.test"), ("user-b", "b@example.test")] {
        store.create_user(new_user(user, email)).unwrap();
    }
    let id = "cross-account-file";
    let plain = media_privacy_document(&[id]);
    for user in ["user-a", "user-b"] {
        store
            .compare_and_swap_account(user, 0, &plain, 100)
            .unwrap();
        store
            .upsert_media(
                user,
                id,
                &sha256_hex(id.as_bytes()),
                "application/octet-stream",
                id.len() as i64,
                id.as_bytes(),
                100,
            )
            .unwrap();
        store
            .compare_and_swap_account(user, 1, &plain, 101)
            .unwrap();
    }
    let mut archives = Vec::new();
    for version in 10..=16 {
        let path = directory.0.join(format!("schema-{version}.sqlite3"));
        store.create_verified_backup(&path, 150).unwrap();
        let copy = Connection::open(&path).unwrap();
        if version < 15 {
            copy.execute_batch(
                "DROP TABLE account_note_privacy; DROP TABLE note_privacy_commit_witnesses;",
            )
            .unwrap();
        }
        if version <= 12 {
            copy.execute_batch("DROP TABLE account_snapshot_media_identities; DROP TABLE legacy_snapshot_repair_allowances; DROP INDEX account_snapshot_media_history_identity_index;
                DROP TRIGGER note_media_metadata_insert_guard; DROP TRIGGER note_media_metadata_update_guard;
                DROP TRIGGER note_media_tombstone_insert_guard; DROP TRIGGER note_media_tombstone_update_guard;
                DROP TRIGGER media_history_metadata_insert_guard; DROP TRIGGER media_history_metadata_update_guard;").unwrap();
        }
        if version == 10 {
            copy.execute_batch("DROP INDEX tokens_pending_activation_index; ALTER TABLE tokens DROP COLUMN activation_state; ALTER TABLE tokens DROP COLUMN activated_at_epoch_millis;").unwrap();
        }
        copy.execute("DELETE FROM schema_migrations WHERE version>?1", [version])
            .unwrap();
        copy.pragma_update(None, "user_version", version).unwrap();
        drop(copy);
        SqliteServerStore::verify_managed_privacy_backup(&path, 160).unwrap();
        archives.push((version, path));
    }
    let current = store.read_account("user-a").unwrap();
    let deleted = crate::app_data::delete_note_permanently_app_data_json(
        &current.app_data_json,
        "private-note",
        200,
    )
    .unwrap();
    store
        .compare_and_swap_account("user-a", current.revision, &deleted, 200)
        .unwrap();
    store.finish_note_privacy_cleanup().unwrap();
    for (version, path) in archives {
        SqliteServerStore::apply_external_privacy_to_backup_copy(&path, store.database_path(), 250)
            .unwrap();
        let copy = Connection::open(&path).unwrap();
        assert_eq!(
            if version == 15 {
                SCHEMA_VERSION
            } else {
                version
            },
            current_schema_version(&copy).unwrap(),
            "legacy layouts stay readable; private-policy archives require the new reader format"
        );
        assert_eq!(
            0,
            copy.query_row(
                "SELECT COUNT(*) FROM note_media WHERE user_id='user-a'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            "schema {version} retained deleted bytes"
        );
        assert_eq!(
            id.as_bytes(),
            copy.query_row(
                "SELECT content FROM note_media WHERE user_id='user-b' AND attachment_id=?1",
                [id],
                |r| r.get::<_, Vec<u8>>(0)
            )
            .unwrap()
        );
        assert_eq!(
            2,
            copy.query_row("SELECT COUNT(*) FROM users", [], |r| r.get::<_, i64>(0))
                .unwrap()
        );
        assert_eq!(
            1,
            copy.query_row(
                "SELECT COUNT(*) FROM media_snapshot_contents WHERE sha256=?1",
                [sha256_hex(id.as_bytes())],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            "other account lost shared historical content"
        );
        drop(copy);
        SqliteServerStore::verify_managed_privacy_backup(&path, 260).unwrap();
    }
}

#[test]
fn server_privacy_media_fences_merge_monotonically_and_reject_invalid_entries() {
    use crate::desktop_state_store::DesktopPrivacyPolicy;
    use std::collections::BTreeMap;
    let older = DesktopPrivacyPolicy::default()
        .including_media_deletions(&BTreeMap::from([("old-media".into(), 100)]))
        .unwrap();
    let newer = older
        .including_media_deletions(&BTreeMap::from([
            ("old-media".into(), 200),
            ("other-media".into(), 150),
        ]))
        .unwrap();
    let encoded = serde_json::to_string(&newer).unwrap();
    let restored: DesktopPrivacyPolicy = serde_json::from_str(&encoded).unwrap();
    for merged in [
        older.merged_policy(&restored).unwrap(),
        restored.merged_policy(&older).unwrap(),
    ] {
        assert_eq!(Some(&200), merged.media_deletions().get("old-media"));
        assert_eq!(Some(&150), merged.media_deletions().get("other-media"));
    }
    for (id, revision) in [
        ("../escape", 300),
        ("bad media", 300),
        ("valid-media", 0),
        ("valid-media", -1),
    ] {
        assert!(older
            .including_media_deletions(&BTreeMap::from([(id.into(), revision)]))
            .is_err());
    }
    let old_format: DesktopPrivacyPolicy =
        serde_json::from_str(r#"{"seals":{},"deletions":{"old-note":100}}"#).unwrap();
    assert!(old_format.media_deletions().is_empty());
}

#[test]
fn server_privacy_media_shared_file_hidden_inside_another_sealed_note_survives() {
    let directory = TestDirectory::new("media-privacy-shared-sealed-reference");
    let store = open_empty(&directory);
    store
        .create_user(new_user("user-a", "a@example.test"))
        .unwrap();
    let id = "sealed-shared-file";
    let mut value: serde_json::Value =
        serde_json::from_str(&media_privacy_document(&[id])).unwrap();
    let mut other = value["notes"][0].clone();
    other["id"] = serde_json::json!("sealed-other-note");
    value["notes"].as_array_mut().unwrap().push(other.clone());
    let plain = crate::app_data::sanitize_app_data_json(&value.to_string(), 100).unwrap();
    store
        .compare_and_swap_account("user-a", 0, &plain, 100)
        .unwrap();
    media_privacy_upload(&store, id, 100);
    let (sealed, session) =
        crate::encrypt_desktop_note_json(&other.to_string(), "synthetic-only-password").unwrap();
    crate::close_desktop_note_session(&session);
    let sealed = crate::app_data::upsert_note_app_data_json(&plain, &sealed, 200).unwrap();
    store
        .compare_and_swap_account("user-a", 1, &sealed, 200)
        .unwrap();
    let deleted =
        crate::app_data::delete_note_permanently_app_data_json(&sealed, "private-note", 300)
            .unwrap();
    store
        .compare_and_swap_account("user-a", 2, &deleted, 300)
        .unwrap();
    store.finish_note_privacy_cleanup().unwrap();
    assert_eq!(
        1,
        media_privacy_live_count(&store, id),
        "purge lost another sealed note's shared external attachment"
    );
}

#[test]
fn server_privacy_media_empty_legacy_snapshot_remains_valid_during_overlay() {
    let directory = TestDirectory::new("media-privacy-empty-legacy");
    let store = open_empty(&directory);
    let mut user = new_user("user-a", "a@example.test");
    user.app_data_json = String::new();
    store.create_user(user).unwrap();
    let note = media_privacy_document(&["deleted-media"]);
    let deleted =
        crate::app_data::delete_note_permanently_app_data_json(&note, "private-note", 200).unwrap();
    let policy = crate::desktop_state_store::DesktopPrivacyPolicy::default()
        .including_snapshot(&deleted)
        .unwrap();
    let mut connection = store.open_connection(false).unwrap();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    note_privacy::overlay_policy(&transaction, "user-a", &policy)
        .expect("valid empty legacy account failed privacy overlay");
    transaction.commit().unwrap();
    assert_eq!("", store.read_account("user-a").unwrap().app_data_json);
    store.validate_integrity().unwrap();
}

fn media_privacy_legacy_pair(shared: bool) -> (TestDirectory, SqliteServerStore, String) {
    let directory = TestDirectory::new("legacy-sealed-media-reference-work");
    let store = open_empty(&directory);
    store
        .create_user(new_user("user-a", "a@example.test"))
        .unwrap();
    let mut value: serde_json::Value =
        serde_json::from_str(&media_privacy_document(&["guarded-file"])).unwrap();
    let mut other = value["notes"][0].clone();
    other["id"] = serde_json::json!("legacy-other-note");
    if !shared {
        other["attachments"] = serde_json::json!([]);
    }
    value["notes"].as_array_mut().unwrap().push(other.clone());
    let plain = crate::app_data::sanitize_app_data_json(&value.to_string(), 100).unwrap();
    store
        .compare_and_swap_account("user-a", 0, &plain, 100)
        .unwrap();
    media_privacy_upload(&store, "guarded-file", 100);
    let (sealed, session) = crate::legacy_note_crypto_fixture::encrypt_note(
        &other.to_string(),
        "synthetic-only-password",
    )
    .unwrap();
    crate::legacy_note_crypto_fixture::close_session(&session);
    let next = crate::app_data::upsert_note_app_data_json(&plain, &sealed, 200).unwrap();
    store
        .compare_and_swap_account("user-a", 1, &next, 200)
        .unwrap();
    let deleted =
        crate::app_data::delete_note_permanently_app_data_json(&next, "private-note", 300).unwrap();
    store
        .compare_and_swap_account("user-a", 2, &deleted, 300)
        .unwrap();
    (directory, store, sealed)
}

fn media_privacy_resolve_legacy(store: &SqliteServerStore, sealed: &str) {
    let (unlocked, token) =
        crate::unlock_desktop_note_json(sealed, "synthetic-only-password").unwrap();
    let declaration = crate::note_crypto::session_media_references_json(&unlocked, &token).unwrap();
    crate::close_desktop_note_session(&token);
    let note: serde_json::Value = serde_json::from_str(sealed).unwrap();
    let current = store.read_account("user-a").unwrap();
    assert!(store
        .register_sealed_media_references(
            "user-a",
            note["id"].as_str().unwrap(),
            current.revision,
            0,
            &declaration
        )
        .unwrap());
    assert_eq!(
        current.app_data_json,
        store.read_account("user-a").unwrap().app_data_json
    );
    assert_eq!(
        current.revision,
        store.read_account("user-a").unwrap().revision
    );
}

#[test]
fn server_privacy_media_legacy_unknown_is_retained_and_reported_across_restart() {
    let (_directory, store, sealed) = media_privacy_legacy_pair(true);
    store.finish_note_privacy_cleanup().unwrap();
    assert_eq!(1, media_privacy_live_count(&store, "guarded-file"));
    assert_eq!(
        1,
        store
            .media_privacy_status()
            .unwrap()
            .awaiting_reference_resolution
    );
    let path = store.database_path.clone();
    drop(store);
    let reopened = SqliteServerStore::open(path, None).unwrap();
    assert_eq!(
        1,
        reopened
            .media_privacy_status()
            .unwrap()
            .awaiting_reference_resolution
    );
    media_privacy_resolve_legacy(&reopened, &sealed);
    assert_eq!(1, media_privacy_live_count(&reopened, "guarded-file"));
    assert_eq!(
        MediaPrivacyStatus::default(),
        reopened.media_privacy_status().unwrap()
    );
    // A later ordinary deletion can now prove that no retained note uses it.
    let current = reopened.read_account("user-a").unwrap();
    let deleted = crate::app_data::delete_note_permanently_app_data_json(
        &current.app_data_json,
        "legacy-other-note",
        500,
    )
    .unwrap();
    reopened
        .compare_and_swap_account("user-a", current.revision, &deleted, 500)
        .unwrap();
    assert_eq!(0, media_privacy_live_count(&reopened, "guarded-file"));
    assert_eq!(
        MediaPrivacyStatus::default(),
        reopened.media_privacy_status().unwrap()
    );
}

#[test]
fn server_privacy_media_legacy_empty_reference_resolution_finishes_deferred_cleanup() {
    let (_directory, store, sealed) = media_privacy_legacy_pair(false);
    store.finish_note_privacy_cleanup().unwrap();
    assert_eq!(1, media_privacy_live_count(&store, "guarded-file"));
    assert_eq!(
        1,
        store
            .media_privacy_status()
            .unwrap()
            .awaiting_reference_resolution
    );
    media_privacy_resolve_legacy(&store, &sealed);
    assert_eq!(0, media_privacy_live_count(&store, "guarded-file"));
    assert_eq!(
        MediaPrivacyStatus::default(),
        store.media_privacy_status().unwrap()
    );
    store.finish_note_privacy_cleanup().unwrap();
    store.validate_integrity().unwrap();
}

#[test]
fn server_privacy_media_lost_legacy_note_provenance_is_never_reported_as_complete() {
    let directory = TestDirectory::new("legacy-deleted-opaque-media");
    let store = open_empty(&directory);
    store
        .create_user(new_user("user-a", "a@example.test"))
        .unwrap();
    let plain = media_privacy_document(&["unresolved-file"]);
    store
        .compare_and_swap_account("user-a", 0, &plain, 100)
        .unwrap();
    media_privacy_upload(&store, "unresolved-file", 100);
    let value: serde_json::Value = serde_json::from_str(&plain).unwrap();
    let (sealed, session) = crate::legacy_note_crypto_fixture::encrypt_note(
        &value["notes"][0].to_string(),
        "synthetic-only-password",
    )
    .unwrap();
    crate::legacy_note_crypto_fixture::close_session(&session);
    let encrypted = crate::app_data::upsert_note_app_data_json(&plain, &sealed, 200).unwrap();
    store
        .compare_and_swap_account("user-a", 1, &encrypted, 200)
        .unwrap();
    let deleted =
        crate::app_data::delete_note_permanently_app_data_json(&encrypted, "private-note", 300)
            .unwrap();
    store
        .compare_and_swap_account("user-a", 2, &deleted, 300)
        .unwrap();
    store.finish_note_privacy_cleanup().unwrap();
    assert_eq!(1, media_privacy_live_count(&store, "unresolved-file"));
    assert_eq!(
        1,
        store.media_privacy_status().unwrap().opaque_note_deletions
    );
}

// v0.0.1 - Keep private declaration submission scoped to one retained account state.
#[test]
fn server_privacy_media_sealed_media_reference_submission_checks_account_revision_and_generation() {
    let (_directory, store, sealed) = media_privacy_legacy_pair(false);
    let (unlocked, token) =
        crate::unlock_desktop_note_json(&sealed, "synthetic-only-password").unwrap();
    let declaration = crate::note_crypto::session_media_references_json(&unlocked, &token).unwrap();
    crate::close_desktop_note_session(&token);
    let before = store.read_account("user-a").unwrap();
    for (user, note, revision, generation) in [
        ("other-account", "legacy-other-note", before.revision, 0),
        ("user-a", "missing-note", before.revision, 0),
        ("user-a", "legacy-other-note", before.revision - 1, 0),
        ("user-a", "legacy-other-note", before.revision, 1),
    ] {
        assert!(!store
            .register_sealed_media_references(user, note, revision, generation, &declaration)
            .unwrap());
    }
    let mut corrupted: serde_json::Value = serde_json::from_str(&declaration).unwrap();
    corrupted["envelopeSha256"] = serde_json::json!("0".repeat(64));
    assert!(store
        .register_sealed_media_references(
            "user-a",
            "legacy-other-note",
            before.revision,
            0,
            &corrupted.to_string()
        )
        .is_err());
    assert_eq!(1, media_privacy_live_count(&store, "guarded-file"));
    assert_eq!(
        1,
        store
            .media_privacy_status()
            .unwrap()
            .awaiting_reference_resolution
    );
    assert!(store
        .register_sealed_media_references(
            "user-a",
            "legacy-other-note",
            before.revision,
            0,
            &declaration
        )
        .unwrap());
    assert_eq!(0, media_privacy_live_count(&store, "guarded-file"));
    assert_eq!(
        MediaPrivacyStatus::default(),
        store.media_privacy_status().unwrap()
    );
    assert_eq!(
        before.app_data_json,
        store.read_account("user-a").unwrap().app_data_json
    );
    assert_eq!(
        before.revision,
        store.read_account("user-a").unwrap().revision
    );
    let path = store.database_path.clone();
    drop(store);
    let reopened = SqliteServerStore::open(path, None).unwrap();
    assert_eq!(
        MediaPrivacyStatus::default(),
        reopened.media_privacy_status().unwrap()
    );
    assert_eq!(0, media_privacy_live_count(&reopened, "guarded-file"));
    reopened.validate_integrity().unwrap();
}

// v0.0.1 - Exercise ordinary DELETE and retention pruning against opaque shared references.
#[test]
fn server_privacy_media_delete_api_preserves_a_shared_sealed_file() {
    let (_directory, store, _) = media_privacy_legacy_pair(true);
    let current = store.read_account("user-a").unwrap();
    // An installed older client can still submit this incorrect deletion intent.
    let mut raw: serde_json::Value = serde_json::from_str(&current.app_data_json).unwrap();
    raw["tombstones"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "entityType":"noteMedia","entityId":"guarded-file","deletedAtEpochMillis":300
        }));
    store
        .compare_and_swap_account("user-a", current.revision, &raw.to_string(), 400)
        .unwrap();
    let result = store.delete_media("user-a", "guarded-file", 300, 400);
    assert!(
        store
            .read_media("user-a", "guarded-file")
            .unwrap()
            .is_some(),
        "ordinary DELETE hid another sealed note's shared attachment: {result:?}"
    );
    assert!(
        result.is_err(),
        "an unresolved reference must not be reported as deleted"
    );
    store
        .prune_deleted_media_content(500 + DELETED_MEDIA_CONTENT_RETENTION_MILLIS)
        .unwrap();
    assert_eq!(1, media_privacy_live_count(&store, "guarded-file"));
}

#[test]
fn server_privacy_media_retention_prune_keeps_opaque_bytes_from_older_delete_records() {
    let (_directory, store, _) = media_privacy_legacy_pair(true);
    let connection = store.open_connection(false).unwrap();
    // Simulate a tombstone already committed by the installed pre-fix API.
    connection
        .execute(
            "INSERT INTO note_media_tombstones VALUES(?1,?2,?3,?4)",
            params!["user-a", "guarded-file", 300, 400],
        )
        .unwrap();
    connection.execute("UPDATE note_media SET deleted_at_epoch_millis=300 WHERE user_id='user-a' AND attachment_id='guarded-file'",[]).unwrap();
    drop(connection);
    assert_eq!(
        0,
        store
            .prune_deleted_media_content(500 + DELETED_MEDIA_CONTENT_RETENTION_MILLIS)
            .unwrap(),
        "retention cleanup erased bytes hidden inside another sealed note"
    );
    assert_eq!(
        1,
        store.media_privacy_status().unwrap().retained_deleted_media
    );
    assert!(matches!(
        store.delete_media("user-a", "guarded-file", 300, 500),
        Err(StoreError::MediaReferenceConflict { opaque: true })
    ));
    let connection = store.open_connection(false).unwrap();
    let count:i64=connection.query_row("SELECT COUNT(*) FROM note_media WHERE user_id='user-a' AND attachment_id='guarded-file'",[],|row|row.get(0)).unwrap();
    assert_eq!(1, count);
}

// v0.0.1 - Cover account isolation, history-only ciphertext and resolving deferred DELETE.
#[test]
fn server_privacy_media_delete_guard_preserves_other_accounts_and_resumes_after_resolution() {
    let (_directory, store, sealed) = media_privacy_legacy_pair(false);
    media_privacy_upload(&store, "unassociated-file", 100);
    assert!(matches!(
        store.delete_media("user-a", "unassociated-file", 300, 400),
        Err(StoreError::MediaReferenceConflict { opaque: true })
    ));
    store
        .create_user(new_user("user-b", "b@example.test"))
        .unwrap();
    let content = b"other-account-file";
    store
        .upsert_media(
            "user-b",
            "unassociated-file",
            &sha256_hex(content),
            "application/octet-stream",
            content.len() as i64,
            content,
            100,
        )
        .unwrap();
    store
        .delete_media("user-b", "unassociated-file", 300, 400)
        .unwrap();
    assert_eq!(
        1,
        store
            .prune_deleted_media_content(500 + DELETED_MEDIA_CONTENT_RETENTION_MILLIS)
            .unwrap()
    );
    assert!(store
        .read_media("user-a", "unassociated-file")
        .unwrap()
        .is_some());
    media_privacy_resolve_legacy(&store, &sealed);
    assert!(store
        .delete_media("user-a", "unassociated-file", 300, 400)
        .unwrap()
        .deleted_at_epoch_millis
        .is_some());
    assert_eq!(
        1,
        store
            .prune_deleted_media_content(500 + DELETED_MEDIA_CONTENT_RETENTION_MILLIS)
            .unwrap()
    );
    store.validate_integrity().unwrap();
}

#[test]
fn server_privacy_media_delete_guard_includes_ciphertext_retained_only_in_history() {
    let (_directory, store, _) = media_privacy_legacy_pair(true);
    let current = store.read_account("user-a").unwrap();
    let mut raw: serde_json::Value = serde_json::from_str(&current.app_data_json).unwrap();
    // A snapshot may omit a note without declaring permanent deletion.
    raw["notes"] = serde_json::json!([]);
    store
        .compare_and_swap_account("user-a", current.revision, &raw.to_string(), 400)
        .unwrap();
    assert!(matches!(
        store.delete_media("user-a", "guarded-file", 500, 500),
        Err(StoreError::MediaReferenceConflict { opaque: true })
    ));
    assert!(store
        .read_media("user-a", "guarded-file")
        .unwrap()
        .is_some());
}

#[test]
fn server_privacy_media_delete_guard_retains_future_or_unknown_reference_formats() {
    for raw in [r#"{"schemaVersion":2147483647,"notes":[]}"#.to_string(), {
        let mut value: serde_json::Value =
            serde_json::from_str(&crate::app_data::default_app_data_json(100)).unwrap();
        value["futureAttachmentIndex"] = serde_json::json!(["format-file"]);
        value.to_string()
    }] {
        let directory = TestDirectory::new("media-delete-future-reference-shape");
        let store = open_empty(&directory);
        store
            .create_user(new_user("user-a", "a@example.test"))
            .unwrap();
        media_privacy_upload(&store, "format-file", 100);
        let write = store.compare_and_swap_account("user-a", 0, &raw, 100);
        if raw.contains("2147483647") {
            assert!(matches!(write, Err(StoreError::Integrity(_))));
            assert_eq!(0, store.read_account("user-a").unwrap().revision);
            assert!(store.read_media("user-a", "format-file").unwrap().is_some());
            continue;
        }
        write.unwrap();
        assert!(matches!(
            store.delete_media("user-a", "format-file", 200, 200),
            Err(StoreError::MediaReferenceConflict { opaque: true })
        ));
        assert!(store.read_media("user-a", "format-file").unwrap().is_some());
    }
}

// v0.0.1 - Do not erase the live fallback while the historical byte copy is damaged.
#[test]
fn server_privacy_media_retention_requires_a_verified_historical_copy() {
    let directory = TestDirectory::new("media-delete-history-byte-proof");
    let store = open_empty(&directory);
    store
        .create_user(new_user("user-a", "a@example.test"))
        .unwrap();
    let content = b"retained-history-fallback";
    let hash = sha256_hex(content);
    let historical = json_with_attachment("history-file", content);
    store
        .compare_and_swap_account("user-a", 0, &historical, 100)
        .unwrap();
    store
        .upsert_media(
            "user-a",
            "history-file",
            &hash,
            "application/octet-stream",
            content.len() as i64,
            content,
            101,
        )
        .unwrap();
    assert!(matches!(
        store.delete_media("user-a", "history-file", 200, 200),
        Err(StoreError::MediaReferenceConflict { opaque: false })
    ));
    store
        .compare_and_swap_account("user-a", 1, r#"{"notes":[]}"#, 200)
        .unwrap();
    store
        .delete_media("user-a", "history-file", 300, 300)
        .unwrap();
    let mut damaged = content.to_vec();
    damaged[0] ^= 1;
    let connection = store.open_connection(false).unwrap();
    assert_eq!(
        1,
        connection
            .execute(
                "UPDATE media_snapshot_contents SET content=?1 WHERE sha256=?2",
                params![damaged, hash]
            )
            .unwrap()
    );
    drop(connection);
    assert_eq!(
        0,
        store
            .prune_deleted_media_content(500 + DELETED_MEDIA_CONTENT_RETENTION_MILLIS)
            .unwrap()
    );
    assert_eq!(
        1,
        store.media_privacy_status().unwrap().retained_deleted_media
    );
    let connection = store.open_connection(false).unwrap();
    connection
        .execute(
            "UPDATE media_snapshot_contents SET content=?1 WHERE sha256=?2",
            params![content.as_slice(), hash],
        )
        .unwrap();
    drop(connection);
    assert_eq!(
        1,
        store
            .prune_deleted_media_content(500 + DELETED_MEDIA_CONTENT_RETENTION_MILLIS)
            .unwrap()
    );
    let restored = store.restore_account_snapshot("user-a", 1, 2, 600).unwrap();
    assert_eq!(historical, restored.app_data_json);
    assert_eq!(
        content.as_slice(),
        store
            .read_media("user-a", "history-file")
            .unwrap()
            .unwrap()
            .content
            .as_slice()
    );
    store.validate_integrity().unwrap();
}

#[test]
fn private_commit_metadata_rejects_an_unapplied_note_privacy_fence() {
    let directory = TestDirectory::new("reference-only-fence-rejection");
    let store = open_empty(&directory);
    store
        .create_user(new_user("user-a", "reference-fence@example.test"))
        .unwrap();
    let plain = server_privacy_audit_plain();
    store
        .compare_and_swap_account("user-a", 0, &plain, 100)
        .unwrap();
    let deleted =
        crate::app_data::delete_note_permanently_app_data_json(&plain, "private-note", 200)
            .unwrap();
    let fence = crate::desktop_state_store::DesktopPrivacyPolicy::default()
        .including_snapshot(&deleted)
        .unwrap();
    let mut connection = store.open_connection(false).unwrap();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let before = note_privacy::read_policy(&transaction, "user-a").unwrap();
    assert!(
        note_privacy::record_reference_metadata(&transaction, "user-a", fence).is_err(),
        "a metadata-only path must reject a new note deletion fence"
    );
    assert_eq!(
        note_privacy::read_policy(&transaction, "user-a").unwrap(),
        before
    );
    assert_eq!(
        read_account_in_transaction(&transaction, "user-a")
            .unwrap()
            .unwrap()
            .app_data_json,
        plain
    );
}

#[test]
fn media_intent_reference_scope_is_revision_bound_and_excludes_independent_archives() {
    let directory = TestDirectory::new("media-intent-reference-scope");
    let store = open_empty(&directory);
    store
        .create_user(new_user("user-a", "a@example.test"))
        .unwrap();
    let previous = media_privacy_document(&["archive-only"]);
    store
        .compare_and_swap_account("user-a", 0, &previous, 100)
        .unwrap();
    let combined = media_privacy_document(&["shared-current"]);
    let before = store.read_account("user-a").unwrap();
    let scope = store
        .media_deletion_reference_scope("user-a", 1, &combined)
        .unwrap();
    assert_eq!(
        HashSet::from(["shared-current".to_string()]),
        scope.referenced_ids
    );
    assert!(!scope.unresolved);
    assert_eq!(before, store.read_account("user-a").unwrap());
    assert!(matches!(
        store.media_deletion_reference_scope("user-a", 0, &combined),
        Err(StoreError::RevisionConflict {
            expected_revision: 0,
            actual_revision: 1
        })
    ));
    let mut malformed: serde_json::Value = serde_json::from_str(&combined).unwrap();
    malformed["notes"] = serde_json::json!([]);
    malformed["syncConflictHistory"] = serde_json::json!([{
        "entityType":"note", "entityId":"different-owner",
        "payload":{"id":"payload-owner", "attachments":[{"id":"ghost"}]}
    }]);
    let scope = store
        .media_deletion_reference_scope("user-a", 1, &malformed.to_string())
        .unwrap();
    assert!(scope.unresolved);
    assert!(scope.referenced_ids.is_empty());
    malformed["syncConflictHistory"][0]["entityType"] = serde_json::json!("session");
    let scope = store
        .media_deletion_reference_scope("user-a", 1, &malformed.to_string())
        .unwrap();
    assert!(!scope.unresolved);
    assert!(scope.referenced_ids.is_empty());
}

fn typed_conflict_media_document(id: &str) -> String {
    let raw = media_privacy_document(&[id]);
    let mut value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let note = value["notes"][0].clone();
    value["notes"] = serde_json::json!([]);
    value["syncConflictHistory"] = serde_json::json!([{
        "id":"typed-conflict", "entityType":"note", "entityId":"private-note",
        "losingRevisionEpochMillis":100, "capturedAtEpochMillis":110, "payload":note
    }]);
    value.to_string()
}

#[test]
fn typed_note_conflict_keeps_verified_history_bytes_after_live_gc() {
    let directory = TestDirectory::new("typed-conflict-media-history");
    let store = open_empty(&directory);
    let raw = typed_conflict_media_document("conflict-media");
    let mut user = new_user("user-a", "a@example.test");
    user.app_data_json = raw.clone();
    store.create_user(user).unwrap();
    media_privacy_upload(&store, "conflict-media", 100);
    store
        .compare_and_swap_account("user-a", 0, "{\"notes\":[]}", 150)
        .unwrap();
    store
        .delete_media("user-a", "conflict-media", 200, 200)
        .unwrap();
    assert_eq!(
        1,
        store
            .prune_deleted_media_content(201 + DELETED_MEDIA_CONTENT_RETENTION_MILLIS)
            .unwrap()
    );
    let restored = store.restore_account_snapshot("user-a", 0, 1, 300).unwrap();
    assert_eq!(raw, restored.app_data_json);
    assert_eq!(
        b"conflict-media",
        store
            .read_media("user-a", "conflict-media")
            .unwrap()
            .unwrap()
            .content
            .as_slice()
    );
    store.validate_integrity().unwrap();
}

fn opaque_restore_retained_history(
    store: &SqliteServerStore,
    sealed: &str,
) -> AccountSnapshotHistory {
    let sealed: serde_json::Value = serde_json::from_str(sealed).unwrap();
    let history = store
        .list_snapshot_history("user-a", 100)
        .unwrap()
        .into_iter()
        .filter(|entry| {
            let value: serde_json::Value = serde_json::from_str(&entry.app_data_json).unwrap();
            // The initial account history is the valid legacy empty object
            // `{}`. It has no notes and is not the ciphertext target.
            value
                .get("notes")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|notes| {
                    notes.iter().any(|note| {
                        note["id"] == sealed["id"] && note["encryption"] == sealed["encryption"]
                    })
                })
        })
        .max_by_key(|entry| entry.revision)
        .expect("fixture must retain the exact ciphertext in real account history");
    assert!(history.revision < store.read_account("user-a").unwrap().revision);
    history
}

fn opaque_restore_atomic_state(
    store: &SqliteServerStore,
) -> (
    i64,
    Vec<(i64, i64, i64, Option<i64>, String)>,
    Vec<(String, i64, i64)>,
) {
    let connection = store.open_connection(false).unwrap();
    let generation = connection
        .query_row(
            "SELECT restore_generation FROM account_snapshots WHERE user_id='user-a'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    let mut statement = connection.prepare("SELECT id,restore_acknowledged,last_seen_restore_generation,pending_restore_generation,pending_restore_receipt FROM tokens WHERE user_id='user-a' ORDER BY id").unwrap();
    let tokens = statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let mut statement = connection.prepare("SELECT attachment_id,deleted_revision_epoch_millis,recorded_at_epoch_millis FROM note_media_tombstones WHERE user_id='user-a' ORDER BY attachment_id").unwrap();
    let tombstones = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    (generation, tokens, tombstones)
}

#[test]
fn opaque_account_history_restore_cannot_hide_required_live_bytes() {
    let (_directory, store, sealed) = media_privacy_legacy_pair(true);
    let current = store.read_account("user-a").unwrap();
    let mut omitted: serde_json::Value = serde_json::from_str(&current.app_data_json).unwrap();
    omitted["notes"] = serde_json::json!([]);
    store
        .compare_and_swap_account("user-a", current.revision, &omitted.to_string(), 350)
        .unwrap();
    let token = store
        .issue_token("user-a", "opaque-restore-test", "test-device", 400, 10_000)
        .unwrap();
    let receipt = store.ensure_restore_receipt("user-a", token.id).unwrap();
    store
        .acknowledge_restore_generation(
            "user-a",
            token.id,
            receipt.current_generation,
            &receipt.receipt,
        )
        .unwrap();
    let historical = opaque_restore_retained_history(&store, &sealed);
    let before = store.read_account("user-a").unwrap();
    let before_state = opaque_restore_atomic_state(&store);
    assert_eq!(1, before_state.1.len());
    let bytes = store
        .read_media("user-a", "guarded-file")
        .unwrap()
        .unwrap()
        .content;
    let result =
        store.restore_account_snapshot("user-a", historical.revision, before.revision, 500);
    assert!(matches!(result,Err(StoreError::Integrity(_))),
        "unknown existing history must fail because references are incomplete, not because it is absent: {result:?}");
    assert_eq!(before, store.read_account("user-a").unwrap());
    assert_eq!(before_state, opaque_restore_atomic_state(&store));
    assert_eq!(
        bytes,
        store
            .read_media("user-a", "guarded-file")
            .unwrap()
            .unwrap()
            .content
    );
    let connection = store.open_connection(false).unwrap();
    assert_eq!(0,connection.query_row("SELECT media_snapshot_complete FROM account_snapshot_history WHERE user_id='user-a' AND revision=?1",[historical.revision],|row|row.get::<_,i64>(0)).unwrap());
    // Reproduce the old writer's false-complete cache with no visible manifest.
    // The restore reader must independently reject unknown ciphertext even if
    // this cache is stale; current plaintext can otherwise archive successfully.
    connection.execute("DELETE FROM account_snapshot_media_history WHERE user_id='user-a' AND account_revision=?1",[historical.revision]).unwrap();
    connection.execute("UPDATE account_snapshot_history SET media_snapshot_complete=1 WHERE user_id='user-a' AND revision=?1",[historical.revision]).unwrap();
    drop(connection);
    assert!(matches!(
        store.restore_account_snapshot("user-a", historical.revision, before.revision, 501),
        Err(StoreError::Integrity(_))
    ));
    assert_eq!(before, store.read_account("user-a").unwrap());
    assert_eq!(before_state, opaque_restore_atomic_state(&store));
    assert_eq!(
        bytes,
        store
            .read_media("user-a", "guarded-file")
            .unwrap()
            .unwrap()
            .content
    );
}

#[test]
fn resolved_opaque_account_history_restore_preserves_declared_bytes() {
    let (_directory, store, sealed) = media_privacy_legacy_pair(true);
    media_privacy_resolve_legacy(&store, &sealed);
    let historical = opaque_restore_retained_history(&store, &sealed);
    let before = store.read_account("user-a").unwrap();
    let bytes = store
        .read_media("user-a", "guarded-file")
        .unwrap()
        .unwrap()
        .content;
    let restored = store
        .restore_account_snapshot("user-a", historical.revision, before.revision, 500)
        .unwrap();
    assert_eq!(historical.app_data_json, restored.app_data_json);
    assert_eq!(
        bytes,
        store
            .read_media("user-a", "guarded-file")
            .unwrap()
            .unwrap()
            .content
    );
    store.validate_integrity().unwrap();
}

#[test]
fn resolved_empty_opaque_account_history_restore_remains_available() {
    let (_directory, store, sealed) = media_privacy_legacy_pair(false);
    media_privacy_resolve_legacy(&store, &sealed);
    let historical = opaque_restore_retained_history(&store, &sealed);
    let before = store.read_account("user-a").unwrap();
    let restored = store
        .restore_account_snapshot("user-a", historical.revision, before.revision, 500)
        .unwrap();
    assert_eq!(historical.app_data_json, restored.app_data_json);
    assert_eq!(before.revision + 1, restored.revision);
    store.validate_integrity().unwrap();
}

#[test]
fn resolved_opaque_history_restores_exact_bytes_after_live_retention_gc() {
    let (_directory, store, sealed) = media_privacy_legacy_pair(true);
    media_privacy_resolve_legacy(&store, &sealed);
    let target = opaque_restore_retained_history(&store, &sealed);
    let current = store.read_account("user-a").unwrap();
    let mut omitted: serde_json::Value = serde_json::from_str(&current.app_data_json).unwrap();
    omitted["notes"] = serde_json::json!([]);
    store
        .compare_and_swap_account("user-a", current.revision, &omitted.to_string(), 400)
        .unwrap();
    store
        .delete_media("user-a", "guarded-file", 500, 500)
        .unwrap();
    assert_eq!(
        1,
        store
            .prune_deleted_media_content(501 + DELETED_MEDIA_CONTENT_RETENTION_MILLIS)
            .unwrap()
    );
    assert_eq!(0, media_privacy_live_count(&store, "guarded-file"));
    let reopened = SqliteServerStore::open(store.database_path(), None).unwrap();
    let restored = reopened
        .restore_account_snapshot("user-a", target.revision, current.revision + 1, 600)
        .unwrap();
    assert_eq!(target.app_data_json, restored.app_data_json);
    assert_eq!(
        b"guarded-file",
        reopened
            .read_media("user-a", "guarded-file")
            .unwrap()
            .unwrap()
            .content
            .as_slice()
    );
    reopened.validate_integrity().unwrap();
}

#[test]
fn sealed_history_declaration_requires_same_account_bytes_and_later_matching_upload() {
    // An attachment ID cannot be rebound to different bytes. Exercise the
    // wrong-content rejection and later matching upload in independent stores.
    for wrong_content_first in [true, false] {
        let directory = TestDirectory::new(if wrong_content_first {
            "sealed-history-wrong-content"
        } else {
            "sealed-history-later-upload"
        });
        let store = open_empty(&directory);
        let raw = media_privacy_document(&["guarded-file"]);
        let document: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let (sealed, session) = crate::legacy_note_crypto_fixture::encrypt_note(
            &document["notes"][0].to_string(),
            "synthetic-only-password",
        )
        .unwrap();
        crate::legacy_note_crypto_fixture::close_session(&session);
        let mut user = new_user("user-a", "a@example.test");
        user.app_data_json = crate::app_data::upsert_note_app_data_json(
            &crate::app_data::default_app_data_json(100),
            &sealed,
            200,
        )
        .unwrap();
        store.create_user(user).unwrap();
        // Another account has the exact ID and hash, including deduplicated history.
        let mut other = new_user("user-b", "b@example.test");
        other.app_data_json = raw;
        store.create_user(other).unwrap();
        store
            .upsert_media(
                "user-b",
                "guarded-file",
                &sha256_hex(b"guarded-file"),
                "application/octet-stream",
                12,
                b"guarded-file",
                100,
            )
            .unwrap();
        let other_before = store.read_media("user-b", "guarded-file").unwrap();
        media_privacy_resolve_legacy(&store, &sealed);
        let complete = || {
            store
                .open_connection(false)
                .unwrap()
                .query_row(
                    "SELECT media_snapshot_complete FROM account_snapshot_history WHERE user_id='user-a' AND revision=0",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
        };
        assert_eq!(
            0,
            complete(),
            "another account's content hash authorized history recovery"
        );
        assert!(matches!(
            store.restore_account_snapshot("user-a", 0, 0, 300),
            Err(StoreError::Integrity(_))
        ));
        if wrong_content_first {
            store
                .upsert_media(
                    "user-a",
                    "guarded-file",
                    &sha256_hex(b"wrong-bytes"),
                    "application/octet-stream",
                    11,
                    b"wrong-bytes",
                    301,
                )
                .unwrap();
            assert_eq!(
                0,
                complete(),
                "a different content hash completed the ciphertext history"
            );
            let account_before = store.read_account("user-a").unwrap();
            let atomic_before = opaque_restore_atomic_state(&store);
            let media_before = store.read_media("user-a", "guarded-file").unwrap();
            assert_eq!(
                b"wrong-bytes",
                media_before.as_ref().unwrap().content.as_slice()
            );
            assert!(matches!(
                store.upsert_media(
                    "user-a",
                    "guarded-file",
                    &sha256_hex(b"guarded-file"),
                    "application/octet-stream",
                    12,
                    b"guarded-file",
                    302,
                ),
                Err(StoreError::Integrity(_))
            ));
            assert_eq!(account_before, store.read_account("user-a").unwrap());
            assert_eq!(atomic_before, opaque_restore_atomic_state(&store));
            assert_eq!(
                media_before,
                store.read_media("user-a", "guarded-file").unwrap()
            );
            assert_eq!(0, complete());
        } else {
            assert!(store
                .read_media("user-a", "guarded-file")
                .unwrap()
                .is_none());
            media_privacy_upload(&store, "guarded-file", 302);
            assert_eq!(1, complete());
            let restored = store.restore_account_snapshot("user-a", 0, 0, 400).unwrap();
            assert_eq!(1, restored.revision);
            assert_eq!(
                b"guarded-file",
                store
                    .read_media("user-a", "guarded-file")
                    .unwrap()
                    .unwrap()
                    .content
                    .as_slice()
            );
        }
        assert_eq!(
            other_before,
            store.read_media("user-b", "guarded-file").unwrap()
        );
        store.validate_integrity().unwrap();
    }
}

#[test]
fn legacy_ids_only_declaration_never_proves_complete_opaque_history() {
    for shared in [false, true] {
        let (_directory, store, sealed) = media_privacy_legacy_pair(shared);
        let historical = opaque_restore_retained_history(&store, &sealed);
        let (unlocked, token) =
            crate::unlock_desktop_note_json(&sealed, "synthetic-only-password").unwrap();
        let strong = crate::note_crypto::session_media_references_json(&unlocked, &token).unwrap();
        crate::close_desktop_note_session(&token);
        let mut legacy: serde_json::Value = serde_json::from_str(&strong).unwrap();
        legacy["formatVersion"] = serde_json::json!(1);
        legacy.as_object_mut().unwrap().remove("contentSha256ById");
        legacy["bindingSha256"] = serde_json::json!(sha256_hex(
            &serde_json::to_vec(&(
                "sealed-note-media-declaration-v1",
                legacy["envelopeSha256"].as_str().unwrap(),
                &legacy["attachmentIds"]
            ))
            .unwrap()
        ));
        let note: serde_json::Value = serde_json::from_str(&sealed).unwrap();
        let before = store.read_account("user-a").unwrap();
        assert!(store
            .register_sealed_media_references(
                "user-a",
                note["id"].as_str().unwrap(),
                before.revision,
                0,
                &legacy.to_string()
            )
            .unwrap());
        assert!(
            matches!(
                store.restore_account_snapshot("user-a", historical.revision, before.revision, 500),
                Err(StoreError::Integrity(_))
            ),
            "an IDs-only declaration, even an empty legacy one, was treated as complete"
        );
        assert_eq!(before, store.read_account("user-a").unwrap());
        media_privacy_resolve_legacy(&store, &sealed);
        store
            .restore_account_snapshot("user-a", historical.revision, before.revision, 501)
            .unwrap();
    }
}

#[test]
fn rolled_back_declaration_cannot_publish_complete_historical_media() {
    let (_directory, store, sealed) = media_privacy_legacy_pair(true);
    let historical = opaque_restore_retained_history(&store, &sealed);
    let (unlocked, token) =
        crate::unlock_desktop_note_json(&sealed, "synthetic-only-password").unwrap();
    let declaration = crate::note_crypto::session_media_references_json(&unlocked, &token).unwrap();
    crate::close_desktop_note_session(&token);
    let sealed: serde_json::Value = serde_json::from_str(&sealed).unwrap();
    let mut connection = store.open_connection(false).unwrap();
    let before_policy = note_privacy::read_policy(&connection, "user-a").unwrap();
    let before_state = opaque_restore_atomic_state(&store);
    {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let policy = before_policy
            .clone()
            .including_sealed_media(&sealed, serde_json::from_str(&declaration).unwrap())
            .unwrap();
        note_privacy::record_reference_metadata(&transaction, "user-a", policy).unwrap();
        assert_eq!(1,transaction.query_row("SELECT media_snapshot_complete FROM account_snapshot_history WHERE user_id='user-a' AND revision=?1",[historical.revision],|row|row.get::<_,i64>(0)).unwrap());
        // Aborting the authenticated request must discard both its declaration
        // and the independent media manifest captured in this transaction.
    }
    assert_eq!(
        before_policy,
        note_privacy::read_policy(&connection, "user-a").unwrap()
    );
    assert_eq!(0,connection.query_row("SELECT media_snapshot_complete FROM account_snapshot_history WHERE user_id='user-a' AND revision=?1",[historical.revision],|row|row.get::<_,i64>(0)).unwrap());
    assert_eq!(before_state, opaque_restore_atomic_state(&store));
    assert_eq!(
        b"guarded-file",
        store
            .read_media("user-a", "guarded-file")
            .unwrap()
            .unwrap()
            .content
            .as_slice()
    );
}
