// v0.0.2 - Keep opaque legacy timer fields while enforcing note privacy through startup.
#[test]
fn backup_privacy_legacy_non_note_fields_survive_startup() {
    for seal in [false, true] {
        let (fixture, plain, runtime, _, _, mut legacy) = legacy_privacy_fixture();
        let mut data: Value = serde_json::from_str(&plain).unwrap();
        data["schemaVersion"] = json!(10);
        data["slots"] = json!({"oldTimerRows": [1, 2, 3]});
        data["retainedForeignDomain"] = json!({"revision": 37, "opaque": [true, "unchanged"]});
        let old_data = serde_json::to_string(&data).unwrap();
        assert_eq!(
            app_data::app_data_json_compatibility(&old_data, 0),
            app_data::AppDataJsonCompatibility::Invalid
        );
        legacy["users"][0]["appDataJson"] = json!(old_data);
        let source = fixture.directory.join("server_store_opaque_previous.json");
        fs::write(&source, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let imported = fixture
            .store
            .import_legacy_files_with_merge(
                &[source.clone()],
                170,
                TOKEN_TTL_MILLIS,
                |current, _, _| Ok(current.into()),
            )
            .unwrap();
        let container = imported.files[0].backup_path.clone();
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
        let expected_account = fixture.store.read_account("user-1").unwrap();
        let expected_tokens = legacy_privacy_token_count(&fixture.store);
        let (opened, _, _) = initialize_sqlite_store_with_runtime_recovery_directory(
            fixture.store.database_path(),
            Some(&runtime),
        )
        .unwrap();
        legacy_privacy_assert_files(&source, &container, &legacy, true);
        for raw in [
            fs::read(&source).unwrap(),
            crate::server_store::legacy_backup_test_plaintext(&container).unwrap(),
        ] {
            let value: Value = serde_json::from_slice(&raw).unwrap();
            let mut after: Value =
                serde_json::from_str(value["users"][0]["appDataJson"].as_str().unwrap()).unwrap();
            let mut before = data.clone();
            for key in ["notes", "tombstones", "syncConflictHistory"] {
                before.as_object_mut().unwrap().remove(key);
                after.as_object_mut().unwrap().remove(key);
            }
            assert_eq!(
                before, after,
                "privacy cleanup rewrote another legacy domain"
            );
        }
        assert_eq!(opened.read_account("user-1").unwrap(), expected_account);
        assert_eq!(legacy_privacy_token_count(&opened), expected_tokens);
        let saved_source = fs::read(&source).unwrap();
        let saved_container = fs::read(&container).unwrap();
        backup_privacy::clean(&opened, &runtime).unwrap();
        assert_eq!(fs::read(&source).unwrap(), saved_source);
        assert_eq!(fs::read(&container).unwrap(), saved_container);
        opened.validate_integrity().unwrap();
    }
}

fn legacy_privacy_fixture() -> (TestSqliteStore, String, PathBuf, PathBuf, PathBuf, Value) {
    let (fixture, plain, _, runtime) = backup_privacy_fixture();
    let other = app_data::upsert_note_app_data_json(
        &app_data::default_app_data_json(100),
        r#"{"id":"backup-private","content":"OTHER_ACCOUNT_75682"}"#,
        100,
    )
    .unwrap();
    fixture
        .store
        .create_user(NewStoredUser {
            id: "user-2".into(),
            email: "other@example.com".into(),
            password_salt: "other-salt".into(),
            password_hash: password_hash("other-salt", "other-password"),
            password_scheme: "legacy_sha256".into(),
            created_at_epoch_millis: 1,
            updated_at_epoch_millis: 1,
            app_data_json: other,
            account_revision: 0,
        })
        .unwrap();
    let source = fixture.directory.join("server_store_previous.json");
    let mut legacy: Value =
        serde_json::from_str(&fixture.store.export_compatible_legacy_json().unwrap()).unwrap();
    // Exports intentionally omit bearer tokens; an actual old import can contain them.
    assert_eq!(legacy["users"][0]["id"], "user-1");
    legacy["users"][0]["tokens"] = json!([{
        "token":"synthetic-legacy-token-78310", "deviceName":"legacy-device",
        "createdAtEpochMillis":100, "lastSeenAtEpochMillis":100,
        "retainedTokenExtension":"TOKEN_EXTENSION_34812"
    }]);
    legacy["retainedExtension"] = json!({"value":"PRESERVE_EXTENSION_83712"});
    legacy["users"][0]["retainedUserExtension"] = json!({"value":17});
    fs::write(&source, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let import = fixture
        .store
        .import_legacy_files_with_merge(
            &[source.clone()],
            150,
            TOKEN_TTL_MILLIS,
            |current, _, _| Ok(current.into()),
        )
        .unwrap();
    assert_eq!(import.files.len(), 1);
    let container = import.files[0].backup_path.clone();
    (fixture, plain, runtime, source, container, legacy)
}

fn legacy_privacy_interrupt<T>(point: u8, action: impl FnOnce() -> T) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            crate::server_store::LEGACY_PRIVACY_INTERRUPT.with(|value| value.set(None));
        }
    }
    crate::server_store::LEGACY_PRIVACY_INTERRUPT.with(|value| value.set(Some(point)));
    let _reset = Reset;
    action()
}

fn legacy_privacy_paths(directory: &Path, prefix: &str, suffix: &str) -> Vec<PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(Result::unwrap)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with(prefix) && name.ends_with(suffix)
        })
        .map(|entry| entry.path())
        .collect();
    paths.sort();
    paths
}

fn legacy_privacy_assert_files(source: &Path, container: &Path, legacy: &Value, ordinary: bool) {
    for raw in [
        fs::read(source).unwrap(),
        crate::server_store::legacy_backup_test_plaintext(container).unwrap(),
    ] {
        let value: Value = serde_json::from_slice(&raw).unwrap();
        let data = value["users"][0]["appDataJson"].as_str().unwrap();
        assert!(
            !data.contains("OLD_BACKUP_PRIVATE_78301"),
            "legacy file retained private content"
        );
        assert_eq!(data.contains("KEEP_ORDINARY_92713"), ordinary);
        assert_eq!(
            value["users"][1], legacy["users"][1],
            "another account was modified"
        );
        let mut expected_user = legacy["users"][0].clone();
        expected_user["appDataJson"] = value["users"][0]["appDataJson"].clone();
        assert_eq!(
            value["users"][0], expected_user,
            "credentials or extensions were changed"
        );
        assert_eq!(value["retainedExtension"], legacy["retainedExtension"]);
    }
}

#[test]
fn backup_privacy_legacy_interruption_recovers_through_startup_without_duplicate_imports() {
    for point in 0..=3 {
        let (fixture, plain, runtime, source, container, legacy) = legacy_privacy_fixture();
        backup_privacy_delete(&fixture, &plain);
        assert!(legacy_privacy_interrupt(point, || backup_privacy::clean(
            &fixture.store,
            &runtime
        ))
        .is_err());
        if point == 1 {
            let stages = legacy_privacy_paths(&fixture.directory, ".legacy_stage_", ".tmp");
            assert_eq!(stages.len(), 1);
            fs::write(&stages[0], b"short").unwrap();
        }
        let account = fixture.store.read_account("user-1").unwrap();
        let token_count = legacy_privacy_token_count(&fixture.store);
        let (opened, _, _) = initialize_sqlite_store_with_runtime_recovery_directory(
            fixture.store.database_path(),
            Some(&runtime),
        )
        .unwrap();
        legacy_privacy_assert_files(&source, &container, &legacy, true);
        assert_eq!(account, opened.read_account("user-1").unwrap());
        assert_eq!(token_count, legacy_privacy_token_count(&opened));
        assert_eq!(
            legacy_privacy_paths(
                &fixture.directory,
                "server_store_previous_pre_sqlite_migration_",
                ".gtlbak"
            )
            .len(),
            1
        );
        assert!(legacy_privacy_paths(&fixture.directory, ".legacy_stage_", ".tmp").is_empty());
        opened.validate_integrity().unwrap();
    }
}

fn legacy_privacy_token_count(store: &SqliteServerStore) -> i64 {
    let connection = rusqlite::Connection::open(store.database_path()).unwrap();
    connection
        .query_row("SELECT COUNT(*) FROM tokens", [], |row| row.get(0))
        .unwrap()
}

#[test]
fn backup_privacy_legacy_restores_consumed_receipts_before_startup_and_applies_new_policy() {
    let (fixture, plain, runtime, source, container, legacy) = legacy_privacy_fixture();
    let database = fixture.store.database_path().to_path_buf();
    let original = fixture.directory.join("rollback_original.sqlite3");
    let backup = fixture
        .store
        .create_verified_backup(&original, 160)
        .unwrap();
    backup_privacy_delete(&fixture, &plain);
    backup_privacy::clean(&fixture.store, &runtime).unwrap();
    fixture.store.finish_note_privacy_cleanup().unwrap();
    fs::remove_file(&database).unwrap();
    restore_verified_backup_to_missing_database(
        &original,
        &database,
        160,
        &backup.sha256,
        backup.size_bytes,
        300,
    )
    .unwrap();
    let restored = SqliteServerStore::open(&database, None).unwrap();
    let account = restored.read_account("user-1").unwrap();
    let token_count = legacy_privacy_token_count(&restored);
    let (opened, _, _) =
        initialize_sqlite_store_with_runtime_recovery_directory(&database, Some(&runtime)).unwrap();
    assert_eq!(
        account,
        opened.read_account("user-1").unwrap(),
        "sanitized legacy source was reimported after rollback"
    );
    assert_eq!(token_count, legacy_privacy_token_count(&opened));
    assert_eq!(
        legacy_privacy_paths(
            &fixture.directory,
            "server_store_previous_pre_sqlite_migration_",
            ".gtlbak"
        )
        .len(),
        1
    );
    let next =
        app_data::delete_note_permanently_app_data_json(&account.app_data_json, "ordinary", 400)
            .unwrap();
    opened
        .compare_and_swap_account("user-1", account.revision, &next, 400)
        .unwrap();
    backup_privacy::clean(&opened, &runtime).unwrap();
    legacy_privacy_assert_files(&source, &container, &legacy, false);
    opened.validate_integrity().unwrap();
}

#[test]
fn backup_privacy_legacy_tampered_proof_and_changed_source_are_preserved() {
    let (fixture, plain, runtime, source, container, legacy) = legacy_privacy_fixture();
    backup_privacy_delete(&fixture, &plain);
    assert!(
        legacy_privacy_interrupt(1, || backup_privacy::clean(&fixture.store, &runtime)).is_err()
    );
    let proofs = legacy_privacy_paths(&fixture.directory, ".legacy_privacy_", ".json");
    assert_eq!(proofs.len(), 1);
    let original_proof = fs::read(&proofs[0]).unwrap();
    let mut proof: Value = serde_json::from_slice(&original_proof).unwrap();
    let target = fixture
        .directory
        .join(proof["proof"]["file"].as_str().unwrap());
    let original_target = fs::read(&target).unwrap();
    let stage = fixture
        .directory
        .join(proof["proof"]["pending_stage"].as_str().unwrap());
    let original_stage = fs::read(&stage).unwrap();
    proof["mac"] = Value::String("0".repeat(64));
    fs::write(&proofs[0], serde_json::to_vec(&proof).unwrap()).unwrap();
    assert!(backup_privacy::clean(&fixture.store, &runtime).is_err());
    assert_eq!(fs::read(&target).unwrap(), original_target);
    assert_eq!(fs::read(&stage).unwrap(), original_stage);
    fs::write(&proofs[0], &original_proof).unwrap();
    fs::write(&target, b"EXTERNAL_REPLACEMENT_MUST_REMAIN_91820").unwrap();
    assert!(backup_privacy::clean(&fixture.store, &runtime).is_err());
    assert_eq!(
        fs::read(&target).unwrap(),
        b"EXTERNAL_REPLACEMENT_MUST_REMAIN_91820"
    );
    assert_eq!(fs::read(&stage).unwrap(), original_stage);
    fs::write(&target, &original_target).unwrap();
    backup_privacy::clean(&fixture.store, &runtime).unwrap();
    legacy_privacy_assert_files(&source, &container, &legacy, true);
    assert!(!stage.exists());
}

#[test]
fn backup_privacy_legacy_does_not_adopt_changed_or_unregistered_files() {
    let (fixture, plain, runtime, source, container, legacy) = legacy_privacy_fixture();
    let unknown = fixture.directory.join("unregistered_legacy.json");
    let unknown_bytes = serde_json::to_vec(&legacy).unwrap();
    fs::write(&unknown, &unknown_bytes).unwrap();
    let mut external = legacy.clone();
    external["externalOwnership"] = json!(true);
    let external_bytes = serde_json::to_vec(&external).unwrap();
    fs::write(&source, &external_bytes).unwrap();
    backup_privacy_delete(&fixture, &plain);
    backup_privacy::clean(&fixture.store, &runtime).unwrap();
    assert_eq!(fs::read(&source).unwrap(), external_bytes);
    assert_eq!(fs::read(&unknown).unwrap(), unknown_bytes);
    let container = crate::server_store::legacy_backup_test_plaintext(&container).unwrap();
    assert!(!String::from_utf8(container)
        .unwrap()
        .contains("OLD_BACKUP_PRIVATE_78301"));
}

#[test]
fn backup_privacy_legacy_repeat_cleanup_keeps_bytes_and_modification_times() {
    let (fixture, plain, runtime, source, container, legacy) = legacy_privacy_fixture();
    backup_privacy_delete(&fixture, &plain);
    backup_privacy::clean(&fixture.store, &runtime).unwrap();
    let mut paths = legacy_privacy_paths(&fixture.directory, ".legacy_privacy_", ".json");
    assert_eq!(paths.len(), 2);
    paths.extend([source.clone(), container.clone()]);
    let before: Vec<_> = paths
        .iter()
        .map(|path| {
            (
                fs::read(path).unwrap(),
                fs::metadata(path).unwrap().modified().unwrap(),
            )
        })
        .collect();
    assert_eq!(backup_privacy::clean(&fixture.store, &runtime).unwrap(), 0);
    for (path, (bytes, modified)) in paths.iter().zip(before) {
        assert_eq!(fs::read(path).unwrap(), bytes);
        assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), modified);
    }
    legacy_privacy_assert_files(&source, &container, &legacy, true);
}
