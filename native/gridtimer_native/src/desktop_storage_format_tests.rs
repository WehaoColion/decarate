// v0.0.1 - Keep recovery records intact and roll back a failed format transition.
#[test]
fn storage_format_desktop_upgrade_preserves_snapshots_policy_and_sync_state() {
    let (dir, store) = temp_store("format_v3_preservation");
    let raw = privacy_test_plain();
    store
        .record_with_sync_state("owner", &raw, b"old-sync-state", 100, "local_save")
        .unwrap();
    let deleted = app_data::delete_note_permanently_app_data_json(&raw, "private", 200).unwrap();
    let before = store
        .record_with_sync_state("owner", &deleted, b"current-sync-state", 200, "local_save")
        .unwrap();
    let evidence = store.journal_evidence().unwrap();
    let policy = store.privacy_policy("owner").unwrap();
    let connection = store.open_connection(false).unwrap();
    connection
        .execute_batch("DROP TABLE desktop_state_mirror_provenance")
        .unwrap();
    connection.pragma_update(None, "user_version", 3).unwrap();
    let retained: i64 = connection
        .query_row("SELECT COUNT(*) FROM desktop_state_snapshots", [], |row| {
            row.get(0)
        })
        .unwrap();
    drop(connection);
    let upgraded = DesktopStateStore::open(store.database_path()).unwrap();
    assert_eq!(
        before,
        upgraded.latest_valid("owner", 300).unwrap().unwrap()
    );
    assert_eq!(evidence, upgraded.journal_evidence().unwrap());
    assert_eq!(policy, upgraded.privacy_policy("owner").unwrap());
    let connection = upgraded.open_connection(false).unwrap();
    assert_eq!(
        retained,
        connection
            .query_row("SELECT COUNT(*) FROM desktop_state_snapshots", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap()
    );
    assert!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap()
            > 3,
        "published readers must reject the upgraded store before touching snapshots"
    );
    drop(connection);
    drop(upgraded);
    drop(store);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn storage_format_desktop_validation_failure_rolls_back_marker_and_keeps_evidence() {
    let (dir, store) = temp_store("format_v3_interruption");
    store
        .record("owner", &privacy_test_plain(), 100, "local_save")
        .unwrap();
    let connection = store.open_connection(false).unwrap();
    let binding: String = connection
        .query_row(
            "SELECT registry_sha256 FROM desktop_state_owners WHERE owner='owner'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    connection
        .execute(
            "UPDATE desktop_state_owners SET registry_sha256=?1 WHERE owner='owner'",
            params!["0".repeat(64)],
        )
        .unwrap();
    connection
        .execute_batch("DROP TABLE desktop_state_mirror_provenance")
        .unwrap();
    connection.pragma_update(None, "user_version", 3).unwrap();
    assert!(DesktopStateStore::open(store.database_path()).is_err());
    assert_eq!(
        3,
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        "failed validation must not publish the new format marker"
    );
    assert_eq!(
        0,
        connection
            .query_row(
                "SELECT COUNT(*) FROM desktop_state_snapshot_quarantine",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap()
    );
    connection
        .execute(
            "UPDATE desktop_state_owners SET registry_sha256=?1 WHERE owner='owner'",
            params![binding],
        )
        .unwrap();
    drop(connection);
    let reopened = DesktopStateStore::open(store.database_path()).unwrap();
    assert!(reopened.latest_valid("owner", 300).unwrap().is_some());
    drop(reopened);
    drop(store);
    fs::remove_dir_all(dir).unwrap();
}
