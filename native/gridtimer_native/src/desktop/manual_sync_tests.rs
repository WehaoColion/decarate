// v0.0.2 - Cover sync availability, visible early failures and private profile startup.
// v0.0.1 - Verify startup readiness on an explicitly prepared private profile copy.

#[test]
fn manual_sync_card_rejects_unverified_or_busy_workspace() {
    let root = temp_test_dir("manual_sync_availability");
    let mut client = test_client_for_account_scope(
        &root,
        "account-a",
        app_data::default_app_data_json(now_millis()),
    );
    let ctx = workspace_test_context();
    for (ready, ai_pending, busy, expected) in [
        (true, false, false, true),
        (false, false, false, false),
        (true, true, false, false),
        (true, false, true, false),
    ] {
        client.workspace_persistence_ready = ready;
        client.ai_pending = ai_pending;
        client.sync_task = busy.then_some(SyncTaskState {
            kind: SyncTaskKind::Sync,
            phase: SyncTaskPhase::AppData,
            started_at_epoch_millis: now_millis(),
        });
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| client.ui_my_signed_in(ui, busy));
        });
        let (_, enabled) = ctx
            .data(|data| data.get_temp::<(egui::Rect, bool)>(egui::Id::new("manual_sync_button")))
            .unwrap();
        assert_eq!(expected, enabled);
    }
    client.sync_task = None;
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn manual_sync_early_failure_replaces_old_success_without_advancing_timestamp() {
    let root = temp_test_dir("manual_sync_feedback");
    let mut client = test_client_for_account_scope(
        &root,
        "account-a",
        app_data::default_app_data_json(now_millis()),
    );
    let snapshot = client.state_json.clone();
    client.sync.last_sync_at_epoch_millis = 123;
    for (ready, ai_pending) in [(false, false), (true, true)] {
        client.workspace_persistence_ready = ready;
        client.ai_pending = ai_pending;
        client.sync.last_message = "previous-success-sentinel".to_string();
        client.sync_now();
        assert!(client.sync_task.is_none());
        assert_eq!(0, client.task_supervisor.active_count());
        assert_eq!(123, client.sync.last_sync_at_epoch_millis);
        assert_eq!(snapshot, client.state_json);
        assert_ne!("previous-success-sentinel", client.sync.last_message);
        assert_eq!(client.status, client.sync.last_message);
    }
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "requires an isolated profile backup in GRIDTIMER_SYNC_ACCEPTANCE_ROOT"]
fn manual_sync_profile_copy_is_ready_after_startup() {
    let root = PathBuf::from(
        std::env::var_os("GRIDTIMER_SYNC_ACCEPTANCE_ROOT").expect("isolated profile root"),
    );
    let consent: Value =
        serde_json::from_slice(&fs::read(root.join("sync_acceptance_copy.json")).unwrap()).unwrap();
    assert_eq!(consent["purpose"], "isolated_sync_acceptance");
    assert_ne!(root, app_dir());
    assert!(root
        .ancestors()
        .any(|p| p.file_name().is_some_and(|s| s == "GridTimerUpgradeChecks")));
    let marker_path = desktop_state_store_marker_path(&root);
    let raw = fs::read(&marker_path).unwrap();
    let decoded = unprotect_secret_bytes(&raw[DPAPI_SECRET_HEADER.len()..]).unwrap();
    let mut marker: DesktopStateInitializationEvidence = serde_json::from_slice(&decoded).unwrap();
    marker.root_fingerprint = desktop_state_root_fingerprint(&root);
    write_desktop_state_evidence_pair(
        &marker_path,
        &desktop_state_store_external_marker_path(&root),
        &marker,
        true,
        true,
    )
    .unwrap();
    let client = TimerWindowsClient::load_from_root(root.clone(), Arc::new(AtomicBool::new(false)));
    let report = json!({"workspaceReady":client.workspace_persistence_ready,"status":client.status,
        "startupSyncScheduled":client.startup_sync_due_epoch_millis>0,"identityBound":sync_identity_is_bound(&client.sync),
        "snapshotBytes":client.state_json.len(),"notes":client.data.notes.len(),"activeTasks":client.task_supervisor.active_count()});
    fs::write(
        root.join("sync_acceptance_result.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!("SYNC_PROFILE_ACCEPTANCE {report}");
    assert!(client.workspace_persistence_ready, "{}", client.status);
    assert!(client.startup_sync_due_epoch_millis > 0);
}
