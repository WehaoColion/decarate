// v0.0.1 - Reject queued replies after cancel, workspace switch and shutdown.
#[test]
fn sync_cancel_preserves_local_state_and_ambiguous_commit_receipt() {
    for action in ["cancel", "scope", "shutdown"] {
        let dir = temp_test_dir(&format!("cancel_{action}"));
        let original = app_state_with_note("note-a", "Local", "LOCAL_PENDING", None);
        let mut client = test_client_for_account_scope(&dir, "account-a", original.clone());
        let id = client
            .stage_background_job(BackgroundJobKind::Sync)
            .unwrap();
        let response = bound_app_data_response(
            &client,
            add_note_to_state(&original, "note-a", "Remote", "LATE_REMOTE", 300),
        );
        let (tx, rx) = mpsc::channel();
        tx.send(SyncTaskResult {
            kind: SyncTaskKind::Sync,
            background_job_id: Some(id.clone()),
            results: vec![SyncTaskStepResult {
                raw: serde_json::to_string(&response).unwrap(),
                keep_password: false,
                identity: bound_result_expectation(&client),
            }],
        })
        .unwrap();
        client.sync_result_rx = Some(rx);
        client.sync_task = Some(SyncTaskState {
            kind: SyncTaskKind::Sync,
            phase: SyncTaskPhase::AppData,
            started_at_epoch_millis: now_millis(),
        });
        let before_sync = serde_json::to_value(&client.sync).unwrap();
        match action {
            "cancel" => client.cancel_active_sync(),
            "scope" => client
                .apply_prepared_state_scope(dir.join("another-workspace.json"), original.clone()),
            _ => client.begin_safe_shutdown(),
        }
        assert!(client.sync_result_rx.is_none());
        assert!(client.note_media_sync_result_rx.is_none());
        assert!(client.sync_task.is_none());
        client.poll_sync_result();
        assert_eq!(
            client.state_json, original,
            "late response altered local data after {action}"
        );
        assert_eq!(serde_json::to_value(&client.sync).unwrap(), before_sync);
        let record = client
            .background_job_store
            .as_ref()
            .unwrap()
            .load(&id)
            .unwrap()
            .unwrap();
        assert_eq!(
            record.state,
            gridtimer_native::desktop_background_jobs::BackgroundJobState::AwaitingReconciliation
        );
        assert!(client.active_background_job_id.is_none());
        assert!(client.task_supervisor.is_accepting() == (action != "shutdown"));
        drop(client);
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn sync_cancel_only_stops_current_account_io_and_allows_next_request() {
    let dir = temp_test_dir("cancel_worker_isolation");
    let mut client =
        test_client_for_account_scope(&dir, "account-a", app_data::default_app_data_json(100));
    let (tx, rx) = mpsc::channel();
    for kind in [
        RuntimeTaskKind::Sync,
        RuntimeTaskKind::MediaSync,
        RuntimeTaskKind::HealthCheck,
        RuntimeTaskKind::TokenRevoke,
    ] {
        let tx = tx.clone();
        client
            .task_supervisor
            .spawn(kind, TaskDurability::Ephemeral, move |token| {
                tx.send((kind, token.clone())).unwrap();
                let started = Instant::now();
                while !token.is_cancelled() && started.elapsed() < Duration::from_secs(4) {
                    thread::sleep(Duration::from_millis(5));
                }
            })
            .unwrap();
    }
    let tokens = (0..4)
        .map(|_| rx.recv_timeout(Duration::from_secs(3)).unwrap())
        .collect::<Vec<_>>();
    client.cancel_active_sync();
    for (kind, token) in &tokens {
        assert_eq!(token.is_cancelled(), *kind != RuntimeTaskKind::TokenRevoke);
    }
    let (next_tx, next_rx) = mpsc::channel();
    client
        .task_supervisor
        .spawn(
            RuntimeTaskKind::Sync,
            TaskDurability::Ephemeral,
            move |token| {
                next_tx.send(token.is_cancelled()).unwrap();
            },
        )
        .unwrap();
    assert!(!next_rx.recv_timeout(Duration::from_secs(1)).unwrap());
    client.task_supervisor.begin_shutdown();
    assert_eq!(
        client
            .task_supervisor
            .drain_for(Duration::from_secs(1))
            .len(),
        5
    );
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}
