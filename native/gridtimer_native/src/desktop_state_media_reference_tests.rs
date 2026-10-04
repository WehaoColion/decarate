// v0.0.1 - Verify retained-reference authority, account isolation and its write reservation.
#[test]
fn private_recovery_mirror_requires_verified_owner_and_exact_bytes() {
    use crate::desktop_private_media_index::DesktopPrivateMediaReferences;
    let (directory, store) = temp_store("private_mirror_proof");
    let (_, sealed, _) = private_media_core_fixture(&store, "owner-a");
    let current = app_data::default_app_data_json(300);
    store.record("owner-a", &sealed, 200, "migration").unwrap();
    store
        .record("owner-a", &current, 300, "local_save")
        .unwrap();
    store.record("owner-b", &current, 300, "migration").unwrap();

    let policy = DesktopPrivacyPolicy::default();
    let mut verified = DesktopPrivateMediaReferences::from_snapshot(&current).unwrap();
    assert!(store
        .include_verified_private_media_recovery_snapshot("owner-a", &sealed, &mut verified)
        .unwrap());
    assert_eq!(verified.queries(&policy, false).unwrap().len(), 1);
    assert_eq!(verified.unverified_recovery_sources(), 0);

    for (owner, raw) in [
        ("owner-b", sealed.clone()),
        ("owner-a", format!("{sealed} ")),
        ("owner-a", "{".into()),
    ] {
        let mut unverified = DesktopPrivateMediaReferences::from_snapshot(&current).unwrap();
        assert!(!store
            .include_verified_private_media_recovery_snapshot(owner, &raw, &mut unverified)
            .unwrap());
        assert!(unverified.queries(&policy, false).unwrap().is_empty());
        assert_eq!(unverified.unverified_recovery_sources(), 1);
        assert!(!unverified.scope(&policy).is_complete());
    }
    drop(store);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn private_recovery_mirror_without_retained_proof_stays_pending() {
    use crate::desktop_private_media_index::DesktopPrivateMediaReferences;
    let (directory, store) = temp_store("private_mirror_pruned_proof");
    let (_, sealed, _) = private_media_core_fixture(&store, "owner-a");
    let current = app_data::default_app_data_json(300);
    store.record("owner-a", &sealed, 200, "migration").unwrap();
    store
        .record("owner-a", &current, 300, "local_save")
        .unwrap();
    let connection = Connection::open(store.database_path()).unwrap();
    connection
        .execute(
            "DELETE FROM desktop_state_snapshots WHERE owner=?1 AND raw_sha256=?2",
            params!["owner-a", sha256_hex(sealed.as_bytes())],
        )
        .unwrap();
    let mut view = DesktopPrivateMediaReferences::from_snapshot(&current).unwrap();
    assert!(!store
        .include_verified_private_media_recovery_snapshot("owner-a", &sealed, &mut view)
        .unwrap());
    assert!(view
        .queries(&DesktopPrivacyPolicy::default(), false)
        .unwrap()
        .is_empty());
    assert_eq!(view.unverified_recovery_sources(), 1);
    drop(connection);
    drop(store);
    fs::remove_dir_all(directory).unwrap();
}

fn media_retention_test_state() -> String {
    app_data::upsert_note_app_data_json(
        &app_data::default_app_data_json(100),
        &json!({"id":"retention-note","title":"Recovery","content":"",
            "attachments":[{"id":"retained-file","mimeType":"image/png",
                "sha256":"a".repeat(64),"sizeBytes":1,"updatedAtEpochMillis":100}],
            "updatedAtEpochMillis":100})
        .to_string(),
        100,
    )
    .unwrap()
}

#[test]
fn local_recovery_media_guard_holds_writes_until_cleanup_finishes() {
    let (directory, store) = temp_store("media_retention_lock");
    let old = media_retention_test_state();
    let current = app_data::default_app_data_json(200);
    store.record("owner-a", &old, 100, "migration").unwrap();
    store
        .record("owner-a", &current, 200, "verified_download")
        .unwrap();
    let guard = DesktopStateStore::lock_retained_media_references(
        store.database_path(),
        "owner-a",
        &current,
    )
    .unwrap();
    assert!(guard.is_complete());
    assert!(guard.ids().contains("retained-file"));
    let writer = Connection::open(store.database_path()).unwrap();
    writer.busy_timeout(Duration::from_millis(20)).unwrap();
    let attempt = writer.execute_batch("BEGIN IMMEDIATE");
    assert!(
        matches!(attempt, Err(rusqlite::Error::SqliteFailure(code,_))
        if matches!(code.code, rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)),
        "media cleanup must reserve the journal against a new reference commit"
    );
    drop(guard);
    writer.execute_batch("BEGIN IMMEDIATE; ROLLBACK;").unwrap();
    drop(writer);
    drop(store);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn local_recovery_media_guard_keeps_damaged_owner_unknown_without_reading_other_accounts() {
    let (directory, store) = temp_store("media_retention_owner");
    let current = app_data::default_app_data_json(200);
    store.record("owner-a", &current, 100, "migration").unwrap();
    store
        .record("owner-b", &media_retention_test_state(), 100, "migration")
        .unwrap();
    let writer = Connection::open(store.database_path()).unwrap();
    writer
        .execute(
            "UPDATE desktop_state_snapshots SET app_data_json='{}' WHERE owner='owner-b'",
            [],
        )
        .unwrap();
    let first = DesktopStateStore::lock_retained_media_references(
        store.database_path(),
        "owner-a",
        &current,
    )
    .unwrap();
    assert!(first.is_complete());
    assert!(first.ids().is_empty());
    drop(first);
    let second = DesktopStateStore::lock_retained_media_references(
        store.database_path(),
        "owner-b",
        &current,
    )
    .unwrap();
    assert!(
        !second.is_complete(),
        "unverified recovery content must never authorize cleanup"
    );
    drop(second);
    writer
        .execute(
            "DELETE FROM desktop_state_snapshots WHERE owner='owner-b'",
            [],
        )
        .unwrap();
    let lost = DesktopStateStore::lock_retained_media_references(
        store.database_path(),
        "owner-b",
        &current,
    )
    .unwrap();
    assert!(
        !lost.is_complete(),
        "an initialized owner with lost history cannot authorize cleanup"
    );
    drop(lost);
    let quarantined: i64 = writer
        .query_row(
            "SELECT count(*) FROM desktop_state_snapshot_quarantine",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        quarantined, 0,
        "retention observation must not rewrite damaged history"
    );
    drop(writer);
    drop(store);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn local_recovery_media_guard_applies_permanent_deletion_to_mirrors_and_rejects_unknown_data() {
    let (directory, store) = temp_store("media_retention_privacy");
    let old = media_retention_test_state();
    store.record("owner-a", &old, 100, "migration").unwrap();
    let deleted =
        app_data::delete_note_permanently_app_data_json(&old, "retention-note", 300).unwrap();
    store
        .record("owner-a", &deleted, 300, "desktop_edit")
        .unwrap();
    let mut guard = DesktopStateStore::lock_retained_media_references(
        store.database_path(),
        "owner-a",
        &deleted,
    )
    .unwrap();
    guard.include_recovery_snapshot(&old);
    assert!(guard.is_complete());
    assert!(!guard.ids().contains("retained-file"));
    guard.include_recovery_snapshot("{");
    assert!(!guard.is_complete());
    drop(guard);
    drop(store);
    fs::remove_dir_all(directory).unwrap();
}
