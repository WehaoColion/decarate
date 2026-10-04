// v0.0.4 - Exercise recovery archives with formats 14, 15 and 16.
// v0.0.3 - Recover real process loss during initialization and preserve unknown evidence.
// v0.0.2 - Notify maintenance only after commit, including failed post-commit publication.
// v0.0.1 - Prove privacy survives database loss and distinguish rollback from committed intent.
fn privacy_journal_fixture(label: &str) -> (TestDirectory, SqliteServerStore, String, String) {
    let directory = TestDirectory::new(label);
    let store = open_empty(&directory);
    store
        .create_user(new_user("user-a", "a@example.test"))
        .unwrap();
    let plain = server_privacy_audit_plain();
    store
        .compare_and_swap_account("user-a", 0, &plain, 100)
        .unwrap();
    let deleted =
        crate::app_data::delete_note_permanently_app_data_json(&plain, "private-note", 200)
            .unwrap();
    (directory, store, plain, deleted)
}

fn privacy_journal_interrupt<F, T>(point: u8, action: F) -> T
where
    F: FnOnce() -> T,
{
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            privacy_journal::COMMIT_INTERRUPTION.with(|value| value.set(0));
        }
    }
    privacy_journal::COMMIT_INTERRUPTION.with(|value| value.set(point));
    let _reset = Reset;
    action()
}

#[test]
fn backup_privacy_notifications_require_commit_and_isolate_databases() {
    for point in [0, 1, 2, 3] {
        let (_directory, store, plain, deleted) = privacy_journal_fixture("privacy-notification");
        let (_other_dir, other, _, _) = privacy_journal_fixture("privacy-notification-other");
        let subscription = store.subscribe_backup_privacy().unwrap();
        let other_subscription = other.subscribe_backup_privacy().unwrap();
        let ordinary = crate::app_data::upsert_note_app_data_json(
            &plain,
            r#"{"id":"unrelated","content":"ordinary edit"}"#,
            150,
        )
        .unwrap();
        store
            .compare_and_swap_account("user-a", 1, &ordinary, 150)
            .unwrap();
        assert_eq!(
            BackupPrivacyWake::Timeout,
            subscription.wait(Duration::ZERO)
        );
        let result = privacy_journal_interrupt(point, || {
            store.compare_and_swap_account("user-a", 2, &deleted, 200)
        });
        assert_eq!(point == 0, result.is_ok());
        let expected = if point == 1 {
            BackupPrivacyWake::Timeout
        } else {
            BackupPrivacyWake::Changed
        };
        assert_eq!(
            expected,
            subscription.wait(Duration::ZERO),
            "notification did not match durable commit"
        );
        assert_eq!(
            BackupPrivacyWake::Timeout,
            other_subscription.wait(Duration::ZERO)
        );
        assert_eq!(
            BackupPrivacyWake::Timeout,
            subscription.wait(Duration::ZERO)
        );
    }
}

#[test]
fn privacy_journal_lost_database_recovers_without_reviving_private_data() {
    for (seal, schema) in [
        (false, 14),
        (true, 14),
        (false, 15),
        (true, 15),
        (false, 16),
        (true, 16),
        (false, 17),
        (true, 17),
    ] {
        let (directory, store, plain, deleted) =
            privacy_journal_fixture("privacy-journal-database-loss");
        store
            .create_user(new_user("user-b", "b@example.test"))
            .unwrap();
        store
            .compare_and_swap_account("user-b", 0, &plain, 100)
            .unwrap();
        let backup = directory.0.join("old.sqlite3");
        store.create_verified_backup(&backup, 150).unwrap();
        if schema == 14 {
            let connection = Connection::open(&backup).unwrap();
            connection.execute_batch("DROP TABLE account_note_privacy; DROP TABLE note_privacy_commit_witnesses; DELETE FROM schema_migrations WHERE version>=15; PRAGMA user_version=14;").unwrap();
        } else if schema == 15 {
            Connection::open(&backup)
                .unwrap()
                .execute_batch(
                    "DELETE FROM schema_migrations WHERE version>=16; PRAGMA user_version=15;",
                )
                .unwrap();
        } else if schema == 16 {
            Connection::open(&backup)
                .unwrap()
                .execute_batch(
                    "DELETE FROM schema_migrations WHERE version>=17; PRAGMA user_version=16;",
                )
                .unwrap();
        }
        let old_report = SqliteServerStore::verify_existing_backup(&backup, 150).unwrap();
        let current = if seal {
            server_privacy_audit_sealed(&plain)
        } else {
            deleted
        };
        store
            .compare_and_swap_account("user-a", 1, &current, 200)
            .unwrap();
        let database = store.database_path().to_path_buf();
        store.finish_note_privacy_cleanup().unwrap();
        drop(store);
        fs::remove_file(&database).unwrap();
        let candidate = directory.0.join("restore.sqlite3");
        fs::copy(&backup, &candidate).unwrap();
        SqliteServerStore::apply_external_privacy_to_recovery_copy(&candidate, &database, 300)
            .unwrap();
        let recovered = SqliteServerStore::open(&candidate, None).unwrap();
        assert!(
            !recovered
                .read_account("user-a")
                .unwrap()
                .app_data_json
                .contains("SERVER_PRIVATE_MARKER_49371"),
            "independent recovery revived private text"
        );
        for history in recovered.list_snapshot_history("user-a", 100).unwrap() {
            assert!(
                !history
                    .app_data_json
                    .contains("SERVER_PRIVATE_MARKER_49371"),
                "independent recovery retained private history"
            );
        }
        assert_eq!(
            plain,
            recovered.read_account("user-b").unwrap().app_data_json
        );
        assert_eq!(
            old_report.sha256,
            sha256_file(&backup).unwrap(),
            "the provenance backup was silently overwritten"
        );
        recovered.validate_integrity().unwrap();
    }
}

#[test]
fn privacy_journal_concurrent_accounts_publish_without_losing_fences() {
    let directory = TestDirectory::new("privacy-journal-concurrent");
    let store = open_empty(&directory);
    let plain = server_privacy_audit_plain();
    let deleted =
        crate::app_data::delete_note_permanently_app_data_json(&plain, "private-note", 200)
            .unwrap();
    for index in 0..4 {
        let user = format!("parallel-{index}");
        store
            .create_user(new_user(&user, &format!("{user}@example.test")))
            .unwrap();
        store
            .compare_and_swap_account(&user, 0, &plain, 100)
            .unwrap();
    }
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
    let workers = (0..4)
        .map(|index| {
            let store = store.clone();
            let barrier = barrier.clone();
            let deleted = deleted.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store
                    .compare_and_swap_account(&format!("parallel-{index}"), 1, &deleted, 200)
                    .unwrap();
            })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.join().unwrap();
    }
    store.finish_note_privacy_cleanup().unwrap();
    let policies = privacy_journal::recovery_policies(
        store.database_path(),
        &store.server_instance_id().unwrap(),
    )
    .unwrap();
    assert_eq!(4, policies.len());
    for (user, _) in policies {
        assert_eq!(deleted, store.read_account(&user).unwrap().app_data_json);
    }
    let journal = Connection::open(privacy_journal::journal_path(store.database_path())).unwrap();
    assert_eq!((0, 0), journal.query_row(
        "SELECT (SELECT COUNT(*) FROM privacy_journal_pending), (SELECT COUNT(*) FROM privacy_journal_commits)",
        [], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    ).unwrap());
    store.validate_integrity().unwrap();
}

#[test]
fn privacy_journal_committed_interruption_is_published_on_restart() {
    for interruption in [2, 3] {
        let (_directory, store, _plain, deleted) =
            privacy_journal_fixture("privacy-journal-after-commit");
        let error = privacy_journal_interrupt(interruption, || {
            store.compare_and_swap_account("user-a", 1, &deleted, 200)
        })
        .unwrap_err();
        assert!(error.to_string().contains("injected interruption after"));
        let database = store.database_path().to_path_buf();
        let connection = store.open_connection(false).unwrap();
        assert_eq!(
            1,
            connection
                .query_row(
                    "SELECT COUNT(*) FROM note_privacy_commit_witnesses",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap()
        );
        drop(connection);
        drop(store);
        let reopened = SqliteServerStore::open(&database, None).unwrap();
        assert_eq!(
            deleted,
            reopened.read_account("user-a").unwrap().app_data_json
        );
        let policies =
            privacy_journal::recovery_policies(&database, &reopened.server_instance_id().unwrap())
                .unwrap();
        assert_eq!(1, policies.len());
        assert_eq!(
            0,
            reopened
                .open_connection(false)
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM note_privacy_commit_witnesses",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap()
        );
    }
}

#[test]
fn privacy_journal_rolled_back_intent_does_not_delete_data_on_restart() {
    let (_directory, store, plain, deleted) =
        privacy_journal_fixture("privacy-journal-before-commit");
    let before = store.list_snapshot_history("user-a", 100).unwrap();
    assert!(privacy_journal_interrupt(1, || store
        .compare_and_swap_account("user-a", 1, &deleted, 200))
    .is_err());
    assert_eq!(plain, store.read_account("user-a").unwrap().app_data_json);
    let database = store.database_path().to_path_buf();
    drop(store);
    let reopened = SqliteServerStore::open(&database, None).unwrap();
    assert_eq!(
        plain,
        reopened.read_account("user-a").unwrap().app_data_json
    );
    assert_eq!(
        before,
        reopened.list_snapshot_history("user-a", 100).unwrap()
    );
    assert!(
        privacy_journal::recovery_policies(&database, &reopened.server_instance_id().unwrap())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn privacy_journal_missing_main_with_uncertain_commit_cannot_publish_old_backup() {
    let (directory, store, _plain, deleted) =
        privacy_journal_fixture("privacy-journal-uncertain-recovery");
    let candidate = directory.0.join("candidate.sqlite3");
    store.create_verified_backup(&candidate, 150).unwrap();
    let original = sha256_file(&candidate).unwrap();
    assert!(privacy_journal_interrupt(2, || store
        .compare_and_swap_account("user-a", 1, &deleted, 200))
    .is_err());
    let database = store.database_path().to_path_buf();
    drop(store);
    fs::remove_file(&database).unwrap();
    assert!(
        SqliteServerStore::apply_external_privacy_to_recovery_copy(&candidate, &database, 300)
            .is_err()
    );
    assert!(!database.exists());
    assert_eq!(original, sha256_file(&candidate).unwrap());
}

#[test]
fn privacy_journal_rejects_foreign_and_altered_fences() {
    let (directory, store, _plain, deleted) = privacy_journal_fixture("privacy-journal-corruption");
    store
        .compare_and_swap_account("user-a", 1, &deleted, 200)
        .unwrap();
    let database = store.database_path().to_path_buf();
    let journal_path = privacy_journal::journal_path(&database);
    let journal = Connection::open(&journal_path).unwrap();
    journal
        .execute("UPDATE privacy_journal_policies SET policy_json='{}'", [])
        .unwrap();
    drop(journal);
    assert!(SqliteServerStore::open(&database, None).is_err());

    let other_path = directory.0.join("foreign.sqlite3");
    let other = SqliteServerStore::open(&other_path, None).unwrap();
    let foreign_path = privacy_journal::journal_path(&other_path);
    fs::copy(&journal_path, &foreign_path).unwrap();
    let before = sha256_file(&foreign_path).unwrap();
    assert!(SqliteServerStore::open(&other_path, None).is_err());
    assert_eq!(before, sha256_file(&foreign_path).unwrap());
    assert_eq!(0, other.stats().unwrap().users);
}

#[test]
fn privacy_journal_initialization_process_loss_recovers() {
    const ROOT_ENV: &str = "TENRATE_PRIVACY_INIT_CRASH_DIR";
    const POINT_ENV: &str = "TENRATE_PRIVACY_INIT_CRASH_POINT";
    if let Some(root) = std::env::var_os(ROOT_ENV) {
        let root = fs::canonicalize(root).unwrap();
        assert!(root.starts_with(fs::canonicalize(std::env::temp_dir()).unwrap()));
        assert!(root
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("gridtimer_server_store_test_privacy-init-crash_"));
        assert_eq!(
            "privacy-init-crash-v1",
            fs::read_to_string(root.join("init_crash_fixture.txt")).unwrap()
        );
        let database = root.join("server_store.sqlite3");
        let store = SqliteServerStore::open(&database, None).unwrap();
        let desired = fs::read_to_string(root.join("desired.json")).unwrap();
        let point = std::env::var(POINT_ENV).unwrap().parse::<u8>().unwrap();
        privacy_journal::INIT_PROCESS_INTERRUPTION.with(|value| value.set(Some(point)));
        let _ = store.compare_and_swap_account("user-a", 1, &desired, 200);
        panic!("child did not reach initialization interruption");
    }
    for point in 0..=3 {
        let (directory, store, plain, deleted) = privacy_journal_fixture("privacy-init-crash");
        let database = store.database_path().to_path_buf();
        assert_eq!(directory.0.join("server_store.sqlite3"), database);
        let desired = if point % 2 == 0 {
            deleted
        } else {
            server_privacy_audit_sealed(&plain)
        };
        let backup = directory.0.join("before.sqlite3");
        store.create_verified_backup(&backup, 150).unwrap();
        fs::write(
            directory.0.join("init_crash_fixture.txt"),
            "privacy-init-crash-v1",
        )
        .unwrap();
        fs::write(directory.0.join("desired.json"), &desired).unwrap();
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "server_store::tests::privacy_journal_initialization_process_loss_recovers",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(ROOT_ENV, &directory.0)
            .env(POINT_ENV, point.to_string())
            .output()
            .unwrap();
        assert_eq!(
            Some(86),
            child.status.code(),
            "initialization interruption not reached: {}",
            String::from_utf8_lossy(&child.stderr)
        );
        let reopened = SqliteServerStore::open(&database, None)
            .expect("interrupted journal creation blocked restart");
        assert_eq!(
            plain,
            reopened.read_account("user-a").unwrap().app_data_json,
            "uncommitted privacy operation changed the account"
        );
        reopened
            .compare_and_swap_account("user-a", 1, &desired, 200)
            .unwrap();
        reopened.finish_note_privacy_cleanup().unwrap();
        let prefix = privacy_journal::initialization_prefix(
            &database,
            &reopened.server_instance_id().unwrap(),
        )
        .unwrap();
        assert!(
            !fs::read_dir(&directory.0).unwrap().any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(&prefix)),
            "unused initialization survived a successful retry"
        );
        let recovery = directory.0.join("restored.sqlite3");
        fs::copy(&backup, &recovery).unwrap();
        SqliteServerStore::apply_external_privacy_to_recovery_copy(&recovery, &database, 300)
            .unwrap();
        let restored = SqliteServerStore::open(&recovery, None).unwrap();
        assert!(
            !restored
                .read_account("user-a")
                .unwrap()
                .app_data_json
                .contains("SERVER_PRIVATE_MARKER_49371"),
            "retry lost its independent privacy fence"
        );
        restored.validate_integrity().unwrap();
    }
}

#[test]
fn privacy_journal_initialization_preserves_existing_and_uncertain_files() {
    let (directory, store, plain, deleted) = privacy_journal_fixture("privacy-init-preservation");
    let database = store.database_path();
    let path = privacy_journal::journal_path(database);
    fs::write(&path, []).unwrap();
    assert!(SqliteServerStore::open(database, None).is_err());
    assert!(store
        .compare_and_swap_account("user-a", 1, &deleted, 200)
        .is_err());
    assert_eq!(
        0,
        fs::metadata(&path).unwrap().len(),
        "preexisting empty evidence was replaced"
    );
    assert_eq!(plain, store.read_account("user-a").unwrap().app_data_json);
    fs::remove_file(&path).unwrap();

    let orphan = sqlite_sidecar_path(&path, "-journal");
    fs::write(&orphan, b"unrecognized rollback evidence").unwrap();
    let orphan_hash = sha256_file(&orphan).unwrap();
    assert!(store
        .compare_and_swap_account("user-a", 1, &deleted, 200)
        .is_err());
    assert!(
        !path.exists(),
        "a journal was published over orphaned recovery evidence"
    );
    assert_eq!(orphan_hash, sha256_file(&orphan).unwrap());
    fs::remove_file(&orphan).unwrap();

    let prefix =
        privacy_journal::initialization_prefix(database, &store.server_instance_id().unwrap())
            .unwrap();
    let empty = directory
        .0
        .join(format!("{prefix}{}.sqlite3", "0".repeat(32)));
    let foreign = directory
        .0
        .join(format!("{prefix}{}.sqlite3", "1".repeat(32)));
    let uncertain = directory
        .0
        .join(format!("{prefix}{}.sqlite3", "2".repeat(32)));
    fs::write(&empty, []).unwrap();
    fs::write(&foreign, b"unrelated file in the reserved namespace").unwrap();
    fs::write(&uncertain, []).unwrap();
    let uncertain_sidecar = sqlite_sidecar_path(&uncertain, "-journal");
    fs::write(&uncertain_sidecar, b"unfinished sqlite transaction").unwrap();
    let before = [
        (&foreign, sha256_file(&foreign).unwrap()),
        (&uncertain, sha256_file(&uncertain).unwrap()),
        (&uncertain_sidecar, sha256_file(&uncertain_sidecar).unwrap()),
    ];
    store
        .compare_and_swap_account("user-a", 1, &deleted, 200)
        .unwrap();
    assert!(!empty.exists(), "unused reserved file was not reclaimed");
    for (file, hash) in before {
        assert_eq!(
            hash,
            sha256_file(file).unwrap(),
            "uncertain initialization evidence was changed"
        );
    }
    assert_eq!(deleted, store.read_account("user-a").unwrap().app_data_json);

    // A correctly named and correctly bound staging file is still evidence
    // once it contains a fence; only a completely unused initializer is disposable.
    let committed = directory
        .0
        .join(format!("{prefix}{}.sqlite3", "3".repeat(32)));
    fs::copy(&path, &committed).unwrap();
    let committed_hash = sha256_file(&committed).unwrap();
    fs::remove_file(&path).unwrap();
    let reopened = SqliteServerStore::open(database, None).unwrap();
    assert!(committed.exists(), "committed temporary fence was removed");
    assert_eq!(committed_hash, sha256_file(&committed).unwrap());
    assert_eq!(
        1,
        privacy_journal::recovery_policies(database, &reopened.server_instance_id().unwrap())
            .unwrap()
            .len()
    );
}

#[test]
fn privacy_journal_initialization_publication_never_replaces_existing_destination() {
    let directory = TestDirectory::new("privacy-init-publish");
    let temporary = directory.0.join("new.sqlite3");
    let destination = directory.0.join("existing.sqlite3");
    fs::write(&temporary, b"new initialization").unwrap();
    fs::write(&destination, b"existing committed privacy fence").unwrap();
    let old_hash = sha256_file(&destination).unwrap();
    let temp_hash = sha256_file(&temporary).unwrap();
    assert!(
        privacy_journal::publish_initial_journal(&temporary, &destination).is_err(),
        "publication replaced existing privacy evidence"
    );
    assert_eq!(old_hash, sha256_file(&destination).unwrap());
    assert_eq!(temp_hash, sha256_file(&temporary).unwrap());
}
