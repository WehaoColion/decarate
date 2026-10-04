// v1.1.0.3 Windows - One timer click colliding with a real synthetic autosave.
// Shared byte-for-byte with the baseline. No network, account, or user data.
mod timer_collision_performance_tests {
    use super::*;

    fn micros(start: Instant, end: Instant) -> u64 {
        end.checked_duration_since(start)
            .expect("collision timestamp order")
            .as_micros()
            .min(u64::MAX as u128) as u64
    }

    fn frame(client: &mut TimerWindowsClient, ctx: &egui::Context) {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 900.0),
                )),
                ..Default::default()
            },
            |ctx| {
                client.pump_workspace_frame(ctx);
                client.ui_workspace(ctx);
            },
        );
    }

    #[test]
    #[ignore = "Explicit isolated release autosave collision; 30 first start clicks and 30 first pause clicks"]
    fn timer_autosave_collision_release_benchmark() {
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
        let mut client = timer_action_performance_tests::make_client(&root, case_mib);
        assert!(client.sync.user_id.is_empty() && client.sync.token.is_empty());
        let initial_sessions = client.data.sessions.len();
        let ctx = egui::Context::default();
        for _ in 0..4 {
            frame(&mut client, &ctx);
            std::thread::sleep(Duration::from_millis(16));
        }
        println!(
            "\nTIMER_COLLISION_FIXTURE {}",
            json!({"variant": variant, "caseMiB": case_mib,
                "stateBytes": client.state_json.len(),
                "journalBytes": fs::metadata(desktop_state_store_path(&root)).unwrap().len(),
                "syntheticOnly": true, "signedOut": true,
                "predecessorGate": "isolated SQLite IMMEDIATE transaction held at least 100ms after the first click"})
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
            client.ensure_selected_slot_draft();
            let title = format!("Synthetic autosave collision {index:02}");
            client.slot_title_draft = title.clone();
            client.mark_slot_dirty();
            let mut gate = rusqlite::Connection::open(desktop_state_store_path(&root)).unwrap();
            let transaction = gate
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .unwrap();
            client.submit_draft_snapshot(&ctx, true);
            assert!(
                client.persistence.pending(),
                "predecessor must already be submitted"
            );
            let slot = client.selected_slot().unwrap();
            let first_click = Instant::now();
            let mut dispatch_return = None;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 900.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    client.pump_workspace_frame(ctx);
                    assert!(
                        client.persistence.pending(),
                        "the lock keeps the autosave in flight"
                    );
                    client.toggle_slot(&slot, ctx);
                    dispatch_return = Some(Instant::now());
                    client.ui_workspace(ctx);
                },
            );
            let first_frame = Instant::now();
            let first_accepted = client.persistence.pending_timer_action.is_some();
            let first_status = client.status.clone();
            assert_eq!(first_accepted, variant == "candidate", "{first_status}");
            assert_eq!(
                client.data.slots[0].running_since_epoch_millis.is_some(),
                !start_action
            );
            while first_click.elapsed() < Duration::from_millis(100) {
                std::thread::sleep(Duration::from_millis(16));
                frame(&mut client, &ctx);
                assert_eq!(
                    client.data.slots[0].running_since_epoch_millis.is_some(),
                    !start_action
                );
            }
            transaction.rollback().unwrap();
            drop(gate);
            let predecessor_gate_released = Instant::now();
            let mut predecessor_observed = None;
            let mut retry_click = None;
            let mut retry_frame = None;
            let mut dispatch_count = 1;
            let (committed_frame, committed_frame_work) = loop {
                assert!(
                    first_click.elapsed() < Duration::from_secs(90),
                    "{}",
                    client.status
                );
                std::thread::sleep(Duration::from_millis(16));
                let frame_start = Instant::now();
                frame(&mut client, &ctx);
                let frame_end = Instant::now();
                assert!(client.workspace_persistence_ready, "{}", client.status);
                assert!(!client.persistence.failed(), "{}", client.status);
                if client.data.slots[0].title == title && predecessor_observed.is_none() {
                    predecessor_observed = Some(frame_end);
                }
                if !first_accepted && retry_click.is_none() && !client.persistence.pending() {
                    assert!(predecessor_observed.is_some() && !client.slot_dirty);
                    assert!(client.persistence.pending_timer_action.is_none());
                    let retry_slot = client.selected_slot().unwrap();
                    let click = Instant::now();
                    let _ = ctx.run(egui::RawInput::default(), |ctx| {
                        client.pump_workspace_frame(ctx);
                        client.toggle_slot(&retry_slot, ctx);
                        client.ui_workspace(ctx);
                    });
                    retry_click = Some(click);
                    retry_frame = Some(Instant::now());
                    dispatch_count += 1;
                    assert!(
                        client.persistence.pending_timer_action.is_some(),
                        "explicit retry rejected"
                    );
                    continue;
                }
                if client.persistence.pending_timer_action.is_none()
                    && !client.persistence.pending()
                {
                    assert_eq!(
                        client.data.slots[0].running_since_epoch_millis.is_some(),
                        start_action
                    );
                    assert_eq!(client.data.slots[0].title, title);
                    assert!(!client.slot_dirty);
                    break (frame_end, micros(frame_start, frame_end));
                }
            };
            assert_eq!(dispatch_count, if first_accepted { 1 } else { 2 });
            let times = client.persistence.timer_latency_probe.as_ref().unwrap();
            let submitted = times.submitted.expect("submitted timestamp");
            let worker_started = times.worker_started.expect("worker start timestamp");
            let transformed = times.transform_completed.expect("transform timestamp");
            let saved = times.save_completed.expect("durable save timestamp");
            let receipt_ready = times.receipt_ready.expect("receipt ready timestamp");
            let receipt = times.receipt_applied.expect("receipt timestamp");
            let accepted_click = retry_click.unwrap_or(first_click);
            assert!(
                accepted_click <= times.accepted
                    && times.accepted <= submitted
                    && submitted <= worker_started
                    && worker_started <= transformed
                    && transformed <= saved
                    && saved <= receipt_ready
                    && receipt_ready <= receipt
                    && receipt <= committed_frame
            );
            let mut sample = json!({
                "variant": variant, "caseMiB": case_mib, "sample": index / 2,
                "action": if start_action { "start" } else { "pause" },
                "firstDispatchAccepted": first_accepted, "firstDispatchStatus": first_status,
                "dispatchCount": dispatch_count,
                "firstDispatchReturnMicros": micros(first_click, dispatch_return.unwrap()),
                "firstFrameMicros": micros(first_click, first_frame),
                "firstPendingFeedbackVisible": first_accepted,
                "predecessorGateReleasedMicros": micros(first_click, predecessor_gate_released),
                "predecessorObservedFrameMicros": micros(first_click, predecessor_observed.unwrap()),
                "retryDispatchMicros": retry_click.map(|at| micros(first_click, at)),
                "retryFirstFrameMicros": retry_frame.map(|at| micros(first_click, at)),
                "firstClickCommittedMicros": if first_accepted { Some(micros(first_click, committed_frame)) } else { None },
                "retryClickCommittedMicros": retry_click.map(|at| micros(at, committed_frame)),
                "initialClickThroughExplicitRetryMicros": micros(first_click, committed_frame),
                "acceptedMicros": micros(accepted_click, times.accepted),
                "committedFrameWorkMicros": committed_frame_work,
                "queuedBehindPredecessor": first_accepted,
                "profile": "synthetic production pump and workspace rendering; fixed 100ms SQLite lock gate; initial rejection and explicit retry kept separate"
            });
            for (key, value) in [
                (
                    "acceptedToSubmittedMicros",
                    micros(times.accepted, submitted),
                ),
                ("workerQueueMicros", micros(submitted, worker_started)),
                ("workerTransformMicros", micros(worker_started, transformed)),
                ("durableSaveMicros", micros(transformed, saved)),
                ("workerPostprocessMicros", micros(saved, receipt_ready)),
                (
                    "receiptDeliveryAndApplyMicros",
                    micros(receipt_ready, receipt),
                ),
                (
                    "acceptedClickSaveCompletedMicros",
                    micros(accepted_click, saved),
                ),
                (
                    "acceptedClickReceiptAppliedMicros",
                    micros(accepted_click, receipt),
                ),
            ] {
                sample[key] = json!(value);
            }
            println!("TIMER_COLLISION_SAMPLE {sample}");
        }
        assert_eq!(client.data.sessions.len(), initial_sessions + 30);
        drop(client);
        let reopened =
            TimerWindowsClient::load_from_root(root.clone(), Arc::new(AtomicBool::new(false)));
        assert!(reopened.workspace_persistence_ready, "{}", reopened.status);
        assert!(reopened.sync.user_id.is_empty() && reopened.sync.token.is_empty());
        assert!(reopened
            .data
            .slots
            .iter()
            .all(|slot| slot.running_since_epoch_millis.is_none()));
        assert_eq!(reopened.data.sessions.len(), initial_sessions + 30);
        assert_eq!(
            reopened.data.slots[0].title,
            "Synthetic autosave collision 59"
        );
        drop(reopened);
        println!("TIMER_COLLISION_REOPEN_VERIFIED sessions=30 running=0 signedOut=true");
    }
}
