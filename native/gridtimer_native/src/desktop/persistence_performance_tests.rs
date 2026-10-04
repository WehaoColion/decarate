// v1.0.3.1 Windows - Exercise appearance recovery through the production autosave path.
// v1.0.3 Windows - Verify timer summary retention, rollover, and complete session invalidation.
// v1.0.2.2 Windows - Verify timer-only phase guards and compare their allocation-free document scan.
// v1.0.2.1 Windows - Ensure save receipts carry current timer models without consuming newer drafts.

#[test]
fn theme_autosave_retries_after_a_new_choice_without_repeating_a_failed_choice() {
    let root = temp_test_dir("theme_autosave_retry");
    let mut client = test_client_for_account_scope(
        &root,
        "theme-autosave-retry",
        app_data::default_app_data_json(now_millis()),
    );
    let database = root.join(DESKTOP_STATE_STORE_FILE);
    // A directory at the SQLite file path gives a real, isolated write error.
    fs::create_dir(&database).unwrap();
    let original = client.state_json.clone();
    let ctx = egui::Context::default();
    assert!(client.set_desktop_theme(2));
    client.flush_due_saves(&ctx);
    assert!(client.persistence.pending());
    drain_draft_writer(&mut client);
    assert_eq!(client.state_json, original);
    assert!(!client.theme_dirty);
    assert_eq!(None, client.pending_theme_code);

    fs::remove_dir(&database).unwrap();
    for _ in 0..3 {
        client.flush_due_saves(&ctx);
        assert!(!client.persistence.pending());
    }
    assert!(!database.exists(), "a failed choice must not retry itself");

    assert!(client.set_desktop_theme(3));
    client.flush_due_saves(&ctx);
    assert!(
        client.persistence.pending(),
        "a new choice must reach autosave"
    );
    drain_draft_writer(&mut client);
    assert!(!client.theme_dirty, "{}", client.status);
    assert_eq!(
        ThemePreference::Oled,
        desktop_theme_preference(&client.data)
    );
    let reloaded = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    assert_eq!(ThemePreference::Oled, desktop_theme_preference(&reloaded));
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn older_theme_failure_preserves_and_saves_the_newer_unsubmitted_choice() {
    let root = temp_test_dir("theme_stale_failure");
    let mut client = test_client_for_account_scope(
        &root,
        "theme-stale-failure",
        app_data::default_app_data_json(now_millis()),
    );
    let database = root.join(DESKTOP_STATE_STORE_FILE);
    fs::create_dir(&database).unwrap();
    let ctx = egui::Context::default();
    assert!(client.set_desktop_theme(2));
    client.flush_due_saves(&ctx);
    assert!(client.persistence.pending());
    // Do not poll or submit between these actions: only the older choice is
    // in the worker, and its failure must not consume this newer UI intent.
    assert!(client.set_desktop_theme(3));
    drain_draft_writer(&mut client);
    assert!(
        client.theme_dirty,
        "an old failure consumed the newer choice"
    );
    assert_eq!(Some(3), client.pending_theme_code);
    assert_eq!(
        ThemePreference::Oled,
        client.effective_desktop_theme_preference()
    );

    fs::remove_dir(&database).unwrap();
    client.flush_due_saves(&ctx);
    assert!(client.persistence.pending());
    drain_draft_writer(&mut client);
    assert!(!client.theme_dirty, "{}", client.status);
    let reloaded = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    assert_eq!(ThemePreference::Oled, desktop_theme_preference(&reloaded));
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn theme_choice_does_not_silently_retry_a_failed_content_save() {
    let root = temp_test_dir("theme_content_failure");
    let mut client = test_client_for_account_scope(
        &root,
        "theme-content-failure",
        app_data::default_app_data_json(now_millis()),
    );
    let database = root.join(DESKTOP_STATE_STORE_FILE);
    fs::create_dir(&database).unwrap();
    client.ensure_selected_slot_draft();
    client.slot_title_draft = "unsaved title survives failure".into();
    client.mark_slot_dirty();
    let ctx = egui::Context::default();
    assert!(client.set_desktop_theme(2));
    client.flush_due_saves(&ctx);
    assert!(client.persistence.pending());
    drain_draft_writer(&mut client);
    fs::remove_dir(&database).unwrap();

    assert!(client.set_desktop_theme(3));
    for _ in 0..3 {
        client.flush_due_saves(&ctx);
        assert!(!client.persistence.pending());
    }
    assert!(
        !database.exists(),
        "appearance must not retry failed content"
    );
    assert!(client.slot_dirty);
    assert_eq!("unsaved title survives failure", client.slot_title_draft);
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn switching_workspace_after_a_failed_theme_save_restores_theme_and_content_autosave() {
    let root = temp_test_dir("theme_failure_workspace_switch");
    let mut client = test_client_for_account_scope(
        &root,
        "failed-workspace-a",
        app_state_with_note("scope-a-note", "Account A", "A content", None),
    );
    let target = test_client_for_account_scope(
        &root,
        "healthy-workspace-b",
        app_state_with_note("scope-b-note", "Account B", "B content", None),
    );
    let mut target_sync = target.sync.clone();
    let target_path = target.state_path.clone();
    drop(target);
    let old_path = client.state_path.clone();
    let original_a = fs::read(&old_path).unwrap();
    let database = root.join(DESKTOP_STATE_STORE_FILE);
    fs::create_dir(&database).unwrap();
    let ctx = egui::Context::default();
    assert!(client.set_desktop_theme(2));
    client.flush_due_saves(&ctx);
    assert!(client.persistence.pending());
    drain_draft_writer(&mut client);
    assert!(!client.theme_dirty);
    let revisions_before = client.persistence.revisions;
    let committed_before = client.persistence.committed_revision;
    fs::remove_dir(&database).unwrap();

    // Use the actual preparation/flush/journal/scope-switch chain, rather
    // than assigning a different state path directly in the fixture.
    client.switch_state_scope(&mut target_sync, false).unwrap();
    assert_eq!(target_path, client.state_path);
    assert_ne!(old_path, client.state_path);
    assert_eq!(revisions_before, client.persistence.revisions);
    assert_eq!(committed_before, client.persistence.committed_revision);
    assert!(client.set_desktop_theme(3));
    client.flush_due_saves(&ctx);
    assert!(
        client.persistence.pending(),
        "old account failure blocked the new account"
    );
    drain_draft_writer(&mut client);
    assert!(!client.theme_dirty, "{}", client.status);

    let note = client
        .data
        .notes
        .iter()
        .find(|note| note.id == "scope-b-note")
        .unwrap()
        .clone();
    client.load_note_draft_without_flush(&note);
    client.set_note_canvas_text("B content saved after switching account");
    client.note_save_due_epoch_millis = 0;
    client.flush_due_saves(&ctx);
    assert!(client.persistence.pending());
    drain_draft_writer(&mut client);
    assert!(!client.note_dirty, "{}", client.status);
    let reloaded = decode_data(&fs::read_to_string(&target_path).unwrap());
    assert_eq!(ThemePreference::Oled, desktop_theme_preference(&reloaded));
    let saved_note = reloaded
        .notes
        .iter()
        .find(|note| note.id == "scope-b-note")
        .unwrap();
    assert_eq!(
        "B content saved after switching account",
        note_body_text(saved_note)
    );
    assert_eq!(
        original_a,
        fs::read(&old_path).unwrap(),
        "switching must not rewrite account A"
    );
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reapplying_the_same_workspace_does_not_clear_its_failed_autosave_gate() {
    let root = temp_test_dir("theme_failure_same_workspace");
    let mut client = test_client_for_account_scope(
        &root,
        "same-workspace",
        app_state_with_note("same-scope-note", "Title", "Original content", None),
    );
    let database = root.join(DESKTOP_STATE_STORE_FILE);
    fs::create_dir(&database).unwrap();
    let ctx = egui::Context::default();
    assert!(client.set_desktop_theme(2));
    client.flush_due_saves(&ctx);
    assert!(client.persistence.pending());
    drain_draft_writer(&mut client);
    fs::remove_dir(&database).unwrap();
    let path_before = client.state_path.clone();
    let mut same_sync = client.sync.clone();
    client.switch_state_scope(&mut same_sync, false).unwrap();
    assert_eq!(path_before, client.state_path);

    let original = fs::read(&client.state_path).unwrap();
    let note = client.data.notes[0].clone();
    client.load_note_draft_without_flush(&note);
    client.set_note_canvas_text("Draft remains pending until explicit retry");
    client.note_save_due_epoch_millis = 0;
    client.flush_due_saves(&ctx);
    assert!(
        !client.persistence.pending(),
        "same-scope refresh must not erase a save failure"
    );
    assert!(client.note_dirty);
    assert_eq!(original, fs::read(&client.state_path).unwrap());
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn draft_receipt_adopts_saved_timer_model_and_preserves_newer_slot_draft() {
    let root = temp_test_dir("draft_receipt_timer_model");
    let now = now_millis();
    let mut client =
        test_client_for_account_scope(&root, "timer-model", app_data::default_app_data_json(now));
    client.ensure_selected_slot_draft();
    client.refresh_timer_projection(now);
    client.slot_title_draft = "saved title".into();
    client.mark_slot_dirty();
    client.submit_draft_snapshot(&egui::Context::default(), true);

    client.slot_title_draft = "newer unsaved title".into();
    client.mark_slot_dirty();
    drain_draft_writer(&mut client);

    assert!(
        client.slot_dirty,
        "an older receipt must not acknowledge new edits"
    );
    assert_eq!(client.slot_title_draft, "newer unsaved title");
    assert_eq!(
        client.desktop_ui.projection_version,
        client.timers_cache_version()
    );
    assert_eq!(
        client
            .desktop_ui
            .projection
            .as_ref()
            .unwrap()
            .slots
            .iter()
            .find(|slot| slot.id == client.selected_slot_id)
            .unwrap()
            .title,
        "saved title"
    );
    let projected = client.desktop_ui.projector.as_ref().unwrap().project(now);
    assert_eq!(
        projected
            .slots
            .iter()
            .find(|slot| slot.id == client.selected_slot_id)
            .unwrap()
            .title,
        "saved title"
    );
    assert_eq!(
        projected,
        app_data::TimerProjector::parse(&client.state_json)
            .unwrap()
            .project(now),
        "the worker model must exactly describe the adopted snapshot"
    );

    client.submit_draft_snapshot(&egui::Context::default(), true);
    drain_draft_writer(&mut client);
    assert!(!client.slot_dirty, "{}", client.status);
    assert_eq!(
        client.desktop_ui.projection_version,
        client.timers_cache_version()
    );
    assert_eq!(
        client
            .desktop_ui
            .projector
            .as_ref()
            .unwrap()
            .project(now)
            .slots
            .iter()
            .find(|slot| slot.id == client.selected_slot_id)
            .unwrap()
            .title,
        "newer unsaved title"
    );
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn theme_receipt_installs_timer_model_for_its_new_state_generation() {
    let root = temp_test_dir("theme_receipt_timer_model");
    let now = now_millis();
    let mut client = test_client_for_account_scope(
        &root,
        "theme-timer-model",
        app_data::default_app_data_json(now),
    );
    client.refresh_timer_projection(now);
    let previous_version = client.desktop_ui.projection_version;
    assert!(client.set_desktop_theme(3));
    client.submit_draft_snapshot(&egui::Context::default(), true);
    drain_draft_writer(&mut client);

    assert!(client.data_version > previous_version);
    assert_eq!(
        client.desktop_ui.projection_version,
        client.timers_cache_version()
    );
    assert_eq!(
        client.desktop_ui.projector.as_ref().unwrap().project(now),
        app_data::TimerProjector::parse(&client.state_json)
            .unwrap()
            .project(now)
    );
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn timer_summary_receipt_requires_same_generation_and_complete_sessions() {
    let sessions = vec![
        DesktopSession {
            id: "run-a".into(),
            slot_id: 1,
            slot_title: "Synthetic timer".into(),
            started_at_epoch_millis: 100,
            ended_at_epoch_millis: 200,
            duration_millis: 100,
        },
        DesktopSession {
            id: "run-b".into(),
            slot_id: 2,
            slot_title: "Second timer".into(),
            started_at_epoch_millis: 300,
            ended_at_epoch_millis: 500,
            duration_millis: 200,
        },
    ];
    let retained = |current, base, key, saved: &[DesktopSession]| {
        desktop_persistence::retained_timer_summary_key(current, base, key, &sessions, saved)
    };
    assert_eq!(retained(7, 7, Some((7, 123)), &sessions), Some((8, 123)));
    assert_eq!(retained(8, 7, Some((7, 123)), &sessions), None);
    assert_eq!(retained(7, 7, Some((6, 123)), &sessions), None);
    assert_eq!(retained(7, 7, Some((8, 123)), &sessions), None);
    assert_eq!(retained(7, 7, None, &sessions), None);

    let changes: [fn(&mut DesktopSession); 6] = [
        |session| session.id.push_str("-changed"),
        |session| session.slot_id += 1,
        |session| session.slot_title.push_str(" changed"),
        |session| session.started_at_epoch_millis += 1,
        |session| session.ended_at_epoch_millis += 1,
        |session| session.duration_millis += 1,
    ];
    for change in changes {
        let mut saved = sessions.clone();
        change(&mut saved[0]);
        assert_eq!(retained(7, 7, Some((7, 123)), &saved), None);
    }
    assert_eq!(retained(7, 7, Some((7, 123)), &sessions[..1]), None);
    let mut reordered = sessions.clone();
    reordered.reverse();
    assert_eq!(retained(7, 7, Some((7, 123)), &reordered), None);
}

#[test]
fn draft_receipt_reuses_unchanged_timer_summary_without_hiding_day_rollover() {
    let root = temp_test_dir("draft_receipt_timer_summary");
    let now = now_millis();
    let mut source: Value = serde_json::from_str(&app_data::default_app_data_json(now)).unwrap();
    let slot_id = source["slots"][0]["id"].as_i64().unwrap() as i32;
    source["sessions"] = json!([{
        "id":"summary-session", "slotId":slot_id, "slotTitle":"Synthetic timer",
        "startedAtEpochMillis":now - 60_000, "endedAtEpochMillis":now,
        "durationMillis":60_000
    }]);
    let state = app_data::sanitize_app_data_json(&source.to_string(), now).unwrap();
    let mut client = test_client_for_account_scope(&root, "timer-summary", state);
    client.refresh_android_timer_summary();
    let (_, cached_day) = client.desktop_ui.parity.timer_stats_key.unwrap();
    let recent_pointer = client.desktop_ui.parity.timer_stats[&slot_id]
        .recent
        .as_ptr();
    client.ensure_selected_slot_draft();
    client.slot_note_draft = "First saved annotation".into();
    client.mark_slot_dirty();
    client.submit_draft_snapshot(&egui::Context::default(), true);
    drain_draft_writer(&mut client);
    assert!(!client.slot_dirty, "{}", client.status);
    assert_eq!(
        client.desktop_ui.parity.timer_stats_key,
        Some((client.data_version, cached_day))
    );
    client.refresh_android_timer_summary();
    assert_eq!(
        client.desktop_ui.parity.timer_stats[&slot_id]
            .recent
            .as_ptr(),
        recent_pointer,
        "unrelated saves must retain the existing summary allocation"
    );

    // Simulate a summary created before the current local day, then save an
    // unrelated edit. Retagging its generation must not relabel its day.
    let expired_day = desktop_local_day_window(now_millis(), 0).0 - 1;
    client.desktop_ui.parity.timer_stats_key = Some((client.data_version, expired_day));
    client
        .desktop_ui
        .parity
        .timer_stats
        .get_mut(&slot_id)
        .unwrap()
        .today_millis = -1;
    client.slot_note_draft = "Second saved annotation".into();
    client.mark_slot_dirty();
    client.submit_draft_snapshot(&egui::Context::default(), true);
    drain_draft_writer(&mut client);
    assert_eq!(
        client.desktop_ui.parity.timer_stats_key,
        Some((client.data_version, expired_day))
    );
    client.refresh_android_timer_summary();
    let window = desktop_local_day_window(now_millis(), 0);
    let expected = android_timer_stats(&client.data.sessions, window);
    assert_eq!(
        client.desktop_ui.parity.timer_stats_key,
        Some((client.data_version, window.0))
    );
    assert_eq!(
        client.desktop_ui.parity.timer_stats[&slot_id].today_millis,
        expected[&slot_id].today_millis
    );
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "Explicit isolated summary retention benchmark; no production data is read"]
fn timer_summary_receipt_retention_benchmark() {
    fn measure(mut operation: impl FnMut()) -> Value {
        for _ in 0..3 {
            operation();
        }
        let mut elapsed = Vec::with_capacity(30);
        for _ in 0..30 {
            let started = Instant::now();
            operation();
            elapsed.push(started.elapsed().as_nanos() as u64);
        }
        elapsed.sort_unstable();
        json!({"medianMicros":elapsed[15] as f64 / 1000.0,
            "p95Micros":elapsed[28] as f64 / 1000.0})
    }

    let mut results = Vec::new();
    let window = (0, 86_400_000);
    for count in [1_000, 20_000] {
        let sessions = (0..count)
            .map(|id| DesktopSession {
                id: format!("synthetic-session-{id}"),
                slot_id: (id % 48) as i32 + 1,
                slot_title: format!("Synthetic timer {}", id % 48),
                started_at_epoch_millis: id as i64 * 60_000,
                ended_at_epoch_millis: (id as i64 + 1) * 60_000,
                duration_millis: 60_000,
            })
            .collect::<Vec<_>>();
        // Use independent string allocations, matching two separately decoded
        // snapshots rather than benefiting from comparisons of the same slice.
        let saved = sessions.clone();
        let old_rebuild = measure(|| {
            std::hint::black_box(android_timer_stats(&saved, window));
        });
        let equality_and_retag = measure(|| {
            std::hint::black_box(desktop_persistence::retained_timer_summary_key(
                7,
                7,
                Some((7, window.0)),
                std::hint::black_box(&sessions),
                std::hint::black_box(&saved),
            ));
        });
        let mut changed = saved.clone();
        changed.last_mut().unwrap().duration_millis += 1;
        let equality_and_changed_rebuild = measure(|| {
            if desktop_persistence::retained_timer_summary_key(
                7,
                7,
                Some((7, window.0)),
                &sessions,
                &changed,
            )
            .is_none()
            {
                std::hint::black_box(android_timer_stats(&changed, window));
            }
        });
        assert_eq!(
            desktop_persistence::retained_timer_summary_key(
                7,
                7,
                Some((7, window.0)),
                &sessions,
                &saved,
            ),
            Some((8, window.0))
        );
        results.push(json!({"sessions":count,"slots":48,
            "oldSummaryRebuild":old_rebuild,
            "completeEqualityAndRetag":equality_and_retag,
            "lastSessionChangedEqualityAndRebuild":equality_and_changed_rebuild}));
    }
    let report = json!({"version":WINDOWS_CLIENT_VERSION,
        "scope":"same-build synthetic receipt summary invalidation; includes complete session equality and changed-session fallback; excludes shared persistence work",
        "results":results});
    println!("TIMER_SUMMARY_RETENTION_PERFORMANCE {}", report);
    if let Some(path) = std::env::var_os("DESKTOP_TIMER_SUMMARY_PERFORMANCE_OUTPUT") {
        fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}

#[test]
#[ignore = "Explicit isolated timer receipt benchmark; no production data is read"]
fn draft_receipt_timer_model_benchmark() {
    fn measure(mut operation: impl FnMut()) -> Value {
        for _ in 0..3 {
            operation();
        }
        let mut elapsed = Vec::with_capacity(30);
        for _ in 0..30 {
            let started = Instant::now();
            operation();
            elapsed.push(started.elapsed().as_nanos() as u64);
        }
        elapsed.sort_unstable();
        json!({"medianMicros": elapsed[15] as f64 / 1000.0,
            "p95Micros": elapsed[28] as f64 / 1000.0})
    }

    let now = now_millis();
    let mut results = Vec::new();
    for target_bytes in [1024 * 1024, 18 * 1024 * 1024] {
        let mut source: Value =
            serde_json::from_str(&app_data::default_app_data_json(now)).unwrap();
        let note = json!({"id":"fixture", "kind":"DOCUMENT", "title":"Synthetic page",
            "content":"sample content ".repeat(100),
            "revisions": (0..6).map(|n| json!({"id":format!("revision-{n}"),
                "content":"history content ".repeat(100)})).collect::<Vec<_>>()});
        let note_bytes = serde_json::to_string(&note).unwrap().len();
        source["notes"] = json!((0..target_bytes / note_bytes)
            .map(|id| {
                let mut note = note.clone();
                note["id"] = json!(format!("fixture-{id}"));
                note
            })
            .collect::<Vec<_>>());
        let state = serde_json::to_string(&source).unwrap();
        let prepared = app_data::TimerProjector::parse(&state).unwrap();
        let old = measure(|| {
            std::hint::black_box(
                app_data::TimerProjector::parse(&state)
                    .unwrap()
                    .project(now),
            );
        });
        let optimized = measure(|| {
            std::hint::black_box(prepared.project(now));
        });
        assert_eq!(
            prepared.project(now),
            app_data::TimerProjector::parse(&state)
                .unwrap()
                .project(now)
        );
        results.push(json!({"stateBytes":state.len(),
            "oldUiParseAndProject":old, "preparedModelProjection":optimized,
            "avoidedSnapshotCopyBytes":state.len()}));
    }
    let report = json!({"version":WINDOWS_CLIENT_VERSION,
        "scope":"synthetic same-build timer receipt UI work; excludes disk persistence and other frame work",
        "results":results});
    println!("DRAFT_RECEIPT_PERFORMANCE {}", report);
    if let Some(path) = std::env::var_os("DESKTOP_PERSISTENCE_PERFORMANCE_OUTPUT") {
        fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}

#[test]
fn timer_phase_snapshot_guard_requires_the_same_explicit_running_identity() {
    let mut source: Value = serde_json::from_str(&app_data::default_app_data_json(100)).unwrap();
    source["slots"][0]["runningSinceEpochMillis"] = json!(100);
    source["slots"][0]["activeRunId"] = json!("expected-run");
    let slot_id = source["slots"][0]["id"].as_i64().unwrap() as i32;
    let matches = |source: &Value, expected_slot, expected_run: &str| {
        desktop_persistence::snapshot_has_active_timer_run(
            &source.to_string(),
            expected_slot,
            expected_run,
        )
    };

    assert!(matches(&source, slot_id, "expected-run"));
    assert!(!matches(&source, slot_id, "older-run"));
    assert!(!matches(&source, -1, "expected-run"));
    source["slots"][0]["runningSinceEpochMillis"] = Value::Null;
    assert!(!matches(&source, slot_id, "expected-run"));
    source["slots"][0]["runningSinceEpochMillis"] = json!(100);
    source["slots"][0]
        .as_object_mut()
        .unwrap()
        .remove("activeRunId");
    assert!(!matches(&source, slot_id, "expected-run"));
    assert!(!matches(&source, slot_id, ""));
    assert!(!desktop_persistence::snapshot_has_active_timer_run(
        r#"{"slots":[],"notes":[invalid]}"#,
        slot_id,
        "expected-run",
    ));
}

#[test]
#[ignore = "Explicit isolated timer phase guard benchmark; no production data is read"]
fn timer_phase_snapshot_guard_benchmark() {
    fn measure(mut operation: impl FnMut()) -> Value {
        for _ in 0..3 {
            operation();
        }
        let mut elapsed = Vec::with_capacity(20);
        for _ in 0..20 {
            let started = Instant::now();
            operation();
            elapsed.push(started.elapsed().as_nanos() as u64);
        }
        elapsed.sort_unstable();
        json!({"medianMicros":elapsed[10] as f64 / 1000.0,
            "p95Micros":elapsed[18] as f64 / 1000.0})
    }

    let mut results = Vec::new();
    for target_bytes in [1024 * 1024, 18 * 1024 * 1024] {
        let mut source: Value =
            serde_json::from_str(&app_data::default_app_data_json(100)).unwrap();
        source["slots"][0]["runningSinceEpochMillis"] = json!(100);
        source["slots"][0]["activeRunId"] = json!("benchmark-run");
        let slot_id = source["slots"][0]["id"].as_i64().unwrap() as i32;
        let note = json!({"id":"fixture", "title":"Synthetic page", "content":"sample ".repeat(200),
            "revisions":(0..6).map(|n| json!({"id":format!("revision-{n}"),
                "content":"history ".repeat(200)})).collect::<Vec<_>>()});
        let note_bytes = serde_json::to_string(&note).unwrap().len();
        source["notes"] = json!((0..target_bytes / note_bytes)
            .map(|id| {
                let mut note = note.clone();
                note["id"] = json!(format!("fixture-{id}"));
                note
            })
            .collect::<Vec<_>>());
        let state = source.to_string();
        let old_guard = || {
            decode_data(&state).slots.iter().any(|slot| {
                slot.id == slot_id && desktop_slot_running_identity(slot) == Some("benchmark-run")
            })
        };
        let new_guard =
            || desktop_persistence::snapshot_has_active_timer_run(&state, slot_id, "benchmark-run");
        assert!(old_guard());
        assert!(new_guard());
        let full = measure(|| {
            std::hint::black_box(old_guard());
        });
        let timers_only = measure(|| {
            std::hint::black_box(new_guard());
        });
        results.push(json!({"stateBytes":state.len(), "fullWorkspaceGuard":full,
            "timerOnlyGuard":timers_only}));
    }
    let report = json!({"version":WINDOWS_CLIENT_VERSION,
        "scope":"same-build background timer phase identity check; excludes shared mutation validation and disk writes",
        "results":results});
    println!("TIMER_PHASE_GUARD_PERFORMANCE {}", report);
    if let Some(path) = std::env::var_os("DESKTOP_PHASE_GUARD_PERFORMANCE_OUTPUT") {
        fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}
