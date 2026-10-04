// v0.0.2 - Cover legacy metadata-only upgrades, deduplication and atomic rollback.
// v0.0.1 - Assert privacy transactions preserve recovery, ownership and failures.
fn privacy_test_plain() -> String {
    app_data::upsert_note_app_data_json(&app_data::default_app_data_json(100),
        r#"{"id":"private","title":"Private","content":"PRIVACY_STORE_MARKER_9215","updatedAtEpochMillis":100}"#, 100).unwrap()
}

#[test]
fn privacy_metadata_upgrade_advances_once_without_duplicating_snapshots() {
    for deduplicated_save in [false, true] {
        let (dir, store) = temp_store("privacy_metadata_upgrade");
        let raw = privacy_test_plain();
        store.record("owner", &raw, 100, "local_save").unwrap();
        let deleted =
            app_data::delete_note_permanently_app_data_json(&raw, "private", 200).unwrap();
        let head = store
            .record_with_sync_state("owner", &deleted, b"bound-account", 200, "local_save")
            .unwrap();
        let before = store.journal_evidence().unwrap();
        let connection = store.open_connection(false).unwrap();
        connection
            .execute_batch("DROP TABLE desktop_state_privacy_barriers; DROP TABLE desktop_state_mirror_provenance; PRAGMA user_version=2;")
            .unwrap();
        drop(connection);
        let upgraded = DesktopStateStore::open(store.database_path()).unwrap();
        if deduplicated_save {
            assert_eq!(
                head,
                upgraded
                    .record_with_sync_state("owner", &deleted, b"bound-account", 300, "local_save")
                    .unwrap()
            );
        } else {
            upgraded.repair_privacy_history("owner").unwrap();
        }
        let after = upgraded.journal_evidence().unwrap();
        assert_eq!(before.journal_id, after.journal_id);
        assert_eq!(before.commit_sequence + 1, after.commit_sequence);
        assert_eq!(head, upgraded.latest_valid("owner", 300).unwrap().unwrap());
        assert!(!upgraded.privacy_policy("owner").unwrap().0.is_empty());
        let connection = upgraded.open_connection(false).unwrap();
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM desktop_state_snapshots", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(2, count);
        upgraded.repair_privacy_history("owner").unwrap();
        assert_eq!(after, upgraded.journal_evidence().unwrap());
        let next = app_data::upsert_note_app_data_json(
            &deleted,
            r#"{"id":"next","title":"Next","content":"safe","updatedAtEpochMillis":400}"#,
            400,
        )
        .unwrap();
        let snapshot = upgraded
            .record_with_sync_state("owner", &next, b"bound-account", 400, "local_save")
            .unwrap();
        assert_eq!(after.commit_sequence + 1, snapshot.id);
        assert_eq!(
            snapshot.id,
            upgraded.journal_evidence().unwrap().commit_sequence
        );
        drop(connection);
        drop(upgraded);
        drop(store);
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn privacy_metadata_upgrade_failure_rolls_back_sequence_and_barrier() {
    for deduplicated_save in [false, true] {
        let (dir, store) = temp_store("privacy_metadata_rollback");
        let deleted =
            app_data::delete_note_permanently_app_data_json(&privacy_test_plain(), "private", 200)
                .unwrap();
        let head = store.record("owner", &deleted, 200, "local_save").unwrap();
        let before = store.journal_evidence().unwrap();
        let connection = store.open_connection(false).unwrap();
        connection
            .execute_batch(
                "DELETE FROM desktop_state_privacy_barriers;
            CREATE TRIGGER reject_metadata_commit BEFORE UPDATE ON desktop_state_journal_metadata
            BEGIN SELECT RAISE(ABORT, 'injected metadata commit failure'); END;",
            )
            .unwrap();
        if deduplicated_save {
            assert!(store.record("owner", &deleted, 300, "local_save").is_err());
        } else {
            assert!(store.repair_privacy_history("owner").is_err());
        }
        assert_eq!(before, store.journal_evidence().unwrap());
        assert_eq!(head, store.latest_valid("owner", 300).unwrap().unwrap());
        assert!(store.privacy_policy("owner").unwrap().0.is_empty());
        connection
            .execute_batch("DROP TRIGGER reject_metadata_commit;")
            .unwrap();
        store.repair_privacy_history("owner").unwrap();
        assert_eq!(
            before.commit_sequence + 1,
            store.journal_evidence().unwrap().commit_sequence
        );
        drop(connection);
        drop(store);
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn privacy_transaction_failure_rolls_back_history_head_and_barrier() {
    let (dir, store) = temp_store("privacy_atomic_failure");
    let raw = privacy_test_plain();
    let before = store.record("owner", &raw, 100, "local_save").unwrap();
    let evidence = store.journal_evidence().unwrap();
    let deleted = app_data::delete_note_permanently_app_data_json(&raw, "private", 200).unwrap();
    let connection = store.open_connection(false).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_privacy_test BEFORE INSERT ON desktop_state_privacy_barriers
        BEGIN SELECT RAISE(ABORT, 'injected privacy write failure'); END;",
        )
        .unwrap();
    assert!(store.record("owner", &deleted, 200, "local_save").is_err());
    assert_eq!(before, store.latest_valid("owner", 300).unwrap().unwrap());
    assert_eq!(evidence, store.journal_evidence().unwrap());
    assert!(store.privacy_policy("owner").unwrap().0.is_empty());
    drop(connection);
    drop(store);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn privacy_refuses_unknown_quarantine_without_destroying_evidence() {
    let (dir, store) = temp_store("privacy_unknown_evidence");
    let raw = privacy_test_plain();
    store.record("owner", &raw, 100, "local_save").unwrap();
    let second = store
        .record_with_sync_state(
            "owner",
            &raw,
            b"another metadata generation",
            110,
            "local_save",
        )
        .unwrap();
    let connection = store.open_connection(false).unwrap();
    connection
        .execute(
            "UPDATE desktop_state_snapshots SET schema_version=?1 WHERE id=?2",
            params![i64::from(app_data::APP_DATA_SCHEMA_VERSION) + 1, second.id],
        )
        .unwrap();
    store.latest_valid("owner", 120).unwrap();
    let deleted = app_data::delete_note_permanently_app_data_json(&raw, "private", 200).unwrap();
    assert!(store.record("owner", &deleted, 200, "local_save").is_err());
    let retained: String = connection
        .query_row(
            "SELECT app_data_json FROM desktop_state_snapshot_quarantine WHERE snapshot_id=?1",
            params![second.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(raw, retained);
    assert!(store.privacy_policy("owner").unwrap().0.is_empty());
    assert_eq!(
        raw,
        store
            .latest_valid("owner", 300)
            .unwrap()
            .unwrap()
            .app_data_json
    );
    drop(connection);
    drop(store);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn privacy_upgrade_from_v2_keeps_journal_identity_and_snapshots() {
    let (dir, store) = temp_store("privacy_v2_upgrade");
    let raw = privacy_test_plain();
    let snapshot = store.record("owner", &raw, 100, "local_save").unwrap();
    let evidence = store.journal_evidence().unwrap();
    let connection = store.open_connection(false).unwrap();
    connection
        .execute_batch("DROP TABLE desktop_state_privacy_barriers; DROP TABLE desktop_state_mirror_provenance; PRAGMA user_version=2;")
        .unwrap();
    drop(connection);
    let upgraded = DesktopStateStore::open(store.database_path()).unwrap();
    assert_eq!(evidence, upgraded.journal_evidence().unwrap());
    assert_eq!(
        snapshot,
        upgraded.latest_valid("owner", 200).unwrap().unwrap()
    );
    assert!(upgraded.privacy_policy("owner").unwrap().0.is_empty());
    let connection = upgraded.open_connection(false).unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(
        STORE_SCHEMA_VERSION, version,
        "older writers must not reopen privacy-aware storage"
    );
    drop(connection);
    drop(upgraded);
    drop(store);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn privacy_deletion_keeps_other_account_and_metadata_exact() {
    let (dir, store) = temp_store("privacy_owner_isolation");
    let raw = privacy_test_plain();
    let first = store
        .record_with_sync_state("owner-a", &raw, b"owner-a-state", 100, "local_save")
        .unwrap();
    let other = store
        .record_with_sync_state("owner-b", &raw, b"owner-b-state", 100, "local_save")
        .unwrap();
    let deleted = app_data::delete_note_permanently_app_data_json(&raw, "private", 200).unwrap();
    store
        .record_with_sync_state("owner-a", &deleted, b"owner-a-state", 200, "local_save")
        .unwrap();
    let connection = store.open_connection(false).unwrap();
    let redacted = read_snapshot_by_id(&connection, first.id).unwrap().unwrap();
    assert!(!redacted.app_data_json.contains("PRIVACY_STORE_MARKER_9215"));
    assert_eq!(first.protected_sync_state, redacted.protected_sync_state);
    verify_snapshot(&redacted, Some("owner-a"), 200).unwrap();
    assert_eq!(other, store.latest_valid("owner-b", 200).unwrap().unwrap());
    assert!(store
        .record("owner-a", &raw, 300, "explicit_restore")
        .is_err());
    drop(connection);
    drop(store);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn privacy_waits_for_media_pin_and_keeps_completed_source_spent() {
    let (dir, store) = temp_store("privacy_media_pin");
    let raw = privacy_test_plain();
    let source = format!("scope_media_tx:{}", "f".repeat(64));
    let snapshot = store.record("owner", &raw, 100, &source).unwrap();
    let deleted = app_data::delete_note_permanently_app_data_json(&raw, "private", 200).unwrap();
    assert!(store.record("owner", &deleted, 200, "local_save").is_err());
    assert_eq!(snapshot, store.latest_valid("owner", 200).unwrap().unwrap());
    assert!(store.privacy_policy("owner").unwrap().0.is_empty());
    store.complete_scope_media_transaction(&source).unwrap();
    store.record("owner", &deleted, 200, "local_save").unwrap();
    let reopened = DesktopStateStore::open(store.database_path()).unwrap();
    assert!(reopened.record("owner", &raw, 300, &source).is_err());
    assert!(reopened.record("owner", &deleted, 300, &source).is_err());
    reopened.complete_scope_media_transaction(&source).unwrap();
    let connection = reopened.open_connection(false).unwrap();
    let retained = read_snapshot_by_id(&connection, snapshot.id)
        .unwrap()
        .unwrap();
    assert!(!retained.app_data_json.contains("PRIVACY_STORE_MARKER_9215"));
    verify_snapshot(&retained, Some("owner"), 300).unwrap();
    drop(connection);
    drop(reopened);
    drop(store);
    fs::remove_dir_all(dir).unwrap();
}
