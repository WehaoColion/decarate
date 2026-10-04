// v1.1.0.3 Windows - Isolated complete frame-pump timing with synthetic workspaces.
// Included inside the desktop binary's tests module, identically in both builds.
mod timer_action_performance_tests {
    use super::*;

    pub(super) fn fixture_root() -> PathBuf {
        let root = PathBuf::from(
            std::env::var_os("GRIDTIMER_TIMER_BENCHMARK_ROOT")
                .expect("set an isolated synthetic benchmark root"),
        );
        assert!(root.is_absolute());
        assert!(root
            .parent()
            .and_then(Path::file_name)
            .and_then(|s| s.to_str())
            .is_some_and(|name| name.starts_with("timer_latency_synthetic_")));
        assert!(
            !root.exists(),
            "benchmark never overwrites an existing workspace"
        );
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn synthetic_state(target_mib: usize) -> String {
        let mut state: Value = serde_json::from_str(&app_data::default_app_data_json(100)).unwrap();
        if target_mib != 0 {
            let content = "Synthetic timer response paragraph. ".repeat(512);
            let sample = desktop_note_save_value(
                "timer-benchmark-note-00000",
                DesktopNoteKind::Sticky,
                "Synthetic timer note 00000",
                &content,
                None,
                100,
            );
            // Sanitization materializes the rich-document representation as
            // well as text. Size the actual persisted JSON, not the input note.
            let empty_bytes = app_data::sanitize_app_data_json(&state.to_string(), 100)
                .unwrap()
                .len();
            let mut one_note = state.clone();
            one_note["notes"] = Value::Array(vec![sample]);
            let one_note_bytes = app_data::sanitize_app_data_json(&one_note.to_string(), 100)
                .unwrap()
                .len();
            let note_bytes = one_note_bytes.checked_sub(empty_bytes).unwrap();
            assert!(note_bytes > 0);
            let requested_bytes = target_mib * 1024 * 1024;
            let count = if target_mib == 32 {
                // The production managed-mirror bound is 32 MiB. Leave room
                // for all 60 timer transitions rather than benchmark its
                // over-limit warning path. Include each array separator.
                requested_bytes.saturating_sub(512 * 1024 + empty_bytes) / (note_bytes + 1)
            } else {
                requested_bytes
                    .saturating_sub(empty_bytes)
                    .div_ceil(note_bytes)
            };
            state["notes"] = Value::Array(
                (0..count)
                    .map(|index| {
                        desktop_note_save_value(
                            &format!("timer-benchmark-note-{index:05}"),
                            DesktopNoteKind::Sticky,
                            &format!("Synthetic timer note {index:05}"),
                            &content,
                            None,
                            100,
                        )
                    })
                    .collect(),
            );
        }
        app_data::sanitize_app_data_json(&state.to_string(), 100).unwrap()
    }

    pub(super) fn make_client(root: &Path, case_mib: usize) -> TimerWindowsClient {
        // Reuse the production-size startup journal shape for the history case.
        let state = synthetic_state(if case_mib == 68 { 1 } else { case_mib });
        let mut client = test_client_for_account_scope(
            root,
            "synthetic-timer-offline",
            app_data::default_app_data_json(100),
        );
        client.sync = DesktopSyncSession::default();
        client.state_path = state_path_for_user(root, "");
        client.state_json = state;
        client.data = decode_data(&client.state_json);
        client.settings.timer_bell_enabled = false;
        client.settings.close_to_tray = false;
        client.save_state().unwrap();
        save_sync_files(&client.sync_path, &client.secrets_path, &client.sync).unwrap();
        client.persist_settings().unwrap();
        if case_mib == 68 {
            for sequence in 0..96 {
                let mut next: Value = serde_json::from_str(&client.state_json).unwrap();
                next["notes"][0]["title"] = json!(format!("Synthetic timer history {sequence}"));
                next["notes"][0]["updatedAtEpochMillis"] = json!(200 + sequence);
                client.state_json =
                    app_data::sanitize_app_data_json(&next.to_string(), 300).unwrap();
                client.save_state().unwrap();
                if fs::metadata(desktop_state_store_path(root)).unwrap().len() >= 64 * 1024 * 1024 {
                    break;
                }
            }
            let bytes = fs::metadata(desktop_state_store_path(root)).unwrap().len();
            assert!(
                (60 * 1024 * 1024..=80 * 1024 * 1024).contains(&bytes),
                "history fixture size={bytes}"
            );
            client.data = decode_data(&client.state_json);
        }
        assert!(client.sync.user_id.is_empty() && client.sync.token.is_empty());
        assert!(!sync_identity_is_bound(&client.sync));
        client.persistence.timer_latency_probe_enabled = true;
        client
    }

    fn frame(client: &mut TimerWindowsClient, ctx: &egui::Context) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 900.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            client.pump_workspace_frame(ctx);
            client.ui_workspace(ctx);
        });
    }

    fn micros(start: Instant, end: Instant) -> u64 {
        end.checked_duration_since(start)
            .expect("probe timestamp order")
            .as_micros()
            .min(u64::MAX as u128) as u64
    }

    #[test]
    #[ignore = "Explicit isolated release timer latency benchmark; 30 starts and 30 pauses"]
    fn timer_action_end_to_end_release_benchmark() {
        assert!(!cfg!(debug_assertions), "run the release test binary");
        let case_mib: usize = std::env::var("GRIDTIMER_TIMER_BENCHMARK_MIB")
            .unwrap()
            .parse()
            .unwrap();
        assert!([0, 1, 8, 32, 68].contains(&case_mib));
        let variant = std::env::var("GRIDTIMER_TIMER_BENCHMARK_VARIANT")
            .unwrap_or_else(|_| "unspecified".into());
        let root = fixture_root();
        let mut client = make_client(&root, case_mib);
        let state_bytes = client.state_json.len();
        let journal_bytes = fs::metadata(desktop_state_store_path(&root)).unwrap().len();
        if [1, 8, 32].contains(&case_mib) {
            let requested = case_mib * 1024 * 1024;
            assert!(
                (requested * 9 / 10..=requested * 11 / 10).contains(&state_bytes),
                "sanitized fixture does not match requested size: {state_bytes}"
            );
        }
        println!(
            "\nTIMER_LATENCY_FIXTURE {}",
            json!({
                "variant": variant, "caseMiB": case_mib, "stateBytes": state_bytes,
                "journalBytes": journal_bytes, "notes": client.data.notes.len(),
                "syntheticOnly": true, "signedOut": true
            })
        );
        let ctx = egui::Context::default();
        // Warm font/layout and initial read tasks equally. No timer action is hidden here.
        for _ in 0..4 {
            frame(&mut client, &ctx);
            std::thread::sleep(Duration::from_millis(16));
        }
        for sample in 0..60 {
            let start_action = sample % 2 == 0;
            let slot = client.data.slots.first().unwrap().clone();
            assert_eq!(slot.running_since_epoch_millis.is_some(), !start_action);
            assert!(client.persistence.pending_timer_action.is_none());
            assert!(!client.persistence.pending());
            let mut dispatch_return = None;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 900.0),
                )),
                ..Default::default()
            };
            // Input is queued before the frame begins. Count the production
            // pump preceding button dispatch as part of the response latency.
            let click = Instant::now();
            let _ = ctx.run(input, |ctx| {
                client.pump_workspace_frame(ctx);
                client.toggle_slot(&slot, ctx);
                dispatch_return = Some(Instant::now());
                client.ui_workspace(ctx);
            });
            let feedback_frame = Instant::now();
            assert!(
                client.persistence.pending_timer_action.is_some(),
                "action was rejected: {}",
                client.status
            );
            let accepted_id = client.persistence.pending_timer_action.as_ref().unwrap().id;
            let mut peak_frame_micros = micros(click, feedback_frame);
            let (committed_frame, committed_frame_work_micros) = loop {
                assert!(
                    click.elapsed() < Duration::from_secs(90),
                    "timer did not commit: {}",
                    client.status
                );
                // Fixed 60 Hz synthetic frame cadence; callback-to-display latency is not claimed.
                std::thread::sleep(Duration::from_millis(16));
                let frame_start = Instant::now();
                frame(&mut client, &ctx);
                let frame_end = Instant::now();
                peak_frame_micros = peak_frame_micros.max(micros(frame_start, frame_end));
                assert!(client.workspace_persistence_ready, "{}", client.status);
                if client.persistence.pending_timer_action.is_none()
                    && !client.persistence.pending()
                {
                    assert_eq!(
                        client
                            .data
                            .slots
                            .first()
                            .unwrap()
                            .running_since_epoch_millis
                            .is_some(),
                        start_action,
                        "action did not produce expected state: {}",
                        client.status
                    );
                    break (frame_end, micros(frame_start, frame_end));
                }
            };
            let times = client
                .persistence
                .timer_latency_probe
                .as_ref()
                .expect("enabled timing probe")
                .clone();
            assert_eq!(times.action_id, accepted_id);
            assert!(
                client.state_json.len() <= 32 * 1024 * 1024,
                "saved state exceeds managed mirror bound"
            );
            assert!(
                client.last_state_mirror_warning.is_none(),
                "benchmark entered a degraded mirror path: {:?}",
                client.last_state_mirror_warning
            );
            let submitted = times.submitted.expect("submitted timestamp");
            let worker_started = times.worker_started.expect("worker start timestamp");
            let transformed = times.transform_completed.expect("transform timestamp");
            let saved = times.save_completed.expect("durable save timestamp");
            let receipt_ready = times.receipt_ready.expect("receipt ready timestamp");
            let receipt = times.receipt_applied.expect("accepted receipt timestamp");
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
            println!(
                "TIMER_LATENCY_SAMPLE {}",
                json!({
                    "variant": variant, "caseMiB": case_mib, "sample": sample / 2,
                    "action": if start_action { "start" } else { "pause" }, "actionId": accepted_id,
                    "stateBytes": state_bytes, "journalBytes": journal_bytes,
                    "savedStateBytes": client.state_json.len(),
                    "dispatchReturnMicros": micros(click, dispatch_return.unwrap()),
                    "acceptedMicros": micros(click, times.accepted),
                    "submittedMicros": micros(click, submitted),
                    "workerStartedMicros": micros(click, worker_started),
                    "transformCompletedMicros": micros(click, transformed),
                    "feedbackFrameMicros": micros(click, feedback_frame),
                    "feedbackVisible": variant == "candidate",
                    "feedbackVisibilityBasis": "candidate pending label; baseline only first frame return",
                    "saveCompletedMicros": micros(click, saved),
                    "receiptReadyMicros": micros(click, receipt_ready),
                    "receiptAppliedMicros": micros(click, receipt),
                    "committedFrameMicros": micros(click, committed_frame),
                    "committedFrameWorkMicros": committed_frame_work_micros,
                    "prepareMainThreadMicros": micros(times.accepted, submitted),
                    "workerQueueMicros": micros(submitted, worker_started),
                    "workerTransformMicros": micros(worker_started, transformed),
                    "durableSaveMicros": micros(transformed, saved),
                    "saveStagesMicros": times.save_stages,
                    "workerPostprocessMicros": micros(saved, receipt_ready),
                    "receiptDeliveryAndApplyMicros": micros(receipt_ready, receipt),
                    "peakFrameMicros": peak_frame_micros,
                    "profile": "release full production frame pump; 60Hz synthetic frames; native input and display excluded"
                })
            );
        }
        println!(
            "TIMER_LATENCY_FINAL_STATE {}",
            json!({
                "variant": variant, "caseMiB": case_mib,
                "savedStateBytes": client.state_json.len(), "mirrorWarning": client.last_state_mirror_warning,
                "managedMirrorBoundBytes": 32 * 1024 * 1024
            })
        );
        drop(client);
        let expected_parent = root.parent().unwrap();
        assert!(expected_parent
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("timer_latency_synthetic_"));
        fs::remove_dir_all(&root).unwrap();
    }
}
