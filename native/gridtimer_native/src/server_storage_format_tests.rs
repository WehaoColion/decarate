// v0.0.1 - Preserve old recovery provenance and resume errors or process loss during sidecar migration.
fn storage_format_make_legacy_journal(store: &SqliteServerStore) {
    privacy_journal::create_native_v1_fixture(store).unwrap();
}

fn storage_format_native15_fixture(
    label: &str,
) -> (TestDirectory, SqliteServerStore, String, String) {
    let directory = TestDirectory::new(label);
    let path = directory.0.join("server_store.sqlite3");
    let initial = crate::app_data::default_app_data_json(100);
    SqliteServerStore::preschema_test_create_native_layout(&path, 15, &[("user-a", &initial)], &[])
        .unwrap();
    let store = SqliteServerStore {
        database_path: path,
    };
    let plain = server_privacy_audit_plain();
    store
        .compare_and_swap_account("user-a", 0, &plain, 100)
        .unwrap();
    let deleted =
        crate::app_data::delete_note_permanently_app_data_json(&plain, "private-note", 200)
            .unwrap();
    (directory, store, plain, deleted)
}

#[test]
fn storage_format_server_upgrade_keeps_verified_pre_schema_backup_and_account_identity() {
    let (_directory, store, _, deleted) = storage_format_native15_fixture("format-v15-backup");
    store
        .compare_and_swap_account("user-a", 1, &deleted, 200)
        .unwrap();
    let identity = store.server_account_identity("user-a").unwrap();
    let generation = store.backup_privacy_generation().unwrap();
    let history: Vec<_> = store
        .list_snapshot_history("user-a", 100)
        .unwrap()
        .into_iter()
        .map(|row| row.app_data_json)
        .collect();
    storage_format_make_legacy_journal(&store);
    let opened = SqliteServerStore::open_with_options(ServerStoreOpenOptions {
        database_path: store.database_path().into(),
        legacy_json_path: None,
        now_epoch_millis: 300,
        legacy_token_ttl_millis: DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
    })
    .unwrap();
    let backup = opened.pre_schema_migration_backup.unwrap();
    let report = SqliteServerStore::verify_existing_backup(&backup.destination, 300).unwrap();
    assert_eq!(backup.sha256, report.sha256);
    assert_eq!(identity.server_instance_id, report.server_instance_id);
    let old_copy = Connection::open(&backup.destination).unwrap();
    assert_eq!(15, current_schema_version(&old_copy).unwrap());
    assert_eq!(
        deleted,
        old_copy
            .query_row(
                "SELECT app_data_json FROM account_snapshots WHERE user_id='user-a'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap()
    );
    assert_eq!(
        identity.account_namespace,
        opened
            .store
            .server_account_identity("user-a")
            .unwrap()
            .account_namespace
    );
    assert_eq!(
        deleted,
        opened.store.read_account("user-a").unwrap().app_data_json
    );
    assert_eq!(
        history,
        opened
            .store
            .list_snapshot_history("user-a", 100)
            .unwrap()
            .into_iter()
            .map(|row| row.app_data_json)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        generation,
        opened.store.backup_privacy_generation().unwrap()
    );
    let journal = Connection::open(privacy_journal::journal_path(store.database_path())).unwrap();
    assert_eq!(
        2,
        journal
            .query_row(
                "SELECT format_version FROM privacy_journal_identity",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap()
    );
    opened.store.validate_integrity().unwrap();
}

#[test]
fn storage_format_server_retries_interrupted_journal_upgrade_without_losing_fences() {
    let (_directory, store, _, deleted) =
        storage_format_native15_fixture("format-v15-interruption");
    store
        .compare_and_swap_account("user-a", 1, &deleted, 200)
        .unwrap();
    let identity = store.server_instance_id().unwrap();
    let fences = privacy_journal::recovery_policies(store.database_path(), &identity).unwrap();
    storage_format_make_legacy_journal(&store);
    privacy_journal::FORMAT_MIGRATION_INTERRUPTION.with(|point| point.set(true));
    let result = SqliteServerStore::open(store.database_path(), None);
    privacy_journal::FORMAT_MIGRATION_INTERRUPTION.with(|point| point.set(false));
    assert!(
        result.is_err(),
        "injected format transition unexpectedly committed"
    );
    let journal = Connection::open(privacy_journal::journal_path(store.database_path())).unwrap();
    assert_eq!(
        1,
        journal
            .query_row(
                "SELECT format_version FROM privacy_journal_identity",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap()
    );
    drop(journal);
    assert_eq!(
        fences,
        privacy_journal::recovery_policies(store.database_path(), &identity).unwrap()
    );
    let reopened = SqliteServerStore::open(store.database_path(), None).unwrap();
    assert_eq!(
        deleted,
        reopened.read_account("user-a").unwrap().app_data_json
    );
    assert_eq!(
        fences,
        privacy_journal::recovery_policies(store.database_path(), &identity).unwrap()
    );
    reopened.validate_integrity().unwrap();
}

#[test]
fn storage_format_server_process_loss_before_and_after_journal_commit_recovers() {
    const ROOT_ENV: &str = "TENRATE_FORMAT_CRASH_DIR";
    const POINT_ENV: &str = "TENRATE_FORMAT_CRASH_POINT";
    if let Some(root) = std::env::var_os(ROOT_ENV) {
        let root = fs::canonicalize(root).unwrap();
        assert!(root.starts_with(fs::canonicalize(std::env::temp_dir()).unwrap()));
        assert!(root
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("gridtimer_server_store_test_format-crash_"));
        assert_eq!(
            "format-crash-v1",
            fs::read_to_string(root.join("format_fixture.txt")).unwrap()
        );
        let point = std::env::var(POINT_ENV).unwrap().parse::<u8>().unwrap();
        privacy_journal::FORMAT_PROCESS_INTERRUPTION.with(|value| value.set(Some(point)));
        let _ = SqliteServerStore::open(root.join("server_store.sqlite3"), None);
        panic!("format migration interruption was not reached");
    }
    for point in [0, 1] {
        let (directory, store, _, deleted) = storage_format_native15_fixture("format-crash");
        store
            .compare_and_swap_account("user-a", 1, &deleted, 200)
            .unwrap();
        let identity = store.server_instance_id().unwrap();
        let fences = privacy_journal::recovery_policies(store.database_path(), &identity).unwrap();
        storage_format_make_legacy_journal(&store);
        fs::write(directory.0.join("format_fixture.txt"), "format-crash-v1").unwrap();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.args(["--exact","server_store::tests::storage_format_server_process_loss_before_and_after_journal_commit_recovers","--nocapture","--test-threads=1"])
            .env(ROOT_ENV,&directory.0).env(POINT_ENV,point.to_string())
            .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let mut child = command.spawn().unwrap();
        let started = std::time::Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if started.elapsed() > Duration::from_secs(30) {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("owned migration child exceeded its timeout");
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let mut errors = String::new();
        use std::io::Read as _;
        child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut errors)
            .unwrap();
        assert_eq!(Some(87), status.code(), "{errors}");
        let reopened = SqliteServerStore::open(store.database_path(), None).unwrap();
        assert_eq!(
            deleted,
            reopened.read_account("user-a").unwrap().app_data_json
        );
        assert_eq!(identity, reopened.server_instance_id().unwrap());
        assert_eq!(
            fences,
            privacy_journal::recovery_policies(store.database_path(), &identity).unwrap()
        );
        reopened.validate_integrity().unwrap();
        let journal =
            Connection::open(privacy_journal::journal_path(store.database_path())).unwrap();
        assert_eq!(
            2,
            journal
                .query_row(
                    "SELECT format_version FROM privacy_journal_identity",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap()
        );
    }
}

#[test]
fn storage_format_15_typed_conflicts_gain_verified_indexes_without_rewriting_old_backup() {
    for live_bytes_available in [true, false] {
        let directory = TestDirectory::new("format-typed-conflict-history");
        let raw = typed_conflict_media_document("conflict-media");
        let path = directory.0.join("server_store.sqlite3");
        let media: Vec<(&str, &str, &[u8])> = if live_bytes_available {
            vec![("user-a", "conflict-media", b"conflict-media".as_slice())]
        } else {
            vec![]
        };
        SqliteServerStore::preschema_test_create_native_layout(
            &path,
            15,
            &[("user-a", &raw)],
            &media,
        )
        .unwrap();
        let store = SqliteServerStore {
            database_path: path,
        };
        storage_format_make_legacy_journal(&store);
        let opened = SqliteServerStore::open_with_options(ServerStoreOpenOptions {
            database_path: store.database_path().into(),
            legacy_json_path: None,
            now_epoch_millis: 200,
            legacy_token_ttl_millis: DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
        })
        .unwrap();
        let backup = opened.pre_schema_migration_backup.unwrap();
        let old = Connection::open(&backup.destination).unwrap();
        assert_eq!(15, current_schema_version(&old).unwrap());
        assert_eq!(
            0,
            old.query_row(
                "SELECT COUNT(*) FROM account_snapshot_media_history",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap()
        );
        drop(old);
        assert_eq!(
            backup.sha256,
            SqliteServerStore::verify_existing_backup(&backup.destination, 200)
                .unwrap()
                .sha256
        );
        let connection = opened.store.open_connection(false).unwrap();
        assert_eq!(1, connection.query_row("SELECT COUNT(*) FROM account_snapshot_media_identities WHERE user_id='user-a' AND attachment_id='conflict-media'", [], |row| row.get::<_, i64>(0)).unwrap());
        let (complete, captured, missing): (i64, Option<String>, String) = connection
            .query_row(
                "SELECT h.media_snapshot_complete,m.content_sha256,m.missing_reason
             FROM account_snapshot_history h JOIN account_snapshot_media_history m
             ON m.user_id=h.user_id AND m.account_revision=h.revision
             WHERE h.user_id='user-a' AND h.revision=0 AND m.attachment_id='conflict-media'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(i64::from(live_bytes_available), complete);
        assert_eq!(live_bytes_available, captured.is_some());
        assert_eq!(!live_bytes_available, !missing.is_empty());
        assert_eq!(
            raw,
            opened.store.read_account("user-a").unwrap().app_data_json
        );
        opened.store.validate_integrity().unwrap();
    }
}
