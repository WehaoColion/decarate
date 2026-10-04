// v1.1.0.3 Windows - Verify serialized sync receipts and timer intent boundaries.

fn sync_apply_test_task(client: &TimerWindowsClient, remote: String) -> SyncTaskResult {
    let checkpoint = workspace_checkpoint_for_account(
        &client.sync,
        &client.sync.server_instance_id,
        &client.sync.account_namespace,
        &client.sync.user_id,
    );
    let result = sync_core::SyncClientResult {
        ok: true,
        current_committed: true,
        user_id: client.sync.user_id.clone(),
        token_id: client.sync.token_id.clone(),
        server_instance_id: client.sync.server_instance_id.clone(),
        account_namespace: client.sync.account_namespace.clone(),
        current_generation: client.sync.acknowledged_generation,
        workspace_id: checkpoint.workspace_id,
        workspace_proof: checkpoint.workspace_capability,
        app_data_json: Some(remote),
        mode: "merged".into(),
        ..Default::default()
    };
    SyncTaskResult {
        kind: SyncTaskKind::Sync,
        background_job_id: client.active_background_job_id.clone(),
        results: vec![SyncTaskStepResult {
            raw: serde_json::to_string(&result).unwrap(),
            keep_password: false,
            identity: bound_result_expectation(client),
        }],
    }
}

fn sync_apply_test_input(client: &TimerWindowsClient, remote: String) -> DesktopSyncApplyInput {
    DesktopSyncApplyInput {
        workspace: client.ai_workspace_identity(),
        version: client.data_version,
        state: client.state_json.clone(),
        sync: client.sync.clone(),
        sync_path: client.sync_path.clone(),
        secrets_path: client.secrets_path.clone(),
        task: sync_apply_test_task(client, remote),
    }
}

fn sync_apply_test_deliver(
    client: &mut TimerWindowsClient,
    input: DesktopSyncApplyInput,
    saved: DesktopSyncApplySaved,
) {
    let (sender, receiver) = mpsc::channel();
    client.sync_apply.receiver = Some(receiver);
    sender
        .send(DesktopSyncApplyCompletion {
            workspace: input.workspace,
            version: input.version,
            task: input.task,
            outcome: Ok(Some(saved)),
        })
        .unwrap();
    client.poll_ordinary_sync_apply();
}

#[test]
fn sync_apply_idle_poll_does_not_bypass_draft_debounce() {
    let root = temp_test_dir("sync_apply_idle_draft");
    let mut client = test_client_for_account_scope(
        &root,
        "account-a",
        app_state_with_note("n", "before", "body", None),
    );
    client.select_note_by_id("n");
    client.note_title_draft = "unsaved".into();
    client.mark_note_dirty();
    client.sync_apply.frame_context = Some(egui::Context::default());
    let due = client.note_save_due_epoch_millis;
    client.poll_sync_result();
    client.poll_note_media_sync_result();
    assert!(client.note_dirty);
    assert!(!client.persistence.pending());
    assert_eq!(client.note_save_due_epoch_millis, due);
    assert_eq!(client.data.notes[0].title, "before");
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn media_apply_response_waits_for_dirty_draft_without_synchronous_flush() {
    for phase in [SyncTaskPhase::NoteMediaPreflight, SyncTaskPhase::NoteMedia] {
        let root = temp_test_dir("media_apply_dirty_response_barrier");
        let mut client = test_client_for_account_scope(
            &root,
            "account-a",
            app_state_with_note("n", "before", "body", None),
        );
        client.save_state().unwrap();
        client.select_note_by_id("n");
        let task = SyncTaskState {
            kind: SyncTaskKind::Sync,
            phase,
            started_at_epoch_millis: now_millis(),
        };
        client.sync_task = Some(task);
        let background_job_id = client
            .stage_background_job(BackgroundJobKind::MediaSync)
            .unwrap();
        client.note_title_draft = "draft received during media sync".into();
        client.mark_note_dirty();
        client.sync_apply.frame_context = Some(egui::Context::default());
        let mut gate = rusqlite::Connection::open(root.join(DESKTOP_STATE_STORE_FILE)).unwrap();
        let transaction = gate
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        sender
            .send(NoteMediaSyncTaskResult {
                task,
                background_job_id: Some(background_job_id),
                origin_workspace: client.ai_workspace_identity(),
                base_status: String::new(),
                result: Ok(Default::default()),
            })
            .unwrap();
        client.note_media_sync_result_rx = Some(receiver);
        client.poll_note_media_sync_result();
        assert!(client.note_media_sync_result_rx.is_some());
        assert!(
            client.persistence.pending(),
            "only the save worker may wait for the journal"
        );
        assert!(client.note_dirty);
        assert_eq!(client.data.notes[0].title, "before");
        transaction.rollback().unwrap();
        drop(gate);
        drain_draft_writer(&mut client);
        assert_eq!(
            client.data.notes[0].title,
            "draft received during media sync"
        );
        assert_eq!(
            decode_data(&fs::read_to_string(&client.state_path).unwrap()).notes[0].title,
            "draft received during media sync"
        );
        assert!(
            client.note_media_sync_result_rx.is_some(),
            "response remains available after the durable draft"
        );
        client.cancel_active_sync();
        drop(client);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn sync_apply_timer_confirmation_gets_a_frame_before_waiting_sync_response() {
    let root = temp_test_dir("sync_apply_timer_priority_frame");
    let mut client = test_client_for_account_scope(
        &root,
        "account-a",
        app_data::default_app_data_json(now_millis()),
    );
    client.settings.timer_bell_enabled = false;
    let ctx = egui::Context::default();
    let slot = client.selected_slot().unwrap();
    client.toggle_slot(&slot, &ctx);
    wait_for_timer_action(&mut client);
    let confirmed = client.state_json.clone();
    let (sender, receiver) = mpsc::channel();
    sender
        .send(sync_apply_test_task(&client, confirmed.clone()))
        .unwrap();
    client.sync_result_rx = Some(receiver);
    client.pump_workspace_frame(&ctx);
    assert!(
        client.sync_result_rx.is_some(),
        "confirmation frame must retain the unrelated response"
    );
    assert!(!client.external_workspace_save_pending());
    assert_eq!(client.state_json, confirmed);
    assert!(client.data.slots[0].running_since_epoch_millis.is_some());
    // The full production frame also starts the independent report-index
    // loader. Quiesce those owned tasks before removing their synthetic root.
    client.cancel_active_sync();
    client.task_supervisor.begin_shutdown();
    let timeout = Instant::now() + Duration::from_secs(10);
    while client.task_supervisor.active_count() > 0 {
        assert!(
            Instant::now() < timeout,
            "fixture background task did not exit"
        );
        client.task_supervisor.reap_finished();
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sync_apply_success_releases_one_timer_and_deferred_finish_never_replays_snapshot() {
    let root = temp_test_dir("sync_apply_serial_timer");
    let mut client = test_client_for_account_scope(
        &root,
        "account-a",
        app_data::default_app_data_json(now_millis()),
    );
    client.settings.timer_bell_enabled = false;
    client.save_state().unwrap();
    client.sync_task = Some(SyncTaskState {
        kind: SyncTaskKind::Sync,
        phase: SyncTaskPhase::AppData,
        started_at_epoch_millis: now_millis(),
    });
    let remote =
        app_data::set_theme_mode_app_data_json(&client.state_json, 2, now_millis()).unwrap();
    let input = sync_apply_test_input(&client, remote);
    let saved = prepare_ordinary_sync_commit(&input, &CancellationToken::new())
        .unwrap()
        .unwrap();
    let (sender, receiver) = mpsc::channel();
    client.sync_apply.receiver = Some(receiver);
    let ctx = egui::Context::default();
    let slot = client.selected_slot().unwrap();
    client.toggle_slot(&slot, &ctx);
    let requested = client.persistence.pending_timer_action.clone().unwrap();
    assert!(
        !client.persistence.pending(),
        "external writer owns the commit barrier"
    );
    assert!(client.draft_write_barrier().is_err());
    // A close request stops new network work, but the previously accepted
    // commit and timer intention must still be acknowledged and written.
    client.persistence.waiting_to_close = true;
    sender
        .send(DesktopSyncApplyCompletion {
            workspace: input.workspace,
            version: input.version,
            task: input.task,
            outcome: Ok(Some(saved)),
        })
        .unwrap();
    client.poll_ordinary_sync_apply();
    assert!(client.persistence.pending());
    assert_eq!(
        client.persistence.pending_timer_action.as_ref().unwrap().id,
        requested.id
    );
    wait_for_timer_action(&mut client);
    assert_eq!(
        client.data.slots[0].running_since_epoch_millis,
        Some(requested.requested_at_epoch_millis)
    );
    assert_eq!(client.data.theme_mode, "DARK");
    assert!(client.data.sessions.is_empty());
    let confirmed = client.state_json.clone();
    assert!(matches!(
        client.shutdown_state,
        ClientShutdownState::Running
    ));
    assert!(client.background_work_is_allowed());
    client.poll_deferred_sync_finish();
    assert_eq!(client.state_json, confirmed);
    assert!(client.sync_apply.completed_sync.is_none());
    assert!(client.sync_task.is_none());
    assert!(client.sync_result_rx.is_none());
    assert!(client.note_media_sync_result_rx.is_none());
    assert!(!client.external_workspace_save_pending());
    assert!(client.active_background_job_id.is_none());
    assert_eq!(fs::read_to_string(&client.state_path).unwrap(), confirmed);
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn closing_wait_receives_report_receipts_but_starts_no_network_or_cleanup() {
    let root = temp_test_dir("closing_wait_receipts_only");
    let mut client = test_client_for_account_scope(
        &root,
        "account-a",
        app_data::default_app_data_json(now_millis()),
    );
    client.persistence.waiting_to_close = true;
    assert!(client.background_work_is_allowed());
    client.sync.last_sync_at_epoch_millis = 10;
    client.sync.account_generation_checkpoints.insert(
        client.sync.account_namespace.clone(),
        AccountGenerationCheckpoint {
            generation: 0,
            workspace_id: "workspace".into(),
            workspace_capability: "capability".into(),
        },
    );
    let scope = client.legal_scope();
    client.desktop_ui.legal_risk.scope = scope.clone();
    client.desktop_ui.legal_risk.loaded_scope = scope.clone();
    client.desktop_ui.legal_risk.sync_normal_seen_at = 10;
    let (sender, receiver) = mpsc::channel();
    client.desktop_ui.legal_risk.sync_rx = Some(receiver);
    sender
        .send(DesktopLegalSyncOutcome {
            scope,
            revision: client.desktop_ui.legal_risk.sync_revision,
            result: Ok(vec![DesktopLegalReportMeta {
                id: "committed-report".into(),
                created_at_epoch_millis: 1,
                sha256: "committed-sha".into(),
                completed: true,
                finding_count: 0,
                source_workspace_id: Some("workspace".into()),
                size_bytes: Some(1),
            }]),
        })
        .unwrap();
    let ctx = egui::Context::default();
    client.poll_desktop_legal_tasks(&ctx);
    assert!(client.desktop_ui.legal_risk.sync_rx.is_none());
    assert_eq!(client.desktop_ui.legal_risk.reports.len(), 1);
    assert_eq!(
        client.desktop_ui.legal_risk.reports[0].id,
        "committed-report"
    );

    // A pending second pass must wait for a future launch, while the receipt
    // above remains visible to the normal global task collector.
    client.desktop_ui.legal_risk.sync_pending = true;
    client.poll_desktop_legal_tasks(&ctx);
    assert!(client.desktop_ui.legal_risk.sync_rx.is_none());
    assert_eq!(client.desktop_ui.legal_risk.sync_last_attempt_at, 0);
    client.start_sync_task_checked(SyncTaskKind::Health, false);
    client.start_note_media_sync(
        SyncTaskKind::Sync,
        gridtimer_native::desktop_note_media_sync::DesktopNoteMediaSyncDirection::Bidirectional,
        SyncTaskPhase::NoteMedia,
    );
    client.start_synced_media_cleanup();
    assert!(client.sync_task.is_none());
    assert!(client.sync_result_rx.is_none());
    assert!(client.note_media_sync_result_rx.is_none());
    assert!(!client.external_workspace_save_pending());
    assert_eq!(client.task_supervisor.active_count(), 0);
    assert!(client.desktop_ui.legal_risk.sync_pending);
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sync_apply_disconnect_fails_closed_and_cancels_waiting_timer() {
    for media in [false, true] {
        let root = temp_test_dir("sync_apply_disconnected");
        let mut client = test_client_for_account_scope(
            &root,
            "account-a",
            app_data::default_app_data_json(now_millis()),
        );
        client.settings.timer_bell_enabled = false;
        let before = client.state_json.clone();
        if media {
            let (sender, receiver) = mpsc::channel::<DesktopMediaApplyCompletion>();
            client.sync_apply.media_receiver = Some(receiver);
            drop(sender);
        } else {
            let (sender, receiver) = mpsc::channel::<DesktopSyncApplyCompletion>();
            client.sync_apply.receiver = Some(receiver);
            drop(sender);
        }
        let ctx = egui::Context::default();
        let slot = client.selected_slot().unwrap();
        client.toggle_slot(&slot, &ctx);
        assert!(client.persistence.pending_timer_action.is_some());
        assert!(!client.persistence.pending());
        if media {
            client.poll_media_result_apply();
        } else {
            client.poll_ordinary_sync_apply();
        }
        assert!(!client.workspace_persistence_ready);
        assert!(client.persistence.pending_timer_action.is_none());
        assert_eq!(client.state_json, before);
        client.toggle_slot(&slot, &ctx);
        assert!(client.persistence.pending_timer_action.is_none());
        assert!(!client.persistence.pending());
        drop(client);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn sync_apply_stale_receipt_is_never_applied_or_allowed_to_release_timer() {
    for changed_workspace in [false, true] {
        let root = temp_test_dir("sync_apply_stale_receipt");
        let mut client = test_client_for_account_scope(
            &root,
            "account-a",
            app_data::default_app_data_json(now_millis()),
        );
        client.settings.timer_bell_enabled = false;
        let before = client.state_json.clone();
        let mut workspace = client.ai_workspace_identity();
        let version = client.data_version;
        let task = sync_apply_test_task(&client, before.clone());
        let (sender, receiver) = mpsc::channel();
        client.sync_apply.receiver = Some(receiver);
        let ctx = egui::Context::default();
        let slot = client.selected_slot().unwrap();
        client.toggle_slot(&slot, &ctx);
        if changed_workspace {
            workspace.user_id = "different-account".into();
        } else {
            client.data_version += 1;
        }
        sender
            .send(DesktopSyncApplyCompletion {
                workspace,
                version,
                task,
                outcome: Ok(None),
            })
            .unwrap();
        client.poll_ordinary_sync_apply();
        assert!(!client.workspace_persistence_ready);
        assert!(client.persistence.pending_timer_action.is_none());
        assert!(client.sync_apply.completed_sync.is_none());
        assert!(!client.persistence.pending());
        assert_eq!(client.state_json, before);
        drop(client);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn sync_apply_same_workspace_rejects_changed_route_token_or_generation() {
    for changed in ["route", "token", "generation"] {
        let root = temp_test_dir("sync_apply_changed_credential_scope");
        let mut client = test_client_for_account_scope(
            &root,
            "account-a",
            app_data::default_app_data_json(now_millis()),
        );
        client.save_state().unwrap();
        let original_workspace = client.ai_workspace_identity();
        let before = client.state_json.clone();
        let input = sync_apply_test_input(&client, before.clone());
        let saved = prepare_ordinary_sync_commit(&input, &CancellationToken::new())
            .unwrap()
            .unwrap();
        match changed {
            "route" => client.sync.server_url = "https://different-sync.example.test".into(),
            "token" => {
                client.sync.token = "synthetic-replacement-token".into();
                client.sync.token_id = sync_core::token_identifier(&client.sync.token);
            }
            "generation" => client.sync.acknowledged_generation += 1,
            _ => unreachable!(),
        }
        let current_route = client.sync.server_url.clone();
        let current_token = client.sync.token_id.clone();
        let current_generation = client.sync.acknowledged_generation;
        assert_eq!(client.ai_workspace_identity(), original_workspace);
        sync_apply_test_deliver(&mut client, input, saved);
        assert!(
            !client.workspace_persistence_ready,
            "{changed} must invalidate the old receipt"
        );
        assert!(client.sync_apply.completed_sync.is_none());
        assert_eq!(client.state_json, before);
        assert_eq!(client.sync.server_url, current_route);
        assert_eq!(client.sync.token_id, current_token);
        assert_eq!(client.sync.acknowledged_generation, current_generation);
        drop(client);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn media_apply_same_workspace_rejects_changed_route_token_or_generation() {
    for changed in ["route", "token", "generation"] {
        let root = temp_test_dir("media_apply_changed_credential_scope");
        let mut client = test_client_for_account_scope(
            &root,
            "account-a",
            app_data::default_app_data_json(now_millis()),
        );
        let workspace = client.ai_workspace_identity();
        let version = client.data_version;
        let identity = media_apply_identity(&client.sync);
        let before = client.state_json.clone();
        let task = NoteMediaSyncTaskResult {
            task: SyncTaskState {
                kind: SyncTaskKind::Sync,
                phase: SyncTaskPhase::NoteMedia,
                started_at_epoch_millis: now_millis(),
            },
            background_job_id: None,
            origin_workspace: workspace.clone(),
            base_status: String::new(),
            result: Ok(Default::default()),
        };
        match changed {
            "route" => client.sync.server_url = "https://different-sync.example.test".into(),
            "token" => {
                client.sync.token = "synthetic-replacement-token".into();
                client.sync.token_id = sync_core::token_identifier(&client.sync.token);
            }
            "generation" => client.sync.acknowledged_generation += 1,
            _ => unreachable!(),
        }
        assert_eq!(client.ai_workspace_identity(), workspace);
        let (sender, receiver) = mpsc::channel();
        client.sync_apply.media_receiver = Some(receiver);
        sender
            .send(DesktopMediaApplyCompletion {
                workspace,
                version,
                identity,
                task,
                outcome: Ok(None),
            })
            .unwrap();
        client.poll_media_result_apply();
        assert!(
            !client.workspace_persistence_ready,
            "{changed} must invalidate the old media receipt"
        );
        assert!(client.sync_apply.completed_media.is_none());
        assert_eq!(client.state_json, before);
        drop(client);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn media_apply_scope_accepts_device_and_ai_drafts_without_overwriting_them() {
    let root = temp_test_dir("media_apply_preserve_settings");
    let mut client = test_client_for_account_scope(
        &root,
        "account-a",
        app_state_with_note("n", "before", "body", None),
    );
    let workspace = client.ai_workspace_identity();
    let version = client.data_version;
    let identity = media_apply_identity(&client.sync);
    client.select_note_by_id("n");
    client.note_title_draft = "unsaved note title".into();
    client.mark_note_dirty();
    client.sync.device_name = "new device name".into();
    client.sync.ai_model = "new-model".into();
    client.sync.ai_api_key = "synthetic-new-key".into();
    client.mark_sync_dirty();
    let task = NoteMediaSyncTaskResult {
        task: SyncTaskState {
            kind: SyncTaskKind::Sync,
            phase: SyncTaskPhase::NoteMedia,
            started_at_epoch_millis: now_millis(),
        },
        background_job_id: None,
        origin_workspace: workspace.clone(),
        base_status: String::new(),
        result: Ok(Default::default()),
    };
    let (sender, receiver) = mpsc::channel();
    client.sync_apply.media_receiver = Some(receiver);
    sender
        .send(DesktopMediaApplyCompletion {
            workspace,
            version,
            identity,
            task,
            outcome: Ok(None),
        })
        .unwrap();
    client.poll_media_result_apply();
    assert!(client.workspace_persistence_ready);
    assert!(client.sync_apply.completed_media.is_some());
    assert!(client.note_dirty);
    assert_eq!(client.note_title_draft, "unsaved note title");
    assert_eq!(client.sync.device_name, "new device name");
    assert_eq!(client.sync.ai_model, "new-model");
    assert_eq!(client.sync.ai_api_key, "synthetic-new-key");
    assert!(client.sync_dirty);
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sync_apply_committed_receipt_keeps_newer_device_and_ai_settings() {
    let root = temp_test_dir("sync_apply_preserve_settings");
    let mut client = test_client_for_account_scope(
        &root,
        "account-a",
        app_data::default_app_data_json(now_millis()),
    );
    client.save_state().unwrap();
    let input = sync_apply_test_input(&client, client.state_json.clone());
    let saved = prepare_ordinary_sync_commit(&input, &CancellationToken::new())
        .unwrap()
        .unwrap();
    client.sync.device_name = "new device choice".into();
    client.sync.ai_base_url = "https://new-model.example.test/v1".into();
    client.sync.ai_model = "new-model".into();
    client.sync.ai_api_key = "synthetic-new-key".into();
    client.mark_sync_dirty();
    sync_apply_test_deliver(&mut client, input, saved);
    assert_eq!(client.sync.device_name, "new device choice");
    assert_eq!(client.sync.ai_base_url, "https://new-model.example.test/v1");
    assert_eq!(client.sync.ai_model, "new-model");
    assert_eq!(client.sync.ai_api_key, "synthetic-new-key");
    assert!(client.sync_dirty);
    client.persist_sync().unwrap();
    assert!(!client.sync_dirty);
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sync_apply_checkpoint_mirror_failure_keeps_committed_data_and_retries_config() {
    let root = temp_test_dir("sync_apply_checkpoint_retry");
    let mut client = test_client_for_account_scope(
        &root,
        "account-a",
        app_data::default_app_data_json(now_millis()),
    );
    client.save_state().unwrap();
    let remote =
        app_data::set_theme_mode_app_data_json(&client.state_json, 2, now_millis()).unwrap();
    let mut input = sync_apply_test_input(&client, remote);
    let blocker = root.join("not-a-directory");
    fs::write(&blocker, b"block config mirror only").unwrap();
    input.sync_path = blocker.join("sync_account.json");
    input.secrets_path = blocker.join("sync_account.secrets");
    let saved = prepare_ordinary_sync_commit(&input, &CancellationToken::new())
        .unwrap()
        .unwrap();
    assert!(!saved.result.ok);
    let committed = saved.state.clone();
    sync_apply_test_deliver(&mut client, input, saved);
    assert!(client.workspace_persistence_ready);
    assert_eq!(client.state_json, committed);
    assert_eq!(fs::read_to_string(&client.state_path).unwrap(), committed);
    assert!(client.sync_dirty);
    client.persist_sync().unwrap();
    assert!(!client.sync_dirty);
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sync_apply_cancel_after_commit_adopts_checkpoint_without_followup() {
    let root = temp_test_dir("sync_apply_cancel_after_commit");
    let mut client = test_client_for_account_scope(
        &root,
        "account-a",
        app_data::default_app_data_json(now_millis()),
    );
    client.save_state().unwrap();
    let remote =
        app_data::set_theme_mode_app_data_json(&client.state_json, 2, now_millis()).unwrap();
    let input = sync_apply_test_input(&client, remote);
    let saved = prepare_ordinary_sync_commit(&input, &CancellationToken::new())
        .unwrap()
        .unwrap();
    let committed = saved.state.clone();
    let (sender, receiver) = mpsc::channel();
    client.sync_apply.receiver = Some(receiver);
    client.cancel_active_sync();
    sender
        .send(DesktopSyncApplyCompletion {
            workspace: input.workspace,
            version: input.version,
            task: input.task,
            outcome: Ok(Some(saved)),
        })
        .unwrap();
    client.poll_ordinary_sync_apply();
    assert_eq!(client.state_json, committed);
    assert!(client.workspace_persistence_ready);
    assert!(client.sync_apply.completed_sync.is_none());
    assert!(client.sync_apply.completed_media.is_none());
    assert!(client.sync_task.is_none());
    assert_eq!(client.startup_sync_due_epoch_millis, 0);
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn media_apply_unchanged_indexes_are_consumed_before_the_frame_receipt() {
    let root = temp_test_dir("media_apply_unchanged_indexes");
    let image = test_one_pixel_bmp();
    let (state, attachment) = test_note_snapshot_with_media("known-media", &image);
    let resolved = BTreeMap::from([(attachment.id, attachment.sha256)]);
    let deleted = BTreeMap::from([("already-deleted-media".to_owned(), 100_i64)]);
    let (state, changed) =
        apply_note_media_sync_summary_tracked(&state, &resolved, &deleted).unwrap();
    assert!(changed);
    let mut client = test_client_for_account_scope(&root, "account-a", state);
    let before = client.state_json.clone();
    let version = client.data_version;
    let task = NoteMediaSyncTaskResult {
        task: SyncTaskState {
            kind: SyncTaskKind::Sync,
            phase: SyncTaskPhase::NoteMedia,
            started_at_epoch_millis: now_millis(),
        },
        background_job_id: None,
        origin_workspace: client.ai_workspace_identity(),
        base_status: String::new(),
        result: Ok(
            gridtimer_native::desktop_note_media_sync::DesktopNoteMediaSyncSummary {
                uploaded: 2,
                downloaded: 3,
                failures: 1,
                failure_messages: vec!["synthetic unrelated media failure".into()],
                resolved_sha256_by_attachment_id: resolved,
                remote_deleted_at_by_attachment_id: deleted,
                ..Default::default()
            },
        ),
    };
    client.start_media_result_apply(task);
    let completion = client
        .sync_apply
        .media_receiver
        .take()
        .unwrap()
        .recv_timeout(Duration::from_secs(30))
        .unwrap();
    assert!(matches!(completion.outcome, Ok(None)));
    let summary = completion.task.result.unwrap();
    assert!(summary.resolved_sha256_by_attachment_id.is_empty());
    assert!(summary.remote_deleted_at_by_attachment_id.is_empty());
    assert_eq!(
        (summary.uploaded, summary.downloaded, summary.failures),
        (2, 3, 1)
    );
    assert_eq!(summary.failure_messages.len(), 1);
    assert_eq!(client.state_json, before);
    assert_eq!(client.data_version, version);
    let _ = client.task_supervisor.drain_for(Duration::from_secs(1));
    drop(client);
    fs::remove_dir_all(root).unwrap();
}
