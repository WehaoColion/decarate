// v1.1.0.2 Windows - Identical isolated startup probes for baseline and candidate.
// Run the load probe in a fresh test process for each sample. The fixture is
// synthetic, uses a signed-out guest workspace, and has no personal content.
fn startup_pipeline_fixture_root() -> PathBuf {
    let root =
        PathBuf::from(std::env::var_os("GRIDTIMER_STARTUP_BENCHMARK_ROOT").expect(
            "set GRIDTIMER_STARTUP_BENCHMARK_ROOT to an isolated startup_synthetic directory",
        ));
    assert!(
        root.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("startup_synthetic"))
            || (root.file_name().and_then(|name| name.to_str()) == Some("GridTimerClientV2")
                && root
                    .parent()
                    .and_then(Path::file_name)
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("startup_synthetic")))
    );
    root
}

#[test]
#[ignore = "Creates a synthetic ~68 MiB startup journal; run explicitly before paired measurements"]
fn startup_pipeline_create_synthetic_fixture() {
    let root = startup_pipeline_fixture_root();
    assert!(
        !root.exists(),
        "fixture creation never overwrites an existing directory"
    );
    let legacy_root = root.parent().unwrap().join("GridTimerClient");
    assert!(
        !legacy_root.exists(),
        "fixture initialization never reads an existing legacy profile"
    );
    ensure_isolated_app_namespace_with_hooks(&legacy_root, &root, || Ok(()), || Ok(())).unwrap();
    let mut state: Value = serde_json::from_str(&app_data::default_app_data_json(100)).unwrap();
    state["notes"] = Value::Array(
        (0..128)
            .map(|index| {
                desktop_note_save_value(
                    &format!("startup-synthetic-note-{index}"),
                    DesktopNoteKind::Sticky,
                    &format!("Synthetic startup note {index}"),
                    &"Synthetic local recovery paragraph. ".repeat(160),
                    None,
                    100,
                )
            })
            .collect(),
    );
    let state = app_data::sanitize_app_data_json(&state.to_string(), 100).unwrap();
    let mut client = test_client_for_account_scope(
        &root,
        "synthetic-offline",
        app_data::default_app_data_json(100),
    );
    client.sync = DesktopSyncSession::default();
    client.state_path = state_path_for_user(&root, "");
    client.state_json = state;
    client.data = decode_data(&client.state_json);
    client.settings.close_to_tray = false;
    client.save_state().unwrap();
    save_sync_files(&client.sync_path, &client.secrets_path, &client.sync).unwrap();
    client.persist_settings().unwrap();
    // Retention keeps approximately 64 MiB of snapshots, plus SQLite pages and
    // metadata. Stop once that stable budget is reached, rather than benchmark
    // an arbitrarily tiny journal.
    for sequence in 0..72 {
        let mut next: Value = serde_json::from_str(&client.state_json).unwrap();
        next["notes"][0]["title"] = json!(format!("Synthetic snapshot {sequence}"));
        next["notes"][0]["updatedAtEpochMillis"] = json!(200 + sequence);
        client.state_json = app_data::sanitize_app_data_json(&next.to_string(), 300).unwrap();
        client.save_state().unwrap();
        if fs::metadata(desktop_state_store_path(&root)).unwrap().len() >= 64 * 1024 * 1024 {
            break;
        }
    }
    save_sync_files(&client.sync_path, &client.secrets_path, &client.sync).unwrap();
    drop(client);
    let reopened =
        TimerWindowsClient::load_from_root(root.clone(), Arc::new(AtomicBool::new(false)));
    assert!(reopened.workspace_persistence_ready, "{}", reopened.status);
    assert!(reopened.sync.user_id.is_empty() && reopened.sync.token.is_empty());
    assert!(!sync_identity_is_bound(&reopened.sync));
    assert!(!reopened.settings.close_to_tray);
    let journal_bytes = fs::metadata(desktop_state_store_path(&root)).unwrap().len();
    assert!(
        (60 * 1024 * 1024..=80 * 1024 * 1024).contains(&journal_bytes),
        "unexpected fixture size {journal_bytes}"
    );
    println!(
        "STARTUP_FIXTURE {}",
        json!({"journalBytes": journal_bytes, "stateBytes": reopened.state_json.len(), "notes": reopened.data.notes.len()})
    );
}

#[test]
#[ignore = "One isolated ready-time measurement; start a fresh process for each sample"]
fn startup_pipeline_load_synthetic_fixture() {
    let root = startup_pipeline_fixture_root();
    assert!(root.is_dir());
    let started = Instant::now();
    let client = TimerWindowsClient::load_from_root(root.clone(), Arc::new(AtomicBool::new(false)));
    let elapsed = started.elapsed();
    assert!(client.workspace_persistence_ready, "{}", client.status);
    assert_eq!(client.data.notes.len(), 128);
    println!(
        "STARTUP_PIPELINE_BENCHMARK {}",
        json!({
            "profile": "release; cold process caches; synthetic journal; full verified workspace ready; excludes font/window creation",
            "readyMillis": elapsed.as_secs_f64() * 1000.0,
            "journalBytes": fs::metadata(desktop_state_store_path(&root)).unwrap().len(),
            "stateBytes": client.state_json.len(),
            "snapshotSha256": format!("{:x}", Sha256::digest(client.state_json.as_bytes())),
        })
    );
}
