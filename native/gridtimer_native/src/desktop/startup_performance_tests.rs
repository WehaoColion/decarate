// v1.0.2.1 - Guard startup checkpoint durability and measure verified reopen work.

include!("startup_pipeline_benchmark.rs");

fn startup_test_fonts() -> (mpsc::Sender<DesktopUiFontBytes>, DesktopStartupFonts) {
    let (sender, receiver) = mpsc::channel();
    (
        sender,
        DesktopStartupFonts {
            receiver,
            worker: None,
            cancelled: Arc::new(AtomicBool::new(false)),
            stage: DesktopStartupFontStage::Pending,
        },
    )
}

fn startup_test_shell(
    receiver: mpsc::Receiver<Result<LoadedWorkspace, StartupFailure>>,
) -> DesktopStartupApp {
    let (_, mut fonts) = startup_test_fonts();
    fonts.stage = DesktopStartupFontStage::Ready;
    DesktopStartupApp {
        state: DesktopStartupState::Loading,
        control: DesktopStartupControl::default(),
        receiver,
        worker: None,
        fonts,
        pending_workspace: None,
        activation_requested: Arc::new(AtomicBool::new(false)),
        close_requested: false,
        #[cfg(target_os = "windows")]
        window_handle: None,
        started: Instant::now(),
        first_frame_recorded: false,
        first_ready_frame_recorded: false,
    }
}

#[test]
fn startup_early_fonts_preserve_the_first_frame_and_cannot_replace_workspace_verification() {
    let (_sender, receiver) = mpsc::channel();
    let mut shell = startup_test_shell(receiver);
    let (font_sender, fonts) = startup_test_fonts();
    shell.fonts = fonts;
    assert!(font_sender.send(DesktopUiFontBytes::default()).is_ok());
    let context = egui::Context::default();
    let _ = context.run(egui::RawInput::default(), |ctx| shell.poll_completion(ctx));
    assert!(matches!(
        shell.fonts.stage,
        DesktopStartupFontStage::Pending
    ));
    assert!(matches!(shell.state, DesktopStartupState::Loading));
    shell.first_frame_recorded = true;
    let _ = context.run(egui::RawInput::default(), |ctx| shell.poll_completion(ctx));
    assert!(matches!(
        shell.fonts.stage,
        DesktopStartupFontStage::Applied(_)
    ));
    assert!(matches!(shell.state, DesktopStartupState::Loading));
    let _ = context.run(egui::RawInput::default(), |ctx| shell.poll_completion(ctx));
    assert!(shell.fonts.is_ready());
    assert!(matches!(shell.state, DesktopStartupState::Loading));
    assert!(shell.pending_workspace.is_none());
}

#[test]
fn startup_verified_receipt_waits_for_fonts_and_the_next_frame() {
    let root = temp_test_dir("startup_late_fonts");
    let client = startup_performance_client(&root, app_data::default_app_data_json(100));
    drop(client);
    let loaded = load_desktop_workspace(root.clone(), &DesktopStartupControl::default()).unwrap();
    assert!(loaded.workspace_persistence_ready);
    let (sender, receiver) = mpsc::channel();
    let mut shell = startup_test_shell(receiver);
    let (font_sender, fonts) = startup_test_fonts();
    shell.fonts = fonts;
    assert!(sender.send(Ok(loaded)).is_ok());
    let context = egui::Context::default();

    let _ = context.run(egui::RawInput::default(), |ctx| shell.poll_completion(ctx));
    assert!(matches!(shell.state, DesktopStartupState::Loading));
    assert!(shell.pending_workspace.is_some());
    shell.first_frame_recorded = true;
    let _ = context.run(egui::RawInput::default(), |ctx| shell.poll_completion(ctx));
    assert!(matches!(shell.state, DesktopStartupState::Loading));
    assert!(!shell.fonts.is_ready());

    assert!(font_sender.send(DesktopUiFontBytes::default()).is_ok());
    let _ = context.run(egui::RawInput::default(), |ctx| shell.poll_completion(ctx));
    assert!(matches!(
        shell.fonts.stage,
        DesktopStartupFontStage::Applied(_)
    ));
    assert!(matches!(shell.state, DesktopStartupState::Loading));
    assert!(shell.pending_workspace.is_some());
    let _ = context.run(egui::RawInput::default(), |ctx| shell.poll_completion(ctx));
    assert!(shell.fonts.is_ready());
    let DesktopStartupState::Ready(client) = &shell.state else {
        panic!("both verified workspace and applied fonts must enable the client");
    };
    assert!(client.workspace_persistence_ready);
    assert!(shell.pending_workspace.is_none());
    drop(shell);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn startup_closing_with_late_fonts_discards_the_verified_receipt_and_owned_payload() {
    let root = temp_test_dir("startup_close_late_fonts");
    let client = startup_performance_client(&root, app_data::default_app_data_json(100));
    drop(client);
    let loaded = load_desktop_workspace(root.clone(), &DesktopStartupControl::default()).unwrap();
    let (sender, receiver) = mpsc::channel();
    let mut shell = startup_test_shell(receiver);
    let (font_sender, mut fonts) = startup_test_fonts();
    let cancelled = Arc::clone(&fonts.cancelled);
    let (release_sender, release_receiver) = mpsc::channel();
    let cancellation_observed = Arc::new(AtomicBool::new(false));
    let worker_observed = Arc::clone(&cancellation_observed);
    fonts.worker = Some(thread::spawn(move || {
        let _ = release_receiver.recv();
        worker_observed.store(
            cancelled.load(AtomicOrdering::Acquire),
            AtomicOrdering::Release,
        );
        // Model bytes finishing an OS read just as the native close arrives.
        let _ = font_sender.send(DesktopUiFontBytes::default());
    }));
    shell.fonts = fonts;
    shell.first_frame_recorded = true;
    assert!(sender.send(Ok(loaded)).is_ok());
    let context = egui::Context::default();
    let _ = context.run(egui::RawInput::default(), |ctx| shell.poll_completion(ctx));
    let receipt_waited = shell.pending_workspace.is_some();
    let mut input = egui::RawInput::default();
    input
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .events
        .push(egui::ViewportEvent::Close);
    let output = context.run(input, |ctx| shell.poll_completion(ctx));
    // Release the deterministic fake read before any assertion may unwind.
    let _ = release_sender.send(());
    shell.fonts.cancel_and_join();
    assert!(receipt_waited);
    assert!(cancellation_observed.load(AtomicOrdering::Acquire));
    assert!(output.viewport_output[&egui::ViewportId::ROOT]
        .commands
        .iter()
        .any(|command| matches!(command, egui::ViewportCommand::CancelClose)));
    assert!(shell.close_requested);
    assert!(shell.pending_workspace.is_none());
    assert!(matches!(
        shell.state,
        DesktopStartupState::Recovery {
            workspace: None,
            ..
        }
    ));
    assert!(!shell.fonts.is_ready());
    assert!(shell.fonts.worker.is_none());
    assert!(shell.fonts.receiver.try_recv().is_err());
    drop(shell);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn startup_cancel_before_loading_cannot_create_a_workspace() {
    let root = temp_test_dir("startup_cancel_before_load");
    let control = DesktopStartupControl::default();
    control.cancel();
    assert!(load_desktop_workspace(root.clone(), &control).is_err());
    assert!(!desktop_state_store_path(&root).exists());
    assert!(!root.join("timer_state.json").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn startup_cancelled_receipt_never_enables_editing_or_sync() {
    let root = temp_test_dir("startup_cancelled_receipt");
    let client = startup_performance_client(&root, app_data::default_app_data_json(100));
    drop(client);
    let loaded = load_desktop_workspace(root.clone(), &DesktopStartupControl::default()).unwrap();
    assert!(loaded.workspace_persistence_ready);
    let (sender, receiver) = mpsc::channel();
    let mut shell = startup_test_shell(receiver);
    shell.control.cancel();
    assert!(sender.send(Ok(loaded)).is_ok());
    shell.poll_completion(&egui::Context::default());
    assert!(matches!(
        shell.state,
        DesktopStartupState::Recovery {
            workspace: None,
            ..
        }
    ));
    assert!(!matches!(shell.state, DesktopStartupState::Ready(_)));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn startup_close_and_completed_load_in_same_frame_cannot_enter_ready() {
    let root = temp_test_dir("startup_same_frame_close");
    let client = startup_performance_client(&root, app_data::default_app_data_json(100));
    drop(client);
    let loaded = load_desktop_workspace(root.clone(), &DesktopStartupControl::default()).unwrap();
    assert!(loaded.workspace_persistence_ready);
    let (sender, receiver) = mpsc::channel();
    let mut shell = startup_test_shell(receiver);
    assert!(sender.send(Ok(loaded)).is_ok());

    let mut input = egui::RawInput::default();
    input
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .events
        .push(egui::ViewportEvent::Close);
    let context = egui::Context::default();
    let _ = context.run(input, |ctx| shell.poll_completion(ctx));

    assert!(shell.close_requested);
    assert!(shell.control.cancelled.load(AtomicOrdering::Acquire));
    assert!(matches!(
        shell.state,
        DesktopStartupState::Recovery {
            workspace: None,
            ..
        }
    ));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn startup_unverified_receipt_requires_explicit_read_only_recovery() {
    let root = temp_test_dir("startup_read_only_receipt");
    let client = startup_performance_client(&root, app_data::default_app_data_json(100));
    drop(client);
    let mut loaded =
        load_desktop_workspace(root.clone(), &DesktopStartupControl::default()).unwrap();
    loaded.workspace_persistence_ready = false;
    loaded.startup_sync_due_epoch_millis = 0;
    let (sender, receiver) = mpsc::channel();
    let mut shell = startup_test_shell(receiver);
    assert!(sender.send(Ok(loaded)).is_ok());
    shell.poll_completion(&egui::Context::default());
    assert!(matches!(
        shell.state,
        DesktopStartupState::Recovery {
            workspace: Some(_),
            ..
        }
    ));
    fs::remove_dir_all(root).unwrap();
}

fn startup_performance_client(root: &Path, state: String) -> TimerWindowsClient {
    let mut client = test_client_for_account_scope(root, "startup-account", state);
    let server = client.sync.server_instance_id.clone();
    let namespace = client.sync.account_namespace.clone();
    let user = client.sync.user_id.clone();
    ensure_workspace_checkpoint(&mut client.sync, &server, &namespace, &user);
    client.save_state().unwrap();
    save_sync_files(&client.sync_path, &client.secrets_path, &client.sync).unwrap();
    client
}

#[test]
fn startup_reuses_only_an_exact_nonempty_checkpoint() {
    let root = temp_test_dir("startup_checkpoint_guard");
    let client = startup_performance_client(&root, app_data::default_app_data_json(100));
    let protected = protect_workspace_journal_sync_state(&client.sync).unwrap();
    assert!(!startup_workspace_requires_initialization_commit(
        &protected,
        &client.sync,
    ));
    assert!(startup_workspace_requires_initialization_commit(
        &[],
        &client.sync,
    ));
    assert!(startup_workspace_requires_initialization_commit(
        b"invalid protected checkpoint",
        &client.sync,
    ));
    let mut advanced = client.sync.clone();
    advanced.acknowledged_generation = 7;
    assert!(startup_workspace_requires_initialization_commit(
        &protected, &advanced,
    ));
    let mut other_account = client.sync.clone();
    other_account.user_id = "another-account".into();
    other_account.account_namespace = test_account_namespace(&other_account.user_id);
    assert!(startup_workspace_requires_initialization_commit(
        &protected,
        &other_account,
    ));
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn startup_unchanged_account_keeps_the_existing_workspace_mirror() {
    let root = temp_test_dir("startup_unchanged_mirror");
    let client = startup_performance_client(&root, app_data::default_app_data_json(100));
    let path = client.state_path.clone();
    let expected = client.state_json.clone();
    drop(client);
    let known_modified = UNIX_EPOCH + Duration::from_secs(946_684_800);
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(known_modified))
        .unwrap();
    let before_modified = fs::metadata(&path).unwrap().modified().unwrap();
    let reopened =
        TimerWindowsClient::load_from_root(root.clone(), Arc::new(AtomicBool::new(false)));
    assert!(reopened.workspace_persistence_ready, "{}", reopened.status);
    assert_eq!(expected, reopened.state_json);
    assert_eq!(
        before_modified,
        fs::metadata(&path).unwrap().modified().unwrap(),
        "an unchanged, verified workspace must not be written again at startup",
    );
    drop(reopened);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn startup_advanced_checkpoint_is_committed_before_writes_are_enabled() {
    let root = temp_test_dir("startup_advanced_checkpoint");
    let mut client = startup_performance_client(&root, app_data::default_app_data_json(100));
    client.sync.acknowledged_generation = 7;
    save_sync_files(&client.sync_path, &client.secrets_path, &client.sync).unwrap();
    drop(client);
    let reopened =
        TimerWindowsClient::load_from_root(root.clone(), Arc::new(AtomicBool::new(false)));
    assert!(reopened.workspace_persistence_ready, "{}", reopened.status);
    assert_eq!(7, reopened.sync.acknowledged_generation);
    let owner = desktop_state_owner(
        &reopened.sync.server_instance_id,
        &reopened.sync.account_namespace,
        &reopened.sync.user_id,
    )
    .unwrap();
    let (_, protected) =
        load_state_snapshot_with_store(&root, &reopened.state_path, &owner, now_millis()).unwrap();
    assert_eq!(
        workspace_journal_sync_state(&reopened.sync).unwrap(),
        decode_workspace_journal_sync_state(&protected).unwrap(),
        "startup must commit a changed checkpoint with the verified snapshot",
    );
    drop(reopened);
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "Explicit isolated startup benchmark; no personal workspace is read"]
fn startup_verified_reopen_benchmark() {
    let root = temp_test_dir("startup_verified_reopen_benchmark");
    let mut state: Value = serde_json::from_str(&app_data::default_app_data_json(100)).unwrap();
    state["notes"] = Value::Array(
        (0..128)
            .map(|index| {
                desktop_note_save_value(
                    &format!("startup-note-{index}"),
                    DesktopNoteKind::Sticky,
                    &format!("Startup note {index}"),
                    &"isolated sample content ".repeat(64),
                    None,
                    100,
                )
            })
            .collect(),
    );
    let state = app_data::sanitize_app_data_json(&state.to_string(), 100).unwrap();
    let mut client = startup_performance_client(&root, state);
    for sequence in 0..7 {
        let mut next: Value = serde_json::from_str(&client.state_json).unwrap();
        next["notes"][0]["title"] = json!(format!("Snapshot {sequence}"));
        next["notes"][0]["updatedAtEpochMillis"] = json!(200 + sequence);
        client.state_json = app_data::sanitize_app_data_json(&next.to_string(), 300).unwrap();
        client.save_state().unwrap();
    }
    let owner = desktop_state_owner(
        &client.sync.server_instance_id,
        &client.sync.account_namespace,
        &client.sync.user_id,
    )
    .unwrap();
    let mut before_ms = Vec::new();
    let mut after_ms = Vec::new();
    for _ in 0..4 {
        let start = Instant::now();
        let (loaded, _) =
            load_state_snapshot_with_store(&root, &client.state_path, &owner, now_millis())
                .unwrap();
        save_state_snapshot_with_store(
            &root,
            &client.state_path,
            &owner,
            &loaded.value,
            &client.sync,
            now_millis(),
            "workspace_initialization",
        )
        .unwrap();
        before_ms.push(start.elapsed().as_secs_f64() * 1000.0);

        let start = Instant::now();
        let (loaded, protected) =
            load_state_snapshot_with_store(&root, &client.state_path, &owner, now_millis())
                .unwrap();
        assert!(!startup_workspace_requires_initialization_commit(
            &protected,
            &client.sync,
        ));
        assert_eq!(loaded.value, client.state_json);
        after_ms.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    println!(
        "STARTUP_REOPEN_BENCHMARK {}",
        json!({
            "profile": "isolated verified reopen stage; excludes window/font initialization",
            "stateBytes": client.state_json.len(),
            "journalBytes": fs::metadata(desktop_state_store_path(&root)).unwrap().len(),
            "notes": 128,
            "historySnapshots": 8,
            "legacyUnconditionalCommitMillis": before_ms,
            "reuseVerifiedPairMillis": after_ms,
        }),
    );
    drop(client);
    fs::remove_dir_all(root).unwrap();
}
