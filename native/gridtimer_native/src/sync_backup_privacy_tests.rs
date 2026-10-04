// v0.0.4 - Clean actual 15-to-16 recovery copies and preserve format-14 archive fixtures.
// v0.0.3 - Verify privacy in imported JSON sources and protected migration containers.
// v0.0.2 - Verify case aliases and real startup, scheduled and notification entry points.
// v0.0.1 - Exercise physical backup cleanup, rollback compatibility and publication failures.
fn backup_privacy_fixture() -> (TestSqliteStore, String, PathBuf, PathBuf) {
    let mut plain = app_data::upsert_note_app_data_json(
        &app_data::default_app_data_json(100),
        r#"{"id":"backup-private","content":"OLD_BACKUP_PRIVATE_78301"}"#,
        100,
    )
    .unwrap();
    plain = app_data::upsert_note_app_data_json(
        &plain,
        r#"{"id":"ordinary","content":"KEEP_ORDINARY_92713"}"#,
        110,
    )
    .unwrap();
    let fixture = account_test_store("backup-cleanup-token", plain.clone());
    let startup =
        ensure_startup_backup(&fixture.store, fixture.store.database_path(), 150, true).unwrap();
    let runtime = fixture.directory.join("recovery");
    perform_runtime_backup(&fixture.store, &runtime, 160).unwrap();
    // The lock-contention tests must not depend on the periodic call under
    // test having run correctly; establish the idle maintenance lock directly.
    backup_privacy::clean(&fixture.store, &runtime).unwrap();
    (fixture, plain, startup, runtime)
}

#[test]
fn storage_format_backup_maintenance_cleans_actual_pre_upgrade_copy() {
    let (fixture, plain, _, runtime) = backup_privacy_fixture();
    rusqlite::Connection::open(fixture.store.database_path())
        .unwrap()
        .execute_batch("DELETE FROM schema_migrations WHERE version>=16; PRAGMA user_version=15;")
        .unwrap();
    let opened =
        SqliteServerStore::open_with_options(crate::server_store::ServerStoreOpenOptions {
            database_path: fixture.store.database_path().into(),
            legacy_json_path: None,
            now_epoch_millis: 175,
            legacy_token_ttl_millis: crate::server_store::DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
        })
        .unwrap();
    let copy = opened.pre_schema_migration_backup.unwrap().destination;
    assert!(copy
        .file_name()
        .unwrap()
        .to_string_lossy()
        .contains("_pre_schema_v15_to_v17_"));
    assert!(backup_privacy_has_marker(&copy, "OLD_BACKUP_PRIVATE_78301"));
    backup_privacy_delete(&fixture, &plain);
    backup_privacy::clean(&fixture.store, &runtime).unwrap();
    assert!(
        !backup_privacy_has_marker(&copy, "OLD_BACKUP_PRIVATE_78301"),
        "the real pre-upgrade copy escaped privacy maintenance"
    );
    assert!(backup_privacy_has_marker(&copy, "KEEP_ORDINARY_92713"));
    SqliteServerStore::verify_existing_backup(&copy, 300).unwrap();
    assert_eq!(
        17,
        rusqlite::Connection::open(&copy)
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap()
    );
}

#[test]
fn startup_clean_keeps_unchanged_schema_16_rollback_hash() {
    let (mut fixture, plain, _, runtime) = backup_privacy_fixture();
    backup_privacy_delete(&fixture, &plain);
    fixture.store.finish_note_privacy_cleanup().unwrap();
    let database = fixture.store.database_path().to_path_buf();
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch(
            "DROP TABLE legal_report_tombstones;
             DROP TABLE legal_report_uploads;
             DROP TABLE legal_reports;
             DELETE FROM schema_migrations WHERE version=17;
             PRAGMA user_version=16;",
        )
        .unwrap();
    let opened =
        SqliteServerStore::open_with_options(crate::server_store::ServerStoreOpenOptions {
            database_path: database,
            legacy_json_path: None,
            now_epoch_millis: 250,
            legacy_token_ttl_millis: crate::server_store::DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
        })
        .unwrap();
    let before = opened.pre_schema_migration_backup.unwrap();
    assert_eq!(
        16,
        rusqlite::Connection::open(&before.destination)
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap()
    );
    fixture.store = opened.store;
    assert!(fixture.store.backup_privacy_generation().unwrap().is_some());
    backup_privacy::clean(&fixture.store, &runtime).unwrap();
    let after = SqliteServerStore::verify_existing_backup(&before.destination, 250).unwrap();
    assert_eq!(before.sha256, after.sha256);
    assert_eq!(before.size_bytes, after.size_bytes);
    assert_eq!(
        fs::metadata(&before.destination).unwrap().len(),
        before.size_bytes
    );
    assert_eq!(
        16,
        rusqlite::Connection::open(&before.destination)
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap()
    );
}

#[test]
fn stronger_privacy_policy_scrubs_schema_16_rollback_without_upgrading_it() {
    let (mut fixture, plain, _, runtime) = backup_privacy_fixture();
    let database = fixture.store.database_path().to_path_buf();
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch(
            "DROP TABLE legal_report_tombstones;
             DROP TABLE legal_report_uploads;
             DROP TABLE legal_reports;
             DELETE FROM schema_migrations WHERE version=17;
             PRAGMA user_version=16;",
        )
        .unwrap();
    let opened =
        SqliteServerStore::open_with_options(crate::server_store::ServerStoreOpenOptions {
            database_path: database,
            legacy_json_path: None,
            now_epoch_millis: 175,
            legacy_token_ttl_millis: crate::server_store::DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
        })
        .unwrap();
    let before = opened.pre_schema_migration_backup.unwrap();
    assert!(backup_privacy_has_marker(
        &before.destination,
        "OLD_BACKUP_PRIVATE_78301"
    ));
    fixture.store = opened.store;
    backup_privacy_delete(&fixture, &plain);
    assert!(backup_privacy::clean(&fixture.store, &runtime).unwrap() >= 1);
    let after = SqliteServerStore::verify_existing_backup(&before.destination, 175).unwrap();
    assert_ne!(before.sha256, after.sha256);
    assert!(!backup_privacy_has_marker(
        &before.destination,
        "OLD_BACKUP_PRIVATE_78301"
    ));
    assert_eq!(
        16,
        rusqlite::Connection::open(&before.destination)
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap()
    );
    let rollback_check = fixture.directory.join("rollback_validation.sqlite3");
    fs::copy(&before.destination, &rollback_check).unwrap();
    let restored = SqliteServerStore::open(&rollback_check, None).unwrap();
    assert!(!restored
        .read_account("user-1")
        .unwrap()
        .app_data_json
        .contains("OLD_BACKUP_PRIVATE_78301"));
}

#[test]
fn backup_privacy_legacy_import_files_do_not_retain_private_notes() {
    for seal in [false, true] {
        let (fixture, plain, runtime, source, container, legacy) = legacy_privacy_fixture();
        let desired = if seal {
            let note = serde_json::from_str::<Value>(&plain).unwrap()["notes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|note| note["id"] == "backup-private")
                .unwrap()
                .to_string();
            let (sealed, session) =
                crate::encrypt_desktop_note_json(&note, "legacy-test-password").unwrap();
            crate::close_desktop_note_session(&session);
            app_data::upsert_note_app_data_json(&plain, &sealed, 200).unwrap()
        } else {
            app_data::delete_note_permanently_app_data_json(&plain, "backup-private", 200).unwrap()
        };
        fixture
            .store
            .compare_and_swap_account("user-1", 0, &desired, 200)
            .unwrap();
        backup_privacy::clean(&fixture.store, &runtime).unwrap();
        let source_bytes = fs::read(&source).unwrap();
        let container_bytes =
            crate::server_store::legacy_backup_test_plaintext(&container).unwrap();
        let retains = |bytes: &[u8]| {
            bytes
                .windows(b"OLD_BACKUP_PRIVATE_78301".len())
                .any(|part| part == b"OLD_BACKUP_PRIVATE_78301")
        };
        assert!(
            !retains(&source_bytes) && !retains(&container_bytes),
            "legacy source or encrypted migration container retained private note content"
        );
        for bytes in [&source_bytes, &container_bytes] {
            let after: Value = serde_json::from_slice(bytes).unwrap();
            assert_eq!(legacy["retainedExtension"], after["retainedExtension"]);
            assert_eq!(legacy["users"][1], after["users"][1]);
            assert!(after["users"][0]["appDataJson"]
                .as_str()
                .unwrap()
                .contains("KEEP_ORDINARY_92713"));
            for key in [
                "id",
                "email",
                "passwordSalt",
                "passwordHash",
                "createdAtEpochMillis",
                "updatedAtEpochMillis",
                "tokens",
            ] {
                assert_eq!(
                    legacy["users"][0][key], after["users"][0][key],
                    "legacy identity or credentials changed: {key}"
                );
            }
        }
        let before = fixture.store.read_account("user-1").unwrap();
        assert!(fixture
            .store
            .import_legacy_files_with_merge(&[source], 300, TOKEN_TTL_MILLIS, |_, _, _| panic!(
                "sanitized source was imported again"
            ))
            .unwrap()
            .files
            .is_empty());
        assert_eq!(before, fixture.store.read_account("user-1").unwrap());
    }
}

fn backup_privacy_delete(fixture: &TestSqliteStore, plain: &str) -> String {
    let deleted =
        app_data::delete_note_permanently_app_data_json(plain, "backup-private", 200).unwrap();
    fixture
        .store
        .compare_and_swap_account("user-1", 0, &deleted, 200)
        .unwrap();
    deleted
}

fn backup_privacy_has_marker(path: &Path, marker: &str) -> bool {
    fs::read(path)
        .unwrap()
        .windows(marker.len())
        .any(|bytes| bytes == marker.as_bytes())
}

fn backup_privacy_interrupt<T>(point: u8, action: impl FnOnce() -> T) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            backup_privacy::INTERRUPT.with(|value| value.set(None));
        }
    }
    backup_privacy::INTERRUPT.with(|value| value.set(Some(point)));
    let _reset = Reset;
    action()
}

#[test]
fn backup_privacy_cleans_owned_files_and_keeps_old_schema_and_ordinary_data() {
    for seal in [false, true] {
        let (fixture, plain, startup, runtime) = backup_privacy_fixture();
        let migration = fixture
            .directory
            .join("server_store_pre_schema_v14_to_v15_140.sqlite3");
        fixture
            .store
            .create_verified_backup(&migration, 140)
            .unwrap();
        let connection = rusqlite::Connection::open(&migration).unwrap();
        connection.execute_batch("DROP TABLE note_privacy_commit_witnesses; DROP TABLE account_note_privacy; DELETE FROM schema_migrations WHERE version>=15; PRAGMA user_version=14;").unwrap();
        drop(connection);
        // Old startup manifests predate explicit identity fields.
        let (mut state, _) = load_valid_startup_backup_state(fixture.store.database_path())
            .unwrap()
            .unwrap();
        state.state_version = 0;
        state.server_instance_id.clear();
        state.target_store_fingerprint.clear();
        write_startup_backup_state(fixture.store.database_path(), &state).unwrap();
        if seal {
            let data: Value = serde_json::from_str(&plain).unwrap();
            let note = data["notes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|note| note["id"] == "backup-private")
                .unwrap();
            let (encrypted, session) =
                crate::encrypt_desktop_note_json(&note.to_string(), "backup-test-password")
                    .unwrap();
            crate::close_desktop_note_session(&session);
            let sealed = app_data::upsert_note_app_data_json(&plain, &encrypted, 200).unwrap();
            fixture
                .store
                .compare_and_swap_account("user-1", 0, &sealed, 200)
                .unwrap();
        } else {
            backup_privacy_delete(&fixture, &plain);
        }
        assert!(backup_privacy_has_marker(
            &startup,
            "OLD_BACKUP_PRIVATE_78301"
        ));
        assert_eq!(3, backup_privacy::clean(&fixture.store, &runtime).unwrap());
        let (_, runtime_report) =
            latest_verified_runtime_backup(&runtime, fixture.store.database_path(), None).unwrap();
        let runtime_backup = runtime_report.unwrap().0.destination;
        for path in [&startup, &migration, &runtime_backup] {
            assert!(
                !backup_privacy_has_marker(path, "OLD_BACKUP_PRIVATE_78301"),
                "managed backup retained private plaintext"
            );
            assert!(backup_privacy_has_marker(path, "KEEP_ORDINARY_92713"));
            SqliteServerStore::verify_existing_backup(path, 300).unwrap();
        }
        let connection = rusqlite::Connection::open_with_flags(
            &migration,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        assert_eq!(
            14,
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap()
        );
        assert!(
            load_valid_startup_backup_state(fixture.store.database_path())
                .unwrap()
                .is_some()
        );
        assert_eq!(
            0,
            backup_privacy::clean(&fixture.store, &runtime).unwrap(),
            "unchanged privacy rewrote the archives again"
        );
    }
}

#[test]
fn backup_privacy_interrupted_publication_recovers_even_without_main_database() {
    for point in 0..=3 {
        let (fixture, plain, _startup, runtime) = backup_privacy_fixture();
        backup_privacy_delete(&fixture, &plain);
        assert!(backup_privacy_interrupt(point, || backup_privacy::clean(
            &fixture.store,
            &runtime
        ))
        .is_err());
        let database = fixture.store.database_path().to_path_buf();
        fixture.store.finish_note_privacy_cleanup().unwrap();
        fs::remove_file(&database).unwrap();
        assert!(
            recover_missing_sqlite_store_if_needed(&database, &runtime, 400)
                .unwrap()
                .is_some()
        );
        assert!(!backup_privacy_has_marker(
            &database,
            "OLD_BACKUP_PRIVATE_78301"
        ));
        let restored = SqliteServerStore::open(&database, None).unwrap();
        backup_privacy::clean(&restored, &runtime).unwrap();
        assert!(load_valid_startup_backup_state(&database)
            .unwrap()
            .is_some());
        let (_, report) = latest_verified_runtime_backup(&runtime, &database, None).unwrap();
        assert!(!backup_privacy_has_marker(
            &report.unwrap().0.destination,
            "OLD_BACKUP_PRIVATE_78301"
        ));
    }
}

#[test]
fn backup_privacy_runtime_manifest_gap_is_repaired_before_backup_selection() {
    for point in [1, 2, 3] {
        let (fixture, plain, startup, runtime) = backup_privacy_fixture();
        fs::remove_file(startup).unwrap();
        fs::remove_file(startup_backup_state_path(fixture.store.database_path())).unwrap();
        backup_privacy_delete(&fixture, &plain);
        assert!(backup_privacy_interrupt(point, || backup_privacy::clean(
            &fixture.store,
            &runtime
        ))
        .is_err());
        let database = fixture.store.database_path().to_path_buf();
        fixture.store.finish_note_privacy_cleanup().unwrap();
        fs::remove_file(&database).unwrap();
        recover_missing_sqlite_store_if_needed(&database, &runtime, 400)
            .unwrap()
            .unwrap();
        assert!(!backup_privacy_has_marker(
            &database,
            "OLD_BACKUP_PRIVATE_78301"
        ));
        assert!(latest_verified_runtime_backup(&runtime, &database, None)
            .unwrap()
            .1
            .is_some());
    }
}

#[test]
fn backup_privacy_preserves_foreign_and_unbound_archives() {
    let (fixture, plain, _startup, runtime) = backup_privacy_fixture();
    let other = account_test_store("other-archive-token", plain.clone());
    let foreign = other
        .store
        .create_verified_backup(
            fixture.directory.join("server_store_startup_120.sqlite3"),
            120,
        )
        .unwrap();
    let unbound = fixture
        .store
        .create_verified_backup(
            unique_runtime_backup_path(&runtime, &fixture.store.server_instance_id().unwrap(), 130)
                .unwrap(),
            130,
        )
        .unwrap();
    backup_privacy_delete(&fixture, &plain);
    backup_privacy::clean(&fixture.store, &runtime).unwrap();
    for report in [&foreign, &unbound] {
        assert_eq!(
            report.sha256,
            SqliteServerStore::verify_existing_backup(
                &report.destination,
                report.created_at_epoch_millis
            )
            .unwrap()
            .sha256
        );
        assert!(backup_privacy_has_marker(
            &report.destination,
            "OLD_BACKUP_PRIVATE_78301"
        ));
    }
}

#[test]
fn backup_privacy_new_fence_invalidates_completed_cleanup_certificate() {
    let (fixture, plain, startup, runtime) = backup_privacy_fixture();
    let deleted = backup_privacy_delete(&fixture, &plain);
    backup_privacy::clean(&fixture.store, &runtime).unwrap();
    let certificate = fs::read_dir(&fixture.directory)
        .unwrap()
        .map(Result::unwrap)
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .ends_with(".privacy_verified_v1.json")
        })
        .unwrap()
        .path();
    fs::write(&certificate, b"damaged cache, not data authority").unwrap();
    assert_eq!(1, backup_privacy::clean(&fixture.store, &runtime).unwrap());
    let second =
        app_data::delete_note_permanently_app_data_json(&deleted, "ordinary", 300).unwrap();
    fixture
        .store
        .compare_and_swap_account("user-1", 1, &second, 300)
        .unwrap();
    assert!(backup_privacy::clean(&fixture.store, &runtime).unwrap() > 0);
    assert!(!backup_privacy_has_marker(&startup, "KEEP_ORDINARY_92713"));
    assert_eq!(0, backup_privacy::clean(&fixture.store, &runtime).unwrap());
    fs::remove_file(&startup).unwrap();
    backup_privacy::clean(&fixture.store, &runtime).unwrap();
    assert!(
        !fs::read_dir(&fixture.directory)
            .unwrap()
            .map(Result::unwrap)
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .ends_with(".privacy_verified_v1.json")),
        "expired backup left an unbounded cleanup certificate"
    );
}

#[test]
fn backup_privacy_preserves_external_replacement_during_interrupted_cleanup() {
    let (fixture, plain, startup, runtime) = backup_privacy_fixture();
    let lock_path = fs::read_dir(&fixture.directory)
        .unwrap()
        .map(Result::unwrap)
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .ends_with(".privacy_maintenance.lock")
        })
        .unwrap()
        .path();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(lock_path)
        .unwrap();
    lock.lock().unwrap();
    assert_eq!(
        io::ErrorKind::WouldBlock,
        backup_privacy::clean(&fixture.store, &runtime)
            .unwrap_err()
            .kind()
    );
    drop(lock);
    backup_privacy_delete(&fixture, &plain);
    assert!(
        backup_privacy_interrupt(1, || backup_privacy::clean(&fixture.store, &runtime)).is_err()
    );
    let other = account_test_store("replacement-token", plain);
    let foreign = other
        .store
        .create_verified_backup(fixture.directory.join("foreign-copy.sqlite3"), 170)
        .unwrap();
    fs::copy(&foreign.destination, &startup).unwrap();
    assert!(backup_privacy::resume(fixture.store.database_path(), &runtime).is_err());
    assert_eq!(
        foreign.sha256,
        SqliteServerStore::verify_existing_backup(&startup, 170)
            .unwrap()
            .sha256
    );
}

#[test]
fn backup_privacy_rejects_escaping_pending_paths() {
    let (fixture, plain, startup, runtime) = backup_privacy_fixture();
    backup_privacy_delete(&fixture, &plain);
    assert!(
        backup_privacy_interrupt(1, || backup_privacy::clean(&fixture.store, &runtime)).is_err()
    );
    let marker = fs::read_dir(&fixture.directory)
        .unwrap()
        .map(Result::unwrap)
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .ends_with(".privacy_pending_v1.json")
        })
        .unwrap()
        .path();
    let mut pending: Value = serde_json::from_slice(&fs::read(&marker).unwrap()).unwrap();
    let victim = fixture.directory.join("preserve.txt");
    fs::write(&victim, b"preserve this unrelated file").unwrap();
    pending["temporary"] = json!("../preserve.txt");
    fs::write(&marker, serde_json::to_vec(&pending).unwrap()).unwrap();
    assert!(backup_privacy::resume(fixture.store.database_path(), &runtime).is_err());
    assert_eq!(
        b"preserve this unrelated file",
        fs::read(&victim).unwrap().as_slice()
    );
    assert!(backup_privacy_has_marker(
        &startup,
        "OLD_BACKUP_PRIVATE_78301"
    ));
}

#[cfg(windows)]
#[test]
fn backup_privacy_windows_case_alias_shares_maintenance_lock_and_archives() {
    let (fixture, plain, startup, runtime) = backup_privacy_fixture();
    let migration = fixture
        .directory
        .join("server_store_pre_schema_v14_to_v15_140.sqlite3");
    fixture
        .store
        .create_verified_backup(&migration, 140)
        .unwrap();
    let connection = rusqlite::Connection::open(&migration).unwrap();
    connection.execute_batch("DROP TABLE note_privacy_commit_witnesses; DROP TABLE account_note_privacy; DELETE FROM schema_migrations WHERE version>=15; PRAGMA user_version=14;").unwrap();
    drop(connection);
    backup_privacy_delete(&fixture, &plain);
    let alias = SqliteServerStore::open(
        fixture
            .store
            .database_path()
            .with_file_name("SERVER_STORE.SQLITE3"),
        None,
    )
    .unwrap();
    let lock_path = fs::read_dir(&fixture.directory)
        .unwrap()
        .map(Result::unwrap)
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .ends_with(".privacy_maintenance.lock")
        })
        .unwrap()
        .path();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(lock_path)
        .unwrap();
    lock.lock().unwrap();
    let blocked = backup_privacy::clean(&alias, &runtime);
    drop(lock);
    assert_eq!(
        io::ErrorKind::WouldBlock,
        blocked
            .expect_err("case alias bypassed maintenance lock")
            .kind()
    );
    backup_privacy::clean(&alias, &runtime).unwrap();
    for path in [&startup, &migration] {
        assert!(!backup_privacy_has_marker(path, "OLD_BACKUP_PRIVATE_78301"));
        assert!(backup_privacy_has_marker(path, "KEEP_ORDINARY_92713"));
    }
}

#[test]
fn backup_privacy_startup_entry_cleans_existing_backups_before_accepting_requests() {
    let (fixture, plain, _, runtime) = backup_privacy_fixture();
    let startup = ensure_startup_backup(
        &fixture.store,
        fixture.store.database_path(),
        now_millis(),
        true,
    )
    .unwrap();
    backup_privacy_delete(&fixture, &plain);
    let (opened, _, _) = initialize_sqlite_store_with_runtime_recovery_directory(
        fixture.store.database_path(),
        Some(&runtime),
    )
    .unwrap();
    assert!(
        !backup_privacy_has_marker(&startup, "OLD_BACKUP_PRIVATE_78301"),
        "startup entry omitted backup privacy cleanup"
    );
    assert!(backup_privacy_has_marker(&startup, "KEEP_ORDINARY_92713"));
    opened.validate_integrity().unwrap();
}

#[test]
fn backup_privacy_scheduled_entry_cleans_old_archives_and_preserves_backup_status() {
    let (fixture, plain, startup, runtime) = backup_privacy_fixture();
    backup_privacy_delete(&fixture, &plain);
    let info = Mutex::new(ServerRuntimeInfo::default());
    perform_scheduled_runtime_backup(&fixture.store, &runtime, &info);
    assert_eq!("ok", info.lock().unwrap().backup_status);
    assert!(
        !backup_privacy_has_marker(&startup, "OLD_BACKUP_PRIVATE_78301"),
        "scheduled entry omitted backup privacy cleanup"
    );
    assert!(backup_privacy_has_marker(&startup, "KEEP_ORDINARY_92713"));
}

#[test]
fn backup_privacy_committed_change_wakes_real_worker_and_recovers_from_lock_failure() {
    let (fixture, plain, startup, runtime) = backup_privacy_fixture();
    let info = Arc::new(Mutex::new(ServerRuntimeInfo::default()));
    let lock_path = fs::read_dir(&fixture.directory)
        .unwrap()
        .map(Result::unwrap)
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .ends_with(".privacy_maintenance.lock")
        })
        .unwrap()
        .path();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(lock_path)
        .unwrap();
    lock.lock().unwrap();
    let worker = backup_worker::start_with_timing(
        Arc::new(fixture.store.clone()),
        runtime.clone(),
        Arc::clone(&info),
        Duration::from_secs(3600),
        Duration::from_millis(10),
        Duration::from_millis(50),
    )
    .unwrap();
    backup_privacy_delete(&fixture, &plain);
    let deadline = Instant::now() + Duration::from_secs(10);
    while info.lock().unwrap().backup_privacy.status != "error" && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        "error",
        info.lock().unwrap().backup_privacy.status,
        "committed privacy change did not wake worker"
    );
    assert!(backup_privacy_has_marker(
        &startup,
        "OLD_BACKUP_PRIVATE_78301"
    ));
    drop(lock);
    while backup_worker::visible_status(&info.lock().unwrap()).status != "ok"
        && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        "ok",
        info.lock().unwrap().backup_privacy.status,
        "worker did not retry cleanup"
    );
    assert!(!backup_privacy_has_marker(
        &startup,
        "OLD_BACKUP_PRIVATE_78301"
    ));
    assert!(backup_privacy_has_marker(&startup, "KEEP_ORDINARY_92713"));
    let state = info.lock().unwrap();
    assert!(state.backup_privacy.last_failure_at_epoch_millis > 0);
    assert!(state.backup_privacy.last_success_at_epoch_millis > 0);
    assert_eq!(
        0, state.last_backup_success_at_epoch_millis,
        "privacy event unnecessarily created a full backup"
    );
    drop(state);
    let health = backup_privacy_read_health(&fixture.store, Arc::clone(&info));
    assert!(health.ok);
    assert_eq!(
        "ok", health.backup_privacy.status,
        "health endpoint omitted privacy maintenance status"
    );
    assert!(health.backup_privacy.last_failure_at_epoch_millis > 0);
    assert!(health.backup_privacy.last_success_at_epoch_millis > 0);
    drop(worker);
}

fn backup_privacy_read_health(
    store: &SqliteServerStore,
    info: Arc<Mutex<ServerRuntimeInfo>>,
) -> SyncClientResult {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let store = Arc::new(store.clone());
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        handle_connection(
            stream,
            store,
            info,
            Arc::new(Mutex::new(LoginRateLimiter::default())),
        )
        .unwrap();
    });
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(3)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    server.join().unwrap();
    let start = find_header_end(&response).unwrap() + 4;
    serde_json::from_slice(&response[start..]).unwrap()
}

#[test]
fn backup_privacy_health_does_not_acknowledge_new_requests_with_older_cleanup() {
    use crate::server_store::BackupPrivacyWake;
    let (fixture, plain, _, runtime) = backup_privacy_fixture();
    let subscription = fixture.store.subscribe_backup_privacy().unwrap();
    let info = Arc::new(Mutex::new(ServerRuntimeInfo {
        backup_privacy: BackupPrivacyStatus {
            status: "ok".into(),
            ..BackupPrivacyStatus::default()
        },
        backup_privacy_wake: Some(subscription.clone()),
        ..ServerRuntimeInfo::default()
    }));
    let deleted = backup_privacy_delete(&fixture, &plain);
    assert_eq!(
        "pending",
        backup_privacy_read_health(&fixture.store, Arc::clone(&info))
            .backup_privacy
            .status,
        "health reported completion before cleanup"
    );
    assert_eq!(
        BackupPrivacyWake::Changed,
        subscription.wait(Duration::ZERO)
    );
    let first_request = subscription.requested();
    backup_privacy::clean(&fixture.store, &runtime).unwrap();
    let second_deleted =
        app_data::delete_note_permanently_app_data_json(&deleted, "ordinary", 300).unwrap();
    fixture
        .store
        .compare_and_swap_account("user-1", 1, &second_deleted, 300)
        .unwrap();
    subscription.complete(first_request);
    assert_eq!(
        "pending",
        backup_privacy_read_health(&fixture.store, Arc::clone(&info))
            .backup_privacy
            .status,
        "old cleanup acknowledged a newer privacy request"
    );
    let second_request = subscription.requested();
    backup_privacy::clean(&fixture.store, &runtime).unwrap();
    subscription.complete(second_request);
    assert_eq!(
        "ok",
        backup_privacy_read_health(&fixture.store, info)
            .backup_privacy
            .status
    );
}

#[test]
fn native14_managed_backup_preserves_typed_conflict_bytes_and_recovery_provenance() {
    let attachment = |id: &str, bytes: &[u8]| {
        json!({
            "id":id,"sha256":sha256_hex_for_test(bytes),"mimeType":"application/octet-stream",
            "sizeBytes":bytes.len(),"createdAtEpochMillis":100,"updatedAtEpochMillis":100
        })
    };
    let mut plain: Value = serde_json::from_str(&app_data::default_app_data_json(100)).unwrap();
    plain["notes"] = json!([{"id":"backup-private","content":"NATIVE14_PRIVATE_NOTE_64938",
        "createdAtEpochMillis":100,"updatedAtEpochMillis":100,
        "attachments":[attachment("private-file",b"private-owner-bytes")]}]);
    plain["syncConflictHistory"] = json!([{"id":"retained-conflict","entityType":"note",
        "entityId":"retained-note","losingRevisionEpochMillis":100,"capturedAtEpochMillis":110,
        "payload":{"id":"retained-note","content":"NATIVE14_RETAINED_CONFLICT_37491",
            "createdAtEpochMillis":100,"updatedAtEpochMillis":100,
            "attachments":[attachment("conflict-file",b"retained-conflict-bytes")]}}]);
    let plain = plain.to_string();
    let mut other: Value = serde_json::from_str(&app_data::default_app_data_json(100)).unwrap();
    other["notes"] = json!([{"id":"other-owner","content":"other account",
        "createdAtEpochMillis":100,"updatedAtEpochMillis":100,
        "attachments":[attachment("private-file",b"other-owner-bytes")]}]);
    let other = other.to_string();
    let directory = std::env::temp_dir().join(format!(
        "gridtimer_sync_core_test_native14_{}",
        random_token(12)
    ));
    let database = directory.join("server_store.sqlite3");
    let original = SqliteServerStore::preschema_test_create_native14(
        &database,
        &[("user-1", &plain), ("user-2", &other)],
        &[
            ("user-1", "private-file", b"private-owner-bytes"),
            ("user-1", "conflict-file", b"retained-conflict-bytes"),
            ("user-2", "private-file", b"other-owner-bytes"),
        ],
    )
    .unwrap();
    SqliteServerStore::verify_existing_backup(&database, 100).unwrap();
    let opened =
        SqliteServerStore::open_with_options(crate::server_store::ServerStoreOpenOptions {
            database_path: database.clone(),
            legacy_json_path: None,
            now_epoch_millis: 150,
            legacy_token_ttl_millis: crate::server_store::DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
        })
        .unwrap();
    let migration_backup = opened.pre_schema_migration_backup.unwrap();
    let archive = migration_backup.destination;
    let fixture = TestSqliteStore {
        store: opened.store,
        directory,
    };
    let before = SqliteServerStore::verify_existing_backup(&archive, 150).unwrap();
    assert_eq!(migration_backup.sha256, before.sha256);
    assert_eq!(original.server_instance_id, before.server_instance_id);
    let schema_shape = |path: &Path| {
        let connection = rusqlite::Connection::open(path).unwrap();
        assert_eq!(
            14,
            connection
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap()
        );
        let mut statement = connection.prepare("SELECT type,name,sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name").unwrap();
        let rows = statement
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        rows
    };
    let original_shape = schema_shape(&archive);
    backup_privacy_delete(&fixture, &plain);
    let runtime = fixture.directory.join("recovery");
    assert!(backup_privacy::clean(&fixture.store, &runtime).unwrap() >= 1);
    let after = SqliteServerStore::verify_existing_backup(&archive, 150).unwrap();
    SqliteServerStore::verify_managed_privacy_backup(&archive, 150).unwrap();
    assert_eq!(original_shape, schema_shape(&archive));
    assert_ne!(before.sha256, after.sha256);
    assert!(!backup_privacy_has_marker(
        &archive,
        "NATIVE14_PRIVATE_NOTE_64938"
    ));
    let connection = rusqlite::Connection::open(&archive).unwrap();
    let kept: String = connection
        .query_row(
            "SELECT app_data_json FROM account_snapshots WHERE user_id='user-1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(kept.contains("NATIVE14_RETAINED_CONFLICT_37491"));
    assert_eq!(0,connection.query_row("SELECT COUNT(*) FROM note_media WHERE user_id='user-1' AND attachment_id='private-file'",[],|r|r.get::<_,i64>(0)).unwrap());
    let bytes: Vec<u8> = connection.query_row("SELECT content FROM note_media WHERE user_id='user-1' AND attachment_id='conflict-file'",[],|r|r.get(0)).unwrap();
    assert_eq!(b"retained-conflict-bytes", bytes.as_slice());
    assert_eq!(
        other,
        connection
            .query_row(
                "SELECT app_data_json FROM account_snapshots WHERE user_id='user-2'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap()
    );
    assert_eq!(b"other-owner-bytes",connection.query_row("SELECT content FROM note_media WHERE user_id='user-2' AND attachment_id='private-file'",[],|r|r.get::<_,Vec<u8>>(0)).unwrap().as_slice());
    drop(connection);
    let signed: Value =
        serde_json::from_slice(&fs::read(preschema_proof(&fixture.directory)).unwrap()).unwrap();
    assert_eq!(1, signed["proof"]["version"]);
    assert_eq!(14, signed["proof"]["schema"]);
    assert_eq!(before.sha256, signed["proof"]["origin"]["sha256"]);
    assert_eq!(after.sha256, signed["proof"]["current"]["sha256"]);
    assert_eq!(0, backup_privacy::clean(&fixture.store, &runtime).unwrap());
    // Reopen the published bytes through the genuine recovery/migration path.
    let recovered_path = fixture
        .directory
        .join("recovered")
        .join("server_store.sqlite3");
    fs::create_dir_all(recovered_path.parent().unwrap()).unwrap();
    fs::copy(&archive, &recovered_path).unwrap();
    SqliteServerStore::apply_external_privacy_to_recovery_copy(&recovered_path, &database, 400)
        .unwrap();
    let recovered = SqliteServerStore::open(&recovered_path, None).unwrap();
    assert_eq!(
        kept,
        recovered.read_account("user-1").unwrap().app_data_json
    );
    recovered
        .compare_and_swap_account("user-1", 0, &app_data::default_app_data_json(450), 450)
        .unwrap();
    recovered
        .delete_media("user-1", "conflict-file", 500, 500)
        .unwrap();
    assert_eq!(
        1,
        recovered
            .prune_deleted_media_content(
                501 + crate::server_store::DELETED_MEDIA_CONTENT_RETENTION_MILLIS
            )
            .unwrap()
    );
    let restored = recovered
        .restore_account_snapshot("user-1", 0, 1, 600)
        .unwrap();
    assert_eq!(kept, restored.app_data_json);
    assert_eq!(
        b"retained-conflict-bytes",
        recovered
            .read_media("user-1", "conflict-file")
            .unwrap()
            .unwrap()
            .content
            .as_slice()
    );
    recovered.validate_integrity().unwrap();
}
