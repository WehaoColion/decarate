// v0.0.2 - Cover schema 10/11 archives and the earlier upgrade filename.
// v0.0.1 - Preserve old-schema recovery while erasing private note generations.
fn preschema_other(count: usize) -> String {
    let sha = hex_bytes(&Sha256::digest([]));
    let attachments=(0..count).map(|i|json!({"id":format!("other-media-{i}"),"sha256":sha,"mimeType":"application/octet-stream","sizeBytes":0,"updatedAtEpochMillis":100})).collect::<Vec<_>>();
    app_data::upsert_note_app_data_json(
        &app_data::default_app_data_json(100),
        &json!({"id":"backup-private","content":"OTHER_ACCOUNT_ARCHIVE_48192","attachments":attachments}).to_string(),
        100,
    ).unwrap()
}
fn preschema_privacy_fixture(version: i64) -> (TestSqliteStore, String, PathBuf, PathBuf) {
    let (mut fixture, plain, _, runtime) = backup_privacy_fixture();
    let other = preschema_other(2);
    fixture
        .store
        .create_user(NewStoredUser {
            id: "user-2".into(),
            email: "other-archive@example.test".into(),
            password_salt: "other-salt".into(),
            password_hash: password_hash("other-salt", "other-password"),
            password_scheme: "legacy_sha256".into(),
            created_at_epoch_millis: 1,
            updated_at_epoch_millis: 1,
            app_data_json: other,
            account_revision: 0,
        })
        .unwrap();
    fixture
        .store
        .preschema_test_seed_history_and_receipts()
        .unwrap();
    let connection = rusqlite::Connection::open(fixture.store.database_path()).unwrap();
    connection.execute_batch("DROP TABLE account_note_privacy; DROP TABLE note_privacy_commit_witnesses; DELETE FROM schema_migrations WHERE version>=13;").unwrap();
    if version <= 12 {
        connection.execute_batch("DROP TABLE account_snapshot_media_identities; DROP TABLE legacy_snapshot_repair_allowances; DROP INDEX account_snapshot_media_history_identity_index;
            DROP TRIGGER note_media_metadata_insert_guard; DROP TRIGGER note_media_metadata_update_guard;
            DROP TRIGGER note_media_tombstone_insert_guard; DROP TRIGGER note_media_tombstone_update_guard;
            DROP TRIGGER media_history_metadata_insert_guard; DROP TRIGGER media_history_metadata_update_guard;
            PRAGMA user_version=12;").unwrap();
    } else {
        connection.execute_batch("INSERT INTO schema_migrations VALUES(13,'synthetic original v13',130); PRAGMA user_version=13;").unwrap();
    }
    if version < 12 {
        connection
            .execute("DELETE FROM schema_migrations WHERE version>?1", [version])
            .unwrap();
        connection
            .pragma_update(None, "user_version", version)
            .unwrap();
        if version == 10 {
            connection
                .execute_batch(
                    "DROP INDEX tokens_pending_activation_index;
                ALTER TABLE tokens DROP COLUMN activation_state;
                ALTER TABLE tokens DROP COLUMN activated_at_epoch_millis;",
                )
                .unwrap();
        }
    }
    drop(connection);
    if version == 13 {
        fixture.store.preschema_test_v13_allowances().unwrap();
    }
    let opened = SqliteServerStore::open_with_options(ServerStoreOpenOptions {
        database_path: fixture.store.database_path().to_path_buf(),
        legacy_json_path: None,
        now_epoch_millis: 180,
        legacy_token_ttl_millis: TOKEN_TTL_MILLIS,
    })
    .unwrap();
    let mut backup = opened.pre_schema_migration_backup.unwrap().destination;
    if version < 12 {
        let old_upgrade_name = backup
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .replace("_to_v17_", "_to_v12_");
        let old_path = backup.with_file_name(old_upgrade_name);
        fs::rename(&backup, &old_path).unwrap();
        backup = old_path;
    }
    fixture.store = opened.store;
    (fixture, plain, runtime, backup)
}

fn preschema_proof(directory: &Path) -> PathBuf {
    fs::read_dir(directory)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".preschema_privacy_")
        })
        .unwrap()
}

#[test]
fn backup_privacy_preschema_keeps_repair_authority_and_audit_after_main_rollback() {
    for version in [12, 13] {
        let (fixture, plain, runtime, backup) = preschema_privacy_fixture(version);
        let reduced = preschema_other(1);
        assert!(fixture
            .store
            .preschema_test_repair("user-2", &reduced, false)
            .unwrap());
        let baseline = fixture
            .directory
            .join("synthetic-main-before-delete.sqlite3");
        fixture
            .store
            .create_verified_backup(&baseline, 190)
            .unwrap();
        let backup_sha = sha256_hex_for_test(&fs::read(&backup).unwrap());
        backup_privacy_delete(&fixture, &plain);
        backup_privacy::clean(&fixture.store, &runtime).unwrap();
        assert!(fixture
            .store
            .preschema_test_repair("user-2", &reduced, false)
            .unwrap());
        fixture.store.finish_note_privacy_cleanup().unwrap();
        fs::copy(&baseline, fixture.store.database_path()).unwrap();
        let reopened = SqliteServerStore::open(fixture.store.database_path(), None).unwrap();
        assert!(!reopened
            .read_account("user-1")
            .unwrap()
            .app_data_json
            .contains("OLD_BACKUP_PRIVATE_78301"));
        assert!(reopened
            .preschema_test_repair("user-2", &reduced, true)
            .unwrap());
        assert!(!reopened
            .preschema_test_repair("user-2", &reduced, true)
            .unwrap());
        let connection = rusqlite::Connection::open(reopened.database_path()).unwrap();
        let audit:String=connection.query_row("SELECT details_json FROM snapshot_history_prune_audit WHERE user_id='user-2' ORDER BY id DESC LIMIT 1",[],|r|r.get(0)).unwrap();
        let audit: Value = serde_json::from_str(&audit).unwrap();
        assert_eq!(
            audit["backupSha256"], backup_sha,
            "original audit root changed"
        );
        assert_ne!(sha256_hex_for_test(&fs::read(&backup).unwrap()), backup_sha);
    }
}

fn sha256_hex_for_test(bytes: &[u8]) -> String {
    hex_bytes(&Sha256::digest(bytes))
}

#[test]
fn backup_privacy_preschema_rejects_tampered_provenance_and_external_replacement() {
    let (fixture, plain, runtime, backup) = preschema_privacy_fixture(12);
    backup_privacy_delete(&fixture, &plain);
    backup_privacy::clean(&fixture.store, &runtime).unwrap();
    let proof = preschema_proof(&fixture.directory);
    let original = fs::read(&proof).unwrap();
    let bytes = fs::read(&backup).unwrap();
    let mut tampered: Value = serde_json::from_slice(&original).unwrap();
    tampered["mac"] = json!("00".repeat(32));
    fs::write(&proof, serde_json::to_vec(&tampered).unwrap()).unwrap();
    assert!(
        fixture
            .store
            .preschema_test_repair("user-2", &preschema_other(1), false)
            .is_err(),
        "tampered privacy provenance authorized repair"
    );
    assert_eq!(bytes, fs::read(&backup).unwrap());
    fs::write(&proof, &original).unwrap();
    assert!(fixture
        .store
        .preschema_test_repair("user-2", &preschema_other(1), false)
        .unwrap());
    let connection = rusqlite::Connection::open(&backup).unwrap();
    connection
        .execute(
            "UPDATE users SET updated_at_epoch_millis=updated_at_epoch_millis+1 WHERE id='user-2'",
            [],
        )
        .unwrap();
    drop(connection);
    let changed = fs::read(&backup).unwrap();
    assert!(fixture
        .store
        .preschema_test_repair("user-2", &preschema_other(1), false)
        .is_err());
    assert!(backup_privacy::clean(&fixture.store, &runtime).is_err());
    assert_eq!(changed, fs::read(&backup).unwrap());
    assert_eq!(original, fs::read(&proof).unwrap());
    // Abort the uncommitted rewrite while its source still matches. Restoring
    // an independently verified file during that transaction must fail closed.
    backup_privacy::resume(fixture.store.database_path(), &runtime).unwrap();
    assert_eq!(changed, fs::read(&backup).unwrap());
    fs::write(&backup, &bytes).unwrap();
    assert!(fixture
        .store
        .preschema_test_repair("user-2", &preschema_other(1), false)
        .unwrap());
}

#[test]
fn backup_privacy_preschema_interruption_recovers_without_losing_original_proof() {
    for version in [10, 11, 12] {
        for point in 0..4 {
            let (fixture, plain, runtime, backup) = preschema_privacy_fixture(version);
            backup_privacy_delete(&fixture, &plain);
            assert!(backup_privacy_interrupt(
                point,
                || backup_privacy::rewrite_preschema_for_test(&fixture.store, &backup, 180)
            )
            .is_err());
            backup_privacy::resume(fixture.store.database_path(), &runtime).unwrap();
            backup_privacy::clean(&fixture.store, &runtime).unwrap();
            assert!(!backup_privacy_has_marker(
                &backup,
                "OLD_BACKUP_PRIVATE_78301"
            ));
            if version == 12 {
                assert!(fixture
                    .store
                    .preschema_test_repair("user-2", &preschema_other(1), false)
                    .unwrap());
            }
            let connection = rusqlite::Connection::open(&backup).unwrap();
            assert_eq!(
                version,
                connection
                    .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .unwrap()
            );
            drop(connection);
            assert_eq!(0, backup_privacy::clean(&fixture.store, &runtime).unwrap());
            assert!(!fs::read_dir(&fixture.directory).unwrap().any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".privacy_copy_")));
        }
    }
}

#[test]
fn backup_privacy_preschema_keeps_foreign_and_invalid_early_archives_untouched() {
    for version in [10, 11] {
        let (fixture, plain, runtime, backup) = preschema_privacy_fixture(version);
        let (foreign, _, _, foreign_backup) = preschema_privacy_fixture(version);
        let foreign_copy = backup.with_file_name(format!(
            "server_store_pre_schema_v{version}_to_v12_170.sqlite3"
        ));
        fs::copy(&foreign_backup, &foreign_copy).unwrap();
        let corrupt_copy = backup.with_file_name(format!(
            "server_store_pre_schema_v{version}_to_v12_160.sqlite3"
        ));
        fs::copy(&backup, &corrupt_copy).unwrap();
        let connection = rusqlite::Connection::open(&corrupt_copy).unwrap();
        connection
            .execute(
                "UPDATE account_snapshots SET content_sha256=?1 WHERE user_id='user-1'",
                ["00".repeat(32)],
            )
            .unwrap();
        drop(connection);
        let before = [&foreign_copy, &corrupt_copy].map(|path| {
            (
                fs::read(path).unwrap(),
                fs::metadata(path).unwrap().modified().unwrap(),
            )
        });
        backup_privacy_delete(&fixture, &plain);
        backup_privacy::clean(&fixture.store, &runtime).unwrap();
        assert!(!backup_privacy_has_marker(
            &backup,
            "OLD_BACKUP_PRIVATE_78301"
        ));
        for (path, (bytes, modified)) in [&foreign_copy, &corrupt_copy].into_iter().zip(before) {
            assert_eq!(
                bytes,
                fs::read(path).unwrap(),
                "unowned or invalid archive was rewritten"
            );
            assert_eq!(modified, fs::metadata(path).unwrap().modified().unwrap());
        }
        assert!(backup_privacy_has_marker(
            &foreign_backup,
            "OLD_BACKUP_PRIVATE_78301"
        ));
        drop(foreign);
    }
}

#[test]
fn backup_privacy_preschema_redacts_without_losing_other_accounts() {
    for version in [10, 11, 12, 13] {
        for seal in [false, true] {
            let (fixture, plain, runtime, backup) = preschema_privacy_fixture(version);
            let connection = rusqlite::Connection::open(&backup).unwrap();
            let mut preserved_queries = vec![
                "SELECT * FROM users ORDER BY id",
                "SELECT * FROM tokens ORDER BY token_hash",
                "SELECT * FROM server_identity",
                "SELECT request_id,user_id,request_fingerprint,account_revision,created_at_epoch_millis FROM request_dedup ORDER BY request_id",
                "SELECT response_json FROM request_dedup WHERE user_id='user-2'",
            ];
            if version == 13 {
                preserved_queries
                    .push("SELECT * FROM legacy_snapshot_repair_allowances ORDER BY user_id");
            }
            let preserved: Vec<_> = preserved_queries
                .iter()
                .map(|sql| preschema_rows(&connection, sql))
                .collect();
            let other_history = preschema_rows(&connection,"SELECT c.content FROM account_snapshot_history h JOIN snapshot_contents c ON c.sha256=h.content_sha256 WHERE h.user_id='user-2' ORDER BY h.revision");
            drop(connection);
            let other_before = fixture.store.read_account("user-2").unwrap();
            let desired = if seal {
                let data: Value = serde_json::from_str(&plain).unwrap();
                let note = data["notes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|n| n["id"] == "backup-private")
                    .unwrap();
                let (sealed, session) =
                    crate::encrypt_desktop_note_json(&note.to_string(), "archive-password")
                        .unwrap();
                crate::close_desktop_note_session(&session);
                app_data::upsert_note_app_data_json(&plain, &sealed, 200).unwrap()
            } else {
                app_data::delete_note_permanently_app_data_json(&plain, "backup-private", 200)
                    .unwrap()
            };
            fixture
                .store
                .compare_and_swap_account("user-1", 0, &desired, 200)
                .unwrap();
            backup_privacy::clean(&fixture.store, &runtime).unwrap();
            assert!(
                !backup_privacy_has_marker(&backup, "OLD_BACKUP_PRIVATE_78301"),
                "old schema {version} backup retained private plaintext"
            );
            assert!(backup_privacy_has_marker(&backup, "KEEP_ORDINARY_92713"));
            let connection = rusqlite::Connection::open_with_flags(
                &backup,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            assert_eq!(
                version,
                connection
                    .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .unwrap()
            );
            let other: String = connection
                .query_row(
                    "SELECT app_data_json FROM account_snapshots WHERE user_id='user-2'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(other_before.app_data_json, other);
            for (sql, before) in preserved_queries.iter().zip(preserved) {
                assert_eq!(
                    before,
                    preschema_rows(&connection, sql),
                    "archive metadata changed: {sql}"
                );
            }
            assert_eq!(other_history, preschema_rows(&connection,"SELECT c.content FROM account_snapshot_history h JOIN snapshot_contents c ON c.sha256=h.content_sha256 WHERE h.user_id='user-2' ORDER BY h.revision"));
            let mut statement = connection
                .prepare("SELECT content FROM snapshot_contents")
                .unwrap();
            let compressed: Vec<Vec<u8>> = statement
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert!(!compressed.is_empty());
            for bytes in compressed {
                let mut decoded = String::new();
                std::io::Read::read_to_string(
                    &mut flate2::read::ZlibDecoder::new(bytes.as_slice()),
                    &mut decoded,
                )
                .unwrap();
                assert!(
                    !decoded.contains("OLD_BACKUP_PRIVATE_78301"),
                    "compressed history retained private content"
                );
            }
            drop(statement);
            let receipt: String = connection
                .query_row(
                    "SELECT response_json FROM request_dedup WHERE user_id='user-1'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(!receipt.contains("OLD_BACKUP_PRIVATE_78301"));
            assert!(receipt.contains("KEEP_ORDINARY_92713"));
            drop(connection);
            let bytes = fs::read(&backup).unwrap();
            let modified = fs::metadata(&backup).unwrap().modified().unwrap();
            assert_eq!(0, backup_privacy::clean(&fixture.store, &runtime).unwrap());
            assert_eq!(bytes, fs::read(&backup).unwrap());
            assert_eq!(modified, fs::metadata(&backup).unwrap().modified().unwrap());
        }
    }
}

fn preschema_rows(
    connection: &rusqlite::Connection,
    sql: &str,
) -> Vec<Vec<rusqlite::types::Value>> {
    let mut statement = connection.prepare(sql).unwrap();
    let count = statement.column_count();
    statement
        .query_map([], |row| (0..count).map(|i| row.get(i)).collect())
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}
