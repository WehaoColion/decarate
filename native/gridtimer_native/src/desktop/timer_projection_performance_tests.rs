// v1.1.0.3 Windows - Keep timer read models across unrelated content receipts.
// v1.0.3.16 Windows - Verify frame pacing and benchmark active timer projection.
// v1.0.2.2 Windows - Verify timer cache invalidation, exact live clocks, and idle reuse.

#[test]
fn timer_read_models_survive_note_receipts_and_refresh_timer_order() {
    let root = temp_test_dir("timer_read_models_content_receipts");
    let mut client = knowledge_test_client(&root);
    let now = now_millis();
    client.refresh_timer_projection(now);
    client.refresh_history_index(now);
    let projection = Arc::clone(client.desktop_ui.projection.as_ref().unwrap());
    let history = Arc::clone(client.desktop_ui.history_index.as_ref().unwrap());
    let version = client.timers_cache_version();

    let mut value: Value = serde_json::from_str(&client.state_json).unwrap();
    value["notes"][0]["title"] = json!("changed knowledge title");
    assert!(client.replace_state(Some(value.to_string()), "updated"));
    assert_eq!(version, client.timers_cache_version());
    client.refresh_timer_projection(now);
    client.refresh_history_index(now);
    assert_eq!(version, client.desktop_ui.projection_version);
    assert!(Arc::ptr_eq(
        &history,
        client.desktop_ui.history_index.as_ref().unwrap()
    ));

    let mut order = projection.slot_order.clone();
    order.reverse();
    let ordered = app_data::set_slot_order_app_data_json(&client.state_json, &order, now).unwrap();
    assert!(client.replace_state(Some(ordered), "updated"));
    assert_ne!(version, client.timers_cache_version());
    client.refresh_timer_projection(now);
    assert!(!Arc::ptr_eq(
        &projection,
        client.desktop_ui.projection.as_ref().unwrap()
    ));
    assert_eq!(
        &order,
        &client.desktop_ui.projection.as_ref().unwrap().slot_order
    );

    // An unsupported current-format root field is rejected by the production
    // mutation gate. Read-only projections of future/foreign snapshots still
    // invalidate conservatively, including at an unchanged wall-clock time.
    let mut value: Value = serde_json::from_str(&client.state_json).unwrap();
    value["futureTimerMetadata"] = json!({"revision": 1});
    assert!(!client.replace_state(Some(value.to_string()), "updated"));
    let previous = client.data.clone();
    client.state_json = value.to_string();
    client.data = decode_data(&client.state_json);
    client.data_version += 1;
    client.apply_content_changes(DesktopContentChanges::between(&previous, &client.data));
    client.refresh_timer_projection(now);
    assert_eq!(
        json!({"revision": 1}),
        client
            .desktop_ui
            .projection
            .as_ref()
            .unwrap()
            .source_extra_fields["futureTimerMetadata"]
    );
    let projection = Arc::clone(client.desktop_ui.projection.as_ref().unwrap());
    client.data_version += 1;
    client.refresh_timer_projection(now);
    assert!(!Arc::ptr_eq(
        &projection,
        client.desktop_ui.projection.as_ref().unwrap()
    ));
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn timer_projection_reuses_paused_values_without_mutating_retained_snapshots() {
    let root = temp_test_dir("timer_projection_paused_reuse");
    let mut client = test_client_for_account_scope(
        &root,
        "paused-projection",
        app_data::default_app_data_json(100),
    );
    client.refresh_timer_projection(100);
    let original_slots = client
        .desktop_ui
        .projection
        .as_ref()
        .unwrap()
        .slots
        .as_ptr();
    client.refresh_timer_projection(200);
    let projection = client.desktop_ui.projection.as_ref().unwrap();
    assert_eq!(original_slots, projection.slots.as_ptr());
    assert_eq!(200, projection.projected_at_epoch_millis);
    assert_eq!(
        projection.as_ref(),
        &client.desktop_ui.projector.as_ref().unwrap().project(200),
    );

    // A board frame may hold an older Arc while handling an action. Updating
    // the live cache must leave that retained snapshot immutable.
    let retained = Arc::clone(projection);
    client.refresh_timer_projection(300);
    assert_eq!(200, retained.projected_at_epoch_millis);
    assert_eq!(
        300,
        client
            .desktop_ui
            .projection
            .as_ref()
            .unwrap()
            .projected_at_epoch_millis,
    );
    drop(retained);
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn timer_projection_advances_live_phases_and_invalidates_same_time_mutations() {
    let root = temp_test_dir("timer_projection_live_invalidation");
    let start = 1_000;
    let raw = app_data::start_slot_app_data_json(&app_data::default_app_data_json(start), 1, start)
        .unwrap();
    let mut client = test_client_for_account_scope(&root, "live-projection", raw);
    client.refresh_timer_projection(start);
    let first = Arc::clone(client.desktop_ui.projection.as_ref().unwrap());
    client.refresh_timer_projection(start);
    assert!(Arc::ptr_eq(
        &first,
        client.desktop_ui.projection.as_ref().unwrap(),
    ));
    let focus_target = first
        .slots
        .iter()
        .find(|s| s.id == 1)
        .unwrap()
        .micro_break_phase_target_millis;
    assert!(focus_target > 0);
    for now in [start + 1_234, start + focus_target + 1, start - 100] {
        client.refresh_timer_projection(now);
        assert_eq!(
            client.desktop_ui.projection.as_ref().unwrap().as_ref(),
            &app_data::TimerProjector::parse(&client.state_json)
                .unwrap()
                .project(now),
            "live phase, total, and backward-clock behavior must remain exact",
        );
    }

    let now = start + 2_000;
    client.refresh_timer_projection(now);
    client.state_json = app_data::pause_slots_app_data_json(&client.state_json, &[1], now).unwrap();
    client.data = decode_data(&client.state_json);
    client.data_version += 1;
    client.refresh_timer_projection(now);
    let projection = client.desktop_ui.projection.as_ref().unwrap();
    assert!(
        !projection
            .slots
            .iter()
            .find(|s| s.id == 1)
            .unwrap()
            .is_running
    );
    assert_eq!(
        projection.as_ref(),
        &app_data::TimerProjector::parse(&client.state_json)
            .unwrap()
            .project(now),
        "a source change must invalidate even at an identical timestamp",
    );
    drop(first);
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn timer_frame_projection_paces_live_repaints_but_never_delays_state_changes() {
    let root = temp_test_dir("timer_frame_projection_pacing");
    let start = 1_000;
    let raw = app_data::start_slot_app_data_json(&app_data::default_app_data_json(start), 1, start)
        .unwrap();
    let mut client = test_client_for_account_scope(&root, "frame-pacing", raw);
    client.refresh_timer_projection_for_frame(start);
    let initial = Arc::clone(client.desktop_ui.projection.as_ref().unwrap());
    client.refresh_timer_projection_for_frame(start + 100);
    assert!(Arc::ptr_eq(
        &initial,
        client.desktop_ui.projection.as_ref().unwrap()
    ));
    client.refresh_timer_projection_for_frame(start + 200);
    assert_eq!(
        start + 200,
        client
            .desktop_ui
            .projection
            .as_ref()
            .unwrap()
            .projected_at_epoch_millis
    );

    // A pause must replace the cached running view even inside the frame window.
    client.state_json =
        app_data::pause_slots_app_data_json(&client.state_json, &[1], start + 250).unwrap();
    client.data = decode_data(&client.state_json);
    client.data_version += 1;
    client.refresh_timer_projection_for_frame(start + 250);
    assert!(
        !client
            .desktop_ui
            .projection
            .as_ref()
            .unwrap()
            .slots
            .iter()
            .find(|slot| slot.id == 1)
            .unwrap()
            .is_running
    );
    drop(initial);
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn timer_frame_projection_reaches_phase_boundary_and_schedules_settlement() {
    let root = temp_test_dir("timer_frame_phase_boundary");
    let start = 1_000;
    let raw = app_data::start_slot_app_data_json(&app_data::default_app_data_json(start), 1, start)
        .unwrap();
    let mut client = test_client_for_account_scope(&root, "frame-phase", raw);
    client.settings.timer_bell_enabled = false;
    client.refresh_timer_projection_for_frame(start);
    let initial = client
        .desktop_ui
        .projection
        .as_ref()
        .unwrap()
        .slots
        .iter()
        .find(|slot| slot.id == 1)
        .unwrap();
    let focus_target = initial.micro_break_phase_target_millis;
    let initial_phase = initial.micro_break_phase;
    assert!(focus_target > 200);

    let before = start + focus_target - 50;
    client.refresh_timer_projection_for_frame(before);
    client.check_timer_bells();
    client.refresh_timer_projection_for_frame(before + 51);
    assert_eq!(
        initial_phase,
        client
            .desktop_ui
            .projection
            .as_ref()
            .unwrap()
            .slots
            .iter()
            .find(|slot| slot.id == 1)
            .unwrap()
            .micro_break_phase
    );
    client.refresh_timer_projection_for_frame(before + 200);
    let next_phase = client
        .desktop_ui
        .projection
        .as_ref()
        .unwrap()
        .slots
        .iter()
        .find(|slot| slot.id == 1)
        .unwrap()
        .micro_break_phase;
    assert_ne!(initial_phase, next_phase);
    client.check_timer_bells();
    assert_eq!(
        Some(1),
        client
            .desktop_ui
            .pending_phase_settlement
            .as_ref()
            .map(|pending| pending.slot_id)
    );
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn timer_board_empty_query_keeps_existing_search_semantics() {
    for query in ["", " ", "\t\n\u{3000}"] {
        assert!(timer_board_text_matches_query("A", "B", "C", 2, query));
    }
    assert!(timer_board_text_matches_query(
        "项目 Alpha",
        "记录 Beta",
        "分类 Gamma",
        2,
        "ALPHA beta gamma 02",
    ));
    assert!(!timer_board_text_matches_query(
        "项目 Alpha",
        "记录 Beta",
        "分类 Gamma",
        2,
        "alpha absent",
    ));
}

#[test]
#[ignore = "Explicit isolated idle/board preparation benchmark; no personal data is read"]
fn timer_projection_idle_and_board_benchmark() {
    fn measure(mut operation: impl FnMut()) -> Value {
        for _ in 0..3 {
            operation();
        }
        let mut times = Vec::with_capacity(60);
        for _ in 0..60 {
            let start = Instant::now();
            for _ in 0..16 {
                operation();
            }
            times.push(start.elapsed().as_nanos() as u64 / 16);
        }
        times.sort_unstable();
        json!({"medianMicros": times[30] as f64 / 1000.0,
            "p95Micros": times[57] as f64 / 1000.0})
    }

    let mut results = Vec::new();
    for note_bytes in [128, 8_192] {
        let root = temp_test_dir(&format!("idle_board_{note_bytes}"));
        let mut state: Value = serde_json::from_str(&app_data::default_app_data_json(100)).unwrap();
        for slot in state["slots"].as_array_mut().unwrap() {
            slot["title"] = json!("A timer card");
            slot["note"] = json!("n".repeat(note_bytes));
        }
        let mut client = test_client_for_account_scope(&root, "board-benchmark", state.to_string());
        client.refresh_timer_projection(100);
        for slot in &client.data.slots {
            client.desktop_ui.parity.timer_stats.insert(
                slot.id,
                AndroidTimerStats {
                    today_millis: 120_000,
                    today_count: 2,
                    recent: (0..20)
                        .map(|n| DesktopSession {
                            id: format!("history-{n}"),
                            slot_title: "historical title ".repeat(32),
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                },
            );
        }
        let mut now = 100;
        let legacy_idle = measure(|| {
            now += 1;
            client.desktop_ui.projection = Some(Arc::new(
                client.desktop_ui.projector.as_ref().unwrap().project(now),
            ));
            std::hint::black_box(&client.desktop_ui.projection);
        });
        let reused_idle = measure(|| {
            now += 1;
            client.refresh_timer_projection(now);
            std::hint::black_box(&client.desktop_ui.projection);
        });
        let legacy_board = measure(|| {
            let projection = client
                .desktop_ui
                .projection
                .as_ref()
                .unwrap()
                .as_ref()
                .clone();
            let ids = projection.slots.iter().map(|s| s.id).collect::<Vec<_>>();
            let slots = desktop_timer::ordered_slot_ids(&ids, &projection.slot_order)
                .iter()
                .filter_map(|id| projection.slots.iter().find(|s| s.id == *id))
                .filter(|slot| {
                    let haystack = format!(
                        "{}\n{}\n\n格子 {:02}\n格子{}\n{}",
                        slot.title, slot.note, slot.id, slot.id, slot.id
                    )
                    .to_lowercase();
                    "".split_whitespace()
                        .all(|keyword| haystack.contains(&keyword.to_lowercase()))
                })
                .cloned()
                .collect::<Vec<_>>();
            for slot in slots.iter().take(6) {
                let stats = client
                    .desktop_ui
                    .parity
                    .timer_stats
                    .get(&slot.id)
                    .cloned()
                    .unwrap_or_default();
                std::hint::black_box((stats.today_millis, stats.today_count));
            }
            std::hint::black_box(slots);
        });
        let borrowed_board = measure(|| {
            let projection = Arc::clone(client.desktop_ui.projection.as_ref().unwrap());
            let ids = projection.slots.iter().map(|s| s.id).collect::<Vec<_>>();
            let slots = desktop_timer::ordered_slot_ids(&ids, &projection.slot_order)
                .iter()
                .filter_map(|id| projection.slots.iter().find(|s| s.id == *id))
                .filter(|slot| timer_board_matches_query(slot, "", ""))
                .collect::<Vec<_>>();
            for slot in slots.iter().take(6) {
                let stats = client
                    .desktop_ui
                    .parity
                    .timer_stats
                    .get(&slot.id)
                    .map(|stats| (stats.today_millis, stats.today_count))
                    .unwrap_or_default();
                std::hint::black_box(stats);
            }
            std::hint::black_box(slots);
        });
        let live_start = 100_000;
        client.state_json =
            app_data::start_slot_app_data_json(&client.state_json, 1, live_start).unwrap();
        client.data = decode_data(&client.state_json);
        client.data_version += 1;
        client.refresh_timer_projection(live_start);
        let mut live_now = live_start;
        let exact_live = measure(|| {
            live_now += 16;
            client.refresh_timer_projection(live_now);
            std::hint::black_box(&client.desktop_ui.projection);
        });
        let paced_live = measure(|| {
            live_now += 16;
            client.refresh_timer_projection_for_frame(live_now);
            std::hint::black_box(&client.desktop_ui.projection);
        });
        results.push(
            json!({"slots":client.data.slots.len(),"noteBytesPerSlot":note_bytes,
            "legacyIdleProjection":legacy_idle,"reusedIdleProjection":reused_idle,
            "legacyBoardPreparation":legacy_board,"borrowedBoardPreparation":borrowed_board,
            "exactLiveProjection":exact_live,"pacedLiveProjection":paced_live}),
        );
        drop(client);
        fs::remove_dir_all(root).unwrap();
    }
    let report = json!({"version":WINDOWS_CLIENT_VERSION,
        "scope":"synthetic same-binary paused/live refresh and board data preparation; excludes rendering and persistence",
        "results":results});
    println!("TIMER_FRAME_PERFORMANCE {}", report);
    if let Some(path) = std::env::var_os("DESKTOP_TIMER_PERFORMANCE_OUTPUT") {
        fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}
