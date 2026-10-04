// Supplemental synthetic sync-completion collision measurement, shared byte-for-byte.
// Only isolated source copies include this file. No OS input/display claim is made.
mod timer_sync_collision_performance_tests {
    use super::*;
    use gridtimer_native::desktop_background_jobs::BackgroundJobState;

    fn micros(start: Instant, end: Instant) -> u64 {
        end.checked_duration_since(start)
            .expect("sync collision timestamp order")
            .as_micros()
            .min(u64::MAX as u128) as u64
    }

    fn input() -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 900.0),
            )),
            ..Default::default()
        }
    }

    fn frame(client: &mut TimerWindowsClient, ctx: &egui::Context) {
        let _ = ctx.run(input(), |ctx| {
            client.pump_workspace_frame(ctx);
            client.ui_workspace(ctx);
        });
    }

    fn make_bound_client(root: &Path, case_mib: usize) -> TimerWindowsClient {
        let state = timer_action_performance_tests::synthetic_state(1);
        let mut client = test_client_for_account_scope(root, "synthetic-sync-collision", state);
        client.settings.timer_bell_enabled = false;
        client.settings.close_to_tray = false;
        let server = client.sync.server_instance_id.clone();
        let namespace = client.sync.account_namespace.clone();
        let user = client.sync.user_id.clone();
        remember_account_generation_and_capability(
            &mut client.sync,
            &server,
            &namespace,
            &user,
            0,
            "synthetic-workspace",
            "synthetic-capability",
        );
        client.save_state().unwrap();
        save_sync_files(&client.sync_path, &client.secrets_path, &client.sync).unwrap();
        client.persist_settings().unwrap();
        if case_mib == 68 {
            for sequence in 0..96 {
                let mut next: Value = serde_json::from_str(&client.state_json).unwrap();
                next["notes"][0]["title"] = json!(format!("Synthetic sync history {sequence}"));
                next["notes"][0]["updatedAtEpochMillis"] = json!(200 + sequence);
                client.state_json =
                    app_data::sanitize_app_data_json(&next.to_string(), 300).unwrap();
                client.save_state().unwrap();
                if fs::metadata(desktop_state_store_path(root)).unwrap().len() >= 64 * 1024 * 1024 {
                    break;
                }
            }
            let bytes = fs::metadata(desktop_state_store_path(root)).unwrap().len();
            assert!((60 * 1024 * 1024..=80 * 1024 * 1024).contains(&bytes));
            client.data = decode_data(&client.state_json);
        }
        assert!(sync_identity_is_bound(&client.sync));
        assert_eq!(client.sync.user_id, "synthetic-sync-collision");
        client.persistence.timer_latency_probe_enabled = true;
        client
    }

    fn response(client: &TimerWindowsClient, remote: String) -> SyncTaskResult {
        let checkpoint = workspace_checkpoint_for_account(
            &client.sync,
            &client.sync.server_instance_id,
            &client.sync.account_namespace,
            &client.sync.user_id,
        );
        assert_eq!(checkpoint.workspace_id, "synthetic-workspace");
        assert_eq!(checkpoint.workspace_capability, "synthetic-capability");
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
        let identity = bound_result_expectation(client);
        validate_sync_result_identity(&result, &identity).unwrap();
        SyncTaskResult {
            kind: SyncTaskKind::Sync,
            background_job_id: client.active_background_job_id.clone(),
            results: vec![SyncTaskStepResult {
                raw: serde_json::to_string(&result).unwrap(),
                keep_password: false,
                identity,
            }],
        }
    }

    fn settle(client: &mut TimerWindowsClient, ctx: &egui::Context) {
        let deadline = Instant::now() + Duration::from_secs(180);
        loop {
            assert!(
                Instant::now() < deadline,
                "sync fixture did not settle: {}",
                client.status
            );
            frame(client, ctx);
            if client.sync_task.is_none()
                && client.sync_result_rx.is_none()
                && client.active_background_job_id.is_none()
                && client.task_supervisor.active_count() == 0
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(16));
        }
        assert!(
            client.note_media_sync_result_rx.is_none(),
            "network dispatch escaped gate"
        );
        assert!(
            client.desktop_ui.legal_risk.sync_rx.is_none(),
            "legal network escaped gate"
        );
    }

    #[test]
    #[ignore = "Isolated supplemental sync receipt collision: 30 starts and 30 pauses"]
    fn timer_sync_receipt_collision_release_benchmark() {
        assert!(!cfg!(debug_assertions));
        let variant = std::env::var("GRIDTIMER_TIMER_BENCHMARK_VARIANT").unwrap();
        assert!(variant == "baseline" || variant == "candidate");
        let case_mib: usize = std::env::var("GRIDTIMER_TIMER_BENCHMARK_MIB")
            .unwrap()
            .parse()
            .unwrap();
        assert!([1, 68].contains(&case_mib));
        let root = timer_action_performance_tests::fixture_root();
        let local_app_data = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap());
        assert!(local_app_data.starts_with(&root));
        let network = timer_sync_collision_network_gate::Scope::enter();
        let mut client = make_bound_client(&root, case_mib);
        let initial_sessions = client.data.sessions.len();
        let ctx = egui::Context::default();
        settle(&mut client, &ctx);
        for _ in 0..4 {
            frame(&mut client, &ctx);
            std::thread::sleep(Duration::from_millis(16));
        }
        println!(
            "\nTIMER_SYNC_COLLISION_FIXTURE {}",
            json!({
                "variant": variant, "caseMiB": case_mib,
                "stateBytes": client.state_json.len(),
                "journalBytes": fs::metadata(desktop_state_store_path(&root)).unwrap().len(),
                "syntheticOnly": true, "boundSyntheticAccount": true,
                "networkDispatchBlocked": true,
                "scope": "actual SyncTaskResult reception, merge, journal/checkpoint commit, timer worker and complete workspace frame; network and OS input/display excluded"
            })
        );
        for index in 0..60 {
            let start_action = index % 2 == 0;
            assert!(!client.persistence.pending());
            assert!(client.persistence.pending_timer_action.is_none());
            assert_eq!(
                client.data.slots[0].running_since_epoch_millis.is_some(),
                !start_action
            );
            client.persistence.timer_latency_probe = None;
            let title = format!("Synthetic sync collision {index:02}");
            let remote = {
                let mut value: Value = serde_json::from_str(&client.state_json).unwrap();
                value["notes"][0]["title"] = json!(title);
                value["notes"][0]["updatedAtEpochMillis"] = json!(now_millis());
                app_data::sanitize_app_data_json(&value.to_string(), now_millis()).unwrap()
            };
            let job = client
                .stage_background_job(BackgroundJobKind::Sync)
                .unwrap();
            client.sync_task = Some(SyncTaskState {
                kind: SyncTaskKind::Sync,
                phase: SyncTaskPhase::AppData,
                started_at_epoch_millis: now_millis(),
            });
            client.sync_state_changed_since_request = false;
            let (sender, receiver) = mpsc::channel();
            sender.send(response(&client, remote)).unwrap();
            drop(sender);
            client.sync_result_rx = Some(receiver);
            let slot = client.selected_slot().unwrap();
            let mut handler_entered = None;
            let mut dispatch_return = None;
            // Model a queued input event. Baseline synchronous response work in
            // the production pump belongs to this interval, before dispatch.
            let click = Instant::now();
            let _ = ctx.run(input(), |ctx| {
                client.pump_workspace_frame(ctx);
                handler_entered = Some(Instant::now());
                client.toggle_slot(&slot, ctx);
                dispatch_return = Some(Instant::now());
                client.ui_workspace(ctx);
            });
            let first_frame = Instant::now();
            assert!(
                client.persistence.pending_timer_action.is_some(),
                "first click rejected: {}",
                client.status
            );
            assert_eq!(
                client.data.slots[0].running_since_epoch_millis.is_some(),
                !start_action
            );
            let accepted_id = client.persistence.pending_timer_action.as_ref().unwrap().id;
            let queued_behind_sync = client
                .persistence
                .timer_latency_probe
                .as_ref()
                .unwrap()
                .submitted
                .is_none();
            let mut sync_observed = (client.data.notes[0].title == title).then_some(first_frame);
            let mut peak_frame = micros(click, first_frame);
            let (committed_frame, committed_frame_work) = loop {
                assert!(
                    click.elapsed() < Duration::from_secs(180),
                    "timer did not commit: {}",
                    client.status
                );
                std::thread::sleep(Duration::from_millis(16));
                let frame_start = Instant::now();
                frame(&mut client, &ctx);
                let frame_end = Instant::now();
                peak_frame = peak_frame.max(micros(frame_start, frame_end));
                assert!(client.workspace_persistence_ready, "{}", client.status);
                assert!(!client.persistence.failed(), "{}", client.status);
                if client.data.notes[0].title == title && sync_observed.is_none() {
                    sync_observed = Some(frame_end);
                }
                if client.persistence.pending_timer_action.is_none()
                    && !client.persistence.pending()
                {
                    assert_eq!(
                        client.data.slots[0].running_since_epoch_millis.is_some(),
                        start_action
                    );
                    assert_eq!(client.data.notes[0].title, title);
                    break (frame_end, micros(frame_start, frame_end));
                }
            };
            let times = client
                .persistence
                .timer_latency_probe
                .as_ref()
                .unwrap()
                .clone();
            assert_eq!(times.action_id, accepted_id);
            let submitted = times.submitted.expect("submitted timestamp");
            let worker_started = times.worker_started.expect("worker start timestamp");
            let transformed = times.transform_completed.expect("transform timestamp");
            let saved = times.save_completed.expect("durable timestamp");
            let receipt_ready = times.receipt_ready.expect("receipt ready timestamp");
            let receipt = times.receipt_applied.expect("receipt applied timestamp");
            assert!(
                click <= times.accepted
                    && times.accepted <= submitted
                    && submitted <= worker_started
                    && worker_started <= transformed
                    && transformed <= saved
                    && saved <= receipt_ready
                    && receipt_ready <= receipt
                    && receipt <= committed_frame
            );
            settle(&mut client, &ctx);
            let sync_settled = Instant::now();
            assert_eq!(
                client.data.notes[0].title, title,
                "deferred finish replayed sync snapshot"
            );
            assert_eq!(
                client.data.slots[0].running_since_epoch_millis.is_some(),
                start_action
            );
            let job_state = client
                .background_job_store()
                .unwrap()
                .load(&job)
                .unwrap()
                .unwrap()
                .state;
            assert_eq!(job_state, BackgroundJobState::Completed);
            let durable = decode_data(&fs::read_to_string(&client.state_path).unwrap());
            assert_eq!(durable.notes[0].title, title);
            assert_eq!(
                durable.slots[0].running_since_epoch_millis.is_some(),
                start_action
            );
            println!(
                "TIMER_SYNC_COLLISION_SAMPLE {}",
                json!({
                    "variant": variant, "caseMiB": case_mib, "sample": index / 2,
                    "action": if start_action { "start" } else { "pause" },
                    "firstDispatchAccepted": true, "dispatchCount": 1,
                    "handlerEnteredMicros": micros(click, handler_entered.unwrap()),
                    "firstDispatchReturnMicros": micros(click, dispatch_return.unwrap()),
                    "firstFrameMicros": micros(click, first_frame),
                    "firstPendingFeedbackVisible": variant == "candidate",
                    "queuedBehindSync": queued_behind_sync,
                    "syncObservedFrameMicros": micros(click, sync_observed.unwrap()),
                    "syncSettledMicros": micros(click, sync_settled),
                    "firstClickCommittedMicros": micros(click, committed_frame),
                    "acceptedMicros": micros(click, times.accepted),
                    "committedFrameWorkMicros": committed_frame_work,
                    "peakFrameMicros": peak_frame,
                    "acceptedToSubmittedMicros": micros(times.accepted, submitted),
                    "workerQueueMicros": micros(submitted, worker_started),
                    "workerTransformMicros": micros(worker_started, transformed),
                    "durableSaveMicros": micros(transformed, saved),
                    "workerPostprocessMicros": micros(saved, receipt_ready),
                    "receiptDeliveryAndApplyMicros": micros(receipt_ready, receipt),
                    "acceptedClickSaveCompletedMicros": micros(click, saved),
                    "acceptedClickReceiptAppliedMicros": micros(click, receipt),
                    "syncMergedTitlePreserved": true, "finalRunningStateVerified": true,
                    "profile": "supplemental synthetic queued input, production pump then dispatch/UI; native input/display and real network excluded"
                })
            );
        }
        assert_eq!(client.data.sessions.len(), initial_sessions + 30);
        settle(&mut client, &ctx);
        let counts = network.counts();
        assert_eq!(
            counts[1], 60,
            "every successful response must reach blocked media dispatch"
        );
        assert_eq!(
            counts[3], 0,
            "synthetic fixture must not contain token revocations"
        );
        drop(client);
        let reopened = TimerWindowsClient::load_from_root(root, Arc::new(AtomicBool::new(false)));
        assert!(reopened.workspace_persistence_ready, "{}", reopened.status);
        assert_eq!(reopened.sync.user_id, "synthetic-sync-collision");
        assert!(sync_identity_is_bound(&reopened.sync));
        assert!(reopened
            .data
            .slots
            .iter()
            .all(|slot| slot.running_since_epoch_millis.is_none()));
        assert_eq!(reopened.data.sessions.len(), initial_sessions + 30);
        assert_eq!(reopened.data.notes[0].title, "Synthetic sync collision 59");
        drop(reopened);
        println!(
            "TIMER_SYNC_COLLISION_REOPEN_VERIFIED {}",
            json!({
                "sessions": 30, "running": 0, "boundSyntheticAccount": true,
                "blockedSyncDispatches": counts[0], "blockedMediaDispatches": counts[1],
                "blockedLegalDispatches": counts[2], "blockedRevocationDispatches": counts[3]
            })
        );
    }
}
