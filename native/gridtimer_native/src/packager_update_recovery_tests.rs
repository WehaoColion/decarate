// Windows - Exercise real publication loss with distinct versions and verified prepared images.
// Database fixtures keep genuine candidate format; no synthetic marker downgrade.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateRehearsalInputs {
    current: PathBuf,
    stable: PathBuf,
    candidate_source_root: PathBuf,
    android_sdk: PathBuf,
    evidence: PathBuf,
}

fn update_rehearsal_inputs() -> UpdateRehearsalInputs {
    let path = PathBuf::from(env::var_os("TENRATE_UPDATE_REHEARSAL_INPUTS").unwrap());
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn update_rehearsal_artifacts(_root: &Path) -> WindowsReleaseArtifacts {
    let input = update_rehearsal_inputs();
    load_prepared_windows(&input.candidate_source_root)
        .expect("the full candidate must pass the real preparation and integrity gates")
}

fn rehearsal_server_path(artifacts: &WindowsReleaseArtifacts) -> &Path {
    &artifacts
        .current
        .iter()
        .find(|item| item.public_base_name == "grid_timer_sync_server")
        .expect("complete prepared artifacts")
        .source
}

fn update_rehearsal_owned_root(root: &Path) {
    let canonical = fs::canonicalize(root).unwrap();
    assert!(canonical.starts_with(fs::canonicalize(env::temp_dir()).unwrap()));
    assert!(canonical
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("grid-timer-packager-update-process-"));
    assert_eq!(
        fs::read(root.join("rehearsal.marker")).unwrap(),
        b"update-rehearsal-v1"
    );
}

fn update_rehearsal_wait(
    command: &mut Command,
    root: &Path,
    label: &str,
    seconds: u64,
) -> std::process::ExitStatus {
    use std::os::windows::process::CommandExt;
    let output = File::create(root.join(format!("{label}.out"))).unwrap();
    let error = File::create(root.join(format!("{label}.err"))).unwrap();
    command
        .stdin(Stdio::null())
        .stdout(output)
        .stderr(error)
        .creation_flags(0x0800_0000);
    let mut child = command.spawn().unwrap();
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if start.elapsed() > Duration::from_secs(seconds) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("Owned update rehearsal child {label} exceeded {seconds} seconds");
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn update_rehearsal_seed_data(root: &Path) -> serde_json::Value {
    use gridtimer_native::{
        app_data,
        desktop_state_store::DesktopStateStore,
        server_store::{NewStoredUser, SqliteServerStore},
    };
    let snapshot = app_data::upsert_note_app_data_json(&app_data::default_app_data_json(100),
        r#"{"id":"update-record","title":"Before update","content":"Retain this record","updatedAtEpochMillis":100}"#,100).unwrap();
    let database = root.join("data/server.sqlite3");
    let store = SqliteServerStore::open(&database, None).unwrap();
    store
        .create_user(NewStoredUser {
            id: "update-owner".into(),
            email: "update@example.invalid".into(),
            password_salt: "synthetic-salt".into(),
            password_hash: "a".repeat(64),
            password_scheme: "legacy_sha256".into(),
            created_at_epoch_millis: 100,
            updated_at_epoch_millis: 100,
            app_data_json: snapshot.clone(),
            account_revision: 0,
        })
        .unwrap();
    let pending = app_data::upsert_note_app_data_json(&snapshot,
        r#"{"id":"deleted-before-publication","content":"Permanent deletion fence","updatedAtEpochMillis":200}"#, 200).unwrap();
    let original = store.read_account("update-owner").unwrap();
    store
        .compare_and_swap_account("update-owner", original.revision, &pending, 200)
        .unwrap();
    let snapshot = app_data::delete_note_permanently_app_data_json(
        &pending,
        "deleted-before-publication",
        300,
    )
    .unwrap();
    let original = store.read_account("update-owner").unwrap();
    store
        .compare_and_swap_account("update-owner", original.revision, &snapshot, 300)
        .unwrap();
    let identity = store.server_account_identity("update-owner").unwrap();
    let journal = DesktopStateStore::open(root.join("data/desktop.sqlite3")).unwrap();
    journal
        .record_with_sync_state(
            "update-owner",
            &snapshot,
            b"update-protected-state",
            100,
            "update-rehearsal",
        )
        .unwrap();
    drop(journal);
    drop(store);
    // Publication recovery must preserve data without opening it. Keep real
    // new-format state here, including the independent privacy journal. The
    // runtime rehearsal seeds schema 15 using the actual old server.
    serde_json::json!({"server":identity.server_instance_id,"account":identity.account_namespace,"snapshot":snapshot})
}

fn update_rehearsal_data_digest(root: &Path) -> serde_json::Value {
    let mut result = serde_json::Map::new();
    for name in [
        "server.sqlite3",
        "server.sqlite3.note_privacy.sqlite3",
        "desktop.sqlite3",
    ] {
        let connection = rusqlite::Connection::open_with_flags(
            root.join("data").join(name),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let tables = connection
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let mut values = serde_json::Map::new();
        for table in tables {
            let sql = format!("SELECT * FROM \"{}\"", table.replace('"', "\"\""));
            let mut statement = connection.prepare(&sql).unwrap();
            let columns = statement.column_count();
            let mut rows = statement
                .query_map([], |row| {
                    (0..columns)
                        .map(|i| row.get_ref(i).map(|value| format!("{value:?}")))
                        .collect::<Result<Vec<_>, _>>()
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            rows.sort();
            values.insert(table,serde_json::json!({"rows":rows.len(),"sha256":format!("{:x}",Sha256::digest(serde_json::to_vec(&rows).unwrap()))}));
        }
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        let integrity: String = connection
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
        result.insert(
            name.into(),
            serde_json::json!({"schema":version,"tables":values}),
        );
    }
    serde_json::Value::Object(result)
}

fn update_rehearsal_commit_new_data(root: &Path, expected: &serde_json::Value) {
    use gridtimer_native::{
        app_data, desktop_state_store::DesktopStateStore, server_store::SqliteServerStore,
    };
    let store = SqliteServerStore::open(root.join("data/server.sqlite3"), None).unwrap();
    let identity = store.server_account_identity("update-owner").unwrap();
    assert_eq!(identity.server_instance_id, expected["server"]);
    assert_eq!(identity.account_namespace, expected["account"]);
    let previous = store.read_account("update-owner").unwrap();
    assert_eq!(previous.app_data_json, expected["snapshot"]);
    let snapshot=app_data::upsert_note_app_data_json(&previous.app_data_json,
        r#"{"id":"new-update-record","title":"After commit","content":"New work must survive recovery","updatedAtEpochMillis":400}"#,400).unwrap();
    store
        .compare_and_swap_account("update-owner", previous.revision, &snapshot, 400)
        .unwrap();
    assert_eq!(
        store.read_account("update-owner").unwrap().app_data_json,
        snapshot
    );
    let journal = DesktopStateStore::open(root.join("data/desktop.sqlite3")).unwrap();
    assert_eq!(
        journal
            .latest_valid("update-owner", 400)
            .unwrap()
            .unwrap()
            .protected_sync_state,
        b"update-protected-state"
    );
    journal
        .record_with_sync_state(
            "update-owner",
            &snapshot,
            b"update-protected-state",
            400,
            "after-release-commit",
        )
        .unwrap();
    assert_eq!(
        journal
            .latest_valid("update-owner", 500)
            .unwrap()
            .unwrap()
            .app_data_json,
        snapshot
    );
}

#[test]
#[ignore = "Explicit isolated update process-loss rehearsal with verified installed binaries; no production update"]
#[cfg(windows)]
fn update_process_loss_recovers_release_and_preserves_data() {
    let inputs = update_rehearsal_inputs();
    if let Some(root) = env::var_os("TENRATE_UPDATE_REHEARSAL_CHILD") {
        let root = PathBuf::from(root);
        update_rehearsal_owned_root(&root);
        let point = env::var("TENRATE_UPDATE_REHEARSAL_POINT").unwrap();
        RELEASE_PROCESS_INTERRUPTION.with(|value| *value.borrow_mut() = Some(point));
        let current = root.join("release_artifacts/current");
        let retained = RetainedAndroidRelease::capture_published(&root).unwrap().0;
        let result = publish_release_transaction_with_scope(
            &root,
            &sample_version(product_identity::WINDOWS_APP_VERSION),
            &inputs.android_sdk,
            retained.current_apk(),
            &update_rehearsal_artifacts(&root),
            true,
        );
        panic!(
            "Publication did not reach process loss: {result:?}, current={}",
            current.display()
        );
    }
    let installed = derive_release_set_descriptor(&inputs.current).unwrap();
    assert_ne!(
        installed.version,
        product_identity::WINDOWS_APP_VERSION,
        "old and candidate Windows versions must be genuinely distinct"
    );
    let artifacts = update_rehearsal_artifacts(&inputs.current);
    let cases = [
        "prepared",
        "phase_StableSwitching",
        "stable_switched",
        "phase_CurrentSwitching",
        "current_removed",
        "current_switched",
        "phase_RootPublishing",
        "phase_Committed",
        "archived_sync_server",
        "archives_complete",
        "finalized",
    ];
    fs::create_dir_all(&inputs.evidence).unwrap();
    let mut results = Vec::new();
    for (index, point) in cases.iter().enumerate() {
        let project = TestDirectory::new("update-process");
        let root = project.path();
        fs::write(root.join("rehearsal.marker"), b"update-rehearsal-v1").unwrap();
        let current = root.join("release_artifacts/current");
        fs::create_dir_all(current.join("tools")).unwrap();
        fs::create_dir_all(root.join("release_artifacts/desktop_entry")).unwrap();
        fs::create_dir_all(root.join("tools")).unwrap();
        fs::create_dir(root.join("APK")).unwrap();
        fs::create_dir(root.join("inputs")).unwrap();
        for descriptor in &installed.files {
            fs::copy(
                release_set_file_path(&inputs.current, descriptor),
                release_set_file_path(&current, descriptor),
            )
            .unwrap();
        }
        let apk = release_apk_descriptor(&installed).unwrap();
        fs::copy(current.join(&apk.file_name), root.join(&apk.file_name)).unwrap();
        fs::copy(
            current.join(&apk.file_name),
            root.join("APK").join(&apk.file_name),
        )
        .unwrap();
        fs::copy(&inputs.stable, stable_desktop_entry_path(root)).unwrap();
        fs::copy(
            current.join("tools/cloudflared.exe"),
            root.join("tools/cloudflared.exe"),
        )
        .unwrap();
        let retained = RetainedAndroidRelease::capture_published(root).unwrap().0;
        let expected = update_rehearsal_seed_data(root);
        let before = update_rehearsal_data_digest(root);
        let mut child = Command::new(env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "tests::update_process_loss_recovers_release_and_preserves_data",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("TENRATE_UPDATE_REHEARSAL_CHILD", root)
            .env("TENRATE_UPDATE_REHEARSAL_POINT", point);
        let status = update_rehearsal_wait(&mut child, root, "publisher", 180);
        let case_dir = inputs.evidence.join(format!("{index:02}_{point}"));
        fs::create_dir(&case_dir).unwrap();
        for file in ["publisher.out", "publisher.err"] {
            fs::copy(root.join(file), case_dir.join(file)).unwrap();
        }
        assert_eq!(
            status.code(),
            Some(86),
            "{point}: {}",
            fs::read_to_string(root.join("publisher.err")).unwrap()
        );
        let journal = load_transaction_journal(root).unwrap();
        if let Some(journal) = &journal {
            fs::copy(
                root.join("release_artifacts")
                    .join(RELEASE_TRANSACTION_JOURNAL_NAME),
                case_dir.join("journal.json"),
            )
            .unwrap();
            assert_eq!(journal.windows_only, true);
        }
        let committed = index >= 7;
        let expected_server = if committed {
            sha256_file_hex(rehearsal_server_path(&artifacts)).unwrap()
        } else {
            let old = installed
                .files
                .iter()
                .find(|file| file.role == "sync_server")
                .unwrap();
            (old.size, old.sha256.clone())
        };
        let expected_version = if committed {
            product_identity::WINDOWS_APP_VERSION
        } else {
            &installed.version
        };
        let mut resolve = Command::new(stable_desktop_entry_path(root));
        resolve.args(["--sync-supervisor", "--resolve-only"]);
        assert!(
            update_rehearsal_wait(&mut resolve, root, "resolve", 60).success(),
            "{point}: {}",
            fs::read_to_string(root.join("resolve.err")).unwrap()
        );
        let resolved = fs::read_to_string(root.join("resolve.out")).unwrap();
        assert_eq!(
            fs::canonicalize(resolved.trim()).unwrap(),
            fs::canonicalize(current.join(versioned_artifact_name(
                "grid_timer_sync_launcher",
                expected_version,
                "exe"
            )))
            .unwrap()
        );
        assert_eq!(
            sha256_file_hex(&current.join(versioned_artifact_name(
                "grid_timer_sync_server",
                expected_version,
                "exe"
            )))
            .unwrap(),
            expected_server
        );
        assert_eq!(
            before,
            update_rehearsal_data_digest(root),
            "release recovery touched data at {point}"
        );
        if committed {
            update_rehearsal_commit_new_data(root, &expected);
        }
        let after_write = update_rehearsal_data_digest(root);
        recover_or_finalize_existing_transaction(root, &inputs.android_sdk).unwrap();
        recover_or_finalize_existing_transaction(root, &inputs.android_sdk).unwrap();
        assert_eq!(
            after_write,
            update_rehearsal_data_digest(root),
            "finishing recovery lost data at {point}"
        );
        assert!(update_rehearsal_wait(&mut resolve, root, "resolve_after_finish", 60).success());
        assert_eq!(
            sha256_file_hex(&current.join(versioned_artifact_name(
                "grid_timer_sync_server",
                expected_version,
                "exe"
            )))
            .unwrap(),
            expected_server
        );
        retained.verify_unchanged().unwrap();
        assert!(!root
            .join("release_artifacts")
            .join(RELEASE_TRANSACTION_JOURNAL_NAME)
            .exists());
        let record = serde_json::json!({"point":point,"publisherExit":86,"committed":committed,"stableEntryRecovered":true,
            "retainedAndroidUnchanged":true,"dataPreserved":true,"oldWindowsVersion":installed.version,"candidateWindowsVersion":product_identity::WINDOWS_APP_VERSION,"preparedSource":artifacts.build_identity.source_snapshot_sha256,"dataFixtureScope":"genuine candidate-format preservation; migration is separately tested","newDataCommitted":committed,"before":before,"after":after_write});
        fs::write(
            case_dir.join("result.json"),
            serde_json::to_vec_pretty(&record).unwrap(),
        )
        .unwrap();
        results.push(record);
        println!("update rehearsal {point}: stable entry, data and Android retention verified");
    }
    fs::write(
        inputs.evidence.join("results.json"),
        serde_json::to_vec_pretty(&results).unwrap(),
    )
    .unwrap();
}
