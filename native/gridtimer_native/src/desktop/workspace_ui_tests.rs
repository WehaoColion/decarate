// v1.0.3.8 Windows - Verify multi-block sticky keyboard edits and undo after frame optimization.
// v1.0.3.17 Windows - Cover asynchronous timer clicks and durable receipts.

fn workspace_test_frame(
    client: &mut TimerWindowsClient,
    ctx: &egui::Context,
    size: egui::Vec2,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
        events,
        ..Default::default()
    };
    ctx.run(input, |ctx| {
        client.restore_untranslated_punctuation_events(ctx);
        client.handle_workspace_shortcuts(ctx);
        client.ui_workspace(ctx);
    })
}

#[test]
fn tray_exit_keeps_failed_save_recovery_visible_and_draft_intact() {
    let root = temp_test_dir("tray_exit_failed_save_visible");
    let mut client = test_client_for_account_scope(
        &root,
        "writer",
        app_state_with_note("n", "before", "body", None),
    );
    client.select_note_by_id("n");
    client.note_title_draft = "unsaved text".into();
    client.mark_note_dirty();
    let blocker = root.join("not-a-directory");
    fs::write(&blocker, b"block").unwrap();
    client.sync_path = blocker.join("sync_account.json");
    client.desktop_ui.parity.tray.hidden = true;
    let ctx = egui::Context::default();
    client.request_desktop_tray_exit(&ctx);
    assert!(!client.desktop_ui.parity.tray.hidden);
    assert!(client.desktop_ui.parity.tray.force_exit);

    let mut input = egui::RawInput::default();
    input
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .events
        .push(egui::ViewportEvent::Close);
    let _ = ctx.run(input, |ctx| client.handle_close_request(ctx));
    assert!(client.persistence.waiting_to_close);
    drain_draft_writer(&mut client);
    assert!(matches!(
        client.shutdown_state,
        ClientShutdownState::SaveFailed { .. }
    ));
    assert!(!client.desktop_ui.parity.tray.hidden);
    assert!(client.note_dirty);
    assert_eq!(client.note_title_draft, "unsaved text");
    assert_eq!(
        decode_data(&fs::read_to_string(&client.state_path).unwrap()).notes[0].title,
        "before"
    );
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workspace_timer_close_barrier_rejects_new_tray_actions() {
    let root = temp_test_dir("timer_close_rejects_new_actions");
    let now = now_millis();
    let state =
        app_data::start_slot_app_data_json(&app_data::default_app_data_json(now), 1, now).unwrap();
    let mut client = test_client_for_account_scope(&root, "writer", state);
    client.settings.timer_bell_enabled = false;
    client.save_state().unwrap();
    let before = fs::read_to_string(&client.state_path).unwrap();
    let before_slots = client.data.slots.clone();
    let next_action_id = client.persistence.next_timer_action_id;
    let ctx = egui::Context::default();
    for (waiting, shutdown) in [
        (true, ClientShutdownState::Running),
        (
            false,
            ClientShutdownState::Draining {
                started: Instant::now(),
            },
        ),
        (
            false,
            ClientShutdownState::SaveFailed {
                message: "previous save failed".into(),
            },
        ),
        (false, ClientShutdownState::ReadyToClose),
    ] {
        client.persistence.waiting_to_close = waiting;
        client.shutdown_state = shutdown;
        // Exercise the same pause entry used by the tray while its events are
        // still being collected, plus a start request from another input path.
        client.pause_running_slots(&ctx);
        assert!(client.persistence.pending_timer_action.is_none());
        assert!(!client.persistence.pending());
        let paused = client
            .data
            .slots
            .iter()
            .find(|slot| slot.id == 2)
            .unwrap()
            .clone();
        client.toggle_slot(&paused, &ctx);
        assert!(client.persistence.pending_timer_action.is_none());
        assert!(!client.persistence.pending());
        assert_eq!(client.persistence.next_timer_action_id, next_action_id);
        assert_eq!(client.data.slots, before_slots);
        assert_eq!(fs::read_to_string(&client.state_path).unwrap(), before);
    }
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

fn workspace_click(
    client: &mut TimerWindowsClient,
    ctx: &egui::Context,
    size: egui::Vec2,
    pos: egui::Pos2,
) {
    workspace_test_frame(
        client,
        ctx,
        size,
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    workspace_test_frame(
        client,
        ctx,
        size,
        vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
}

fn wait_for_timer_action(client: &mut TimerWindowsClient) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while (client.persistence.pending_timer_action.is_some() || client.persistence.pending())
        && Instant::now() < deadline
    {
        client.poll_draft_persistence();
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        client.persistence.pending_timer_action.is_none() && !client.persistence.pending(),
        "timer save did not finish: {}",
        client.status
    );
}

fn wait_for_external_workspace_receipt(client: &mut TimerWindowsClient) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        client.poll_ordinary_sync_apply();
        client.poll_media_result_apply();
        client.poll_synced_media_cleanup();
        client.poll_deferred_sync_finish();
        if !client.external_workspace_save_pending()
            && client.sync_apply.completed_sync.is_none()
            && client.sync_apply.completed_media.is_none()
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "external save receipt did not finish: {}",
            client.status
        );
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn workspace_timer_clock_and_edit_are_separate_click_targets() {
    let dir = temp_test_dir("workspace_hit_targets");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_data::default_app_data_json(now_millis()),
    );
    client.settings.timer_bell_enabled = false;
    let ctx = workspace_test_context();
    let size = egui::vec2(1240.0, 800.0);
    for _ in 0..3 {
        workspace_test_frame(&mut client, &ctx, size, vec![]);
    }
    let clock = ctx
        .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("timer_clock", 1))))
        .unwrap();
    workspace_click(&mut client, &ctx, size, clock.center());
    assert!(client.persistence.pending_timer_action.is_some());
    assert!(client.data.slots[0].running_since_epoch_millis.is_none());
    wait_for_timer_action(&mut client);
    assert!(
        client
            .data
            .slots
            .iter()
            .find(|s| s.id == 1)
            .unwrap()
            .running_since_epoch_millis
            .is_some(),
        "clock must start its own slot"
    );
    assert!(!client.desktop_ui.slot_editor_open);
    workspace_click(&mut client, &ctx, size, clock.center());
    wait_for_timer_action(&mut client);
    assert!(client
        .data
        .slots
        .iter()
        .find(|s| s.id == 1)
        .unwrap()
        .running_since_epoch_millis
        .is_none());
    let edit = ctx
        .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("timer_edit", 1))))
        .unwrap();
    workspace_click(&mut client, &ctx, size, edit.center());
    assert!(
        client.desktop_ui.slot_editor_open,
        "edit must open without scrolling to the bottom"
    );
    assert!(client
        .data
        .slots
        .iter()
        .all(|s| s.running_since_epoch_millis.is_none()));
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_timer_repeat_click_during_save_commits_only_one_transition() {
    let dir = temp_test_dir("workspace_timer_repeat_click");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_data::default_app_data_json(now_millis()),
    );
    client.settings.timer_bell_enabled = false;
    client.save_state().unwrap();
    let before = client.state_json.clone();
    let mut gate = rusqlite::Connection::open(dir.join(DESKTOP_STATE_STORE_FILE)).unwrap();
    let transaction = gate
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let ctx = egui::Context::default();
    let slot = client.selected_slot().unwrap();
    let click_started = Instant::now();
    client.toggle_slot(&slot, &ctx);
    assert!(
        click_started.elapsed() < Duration::from_secs(2),
        "timer click blocked on the journal"
    );
    let pending_id = client
        .persistence
        .pending_timer_action
        .as_ref()
        .expect("first click should queue a timer save")
        .id;
    assert_eq!(client.state_json, before);
    assert!(client.data.slots[0].running_since_epoch_millis.is_none());
    client.toggle_slot(&slot, &ctx);
    assert_eq!(
        client
            .persistence
            .pending_timer_action
            .as_ref()
            .map(|a| a.id),
        Some(pending_id),
        "a second click during the save must not queue an opposite transition"
    );
    assert_eq!(client.state_json, before);
    transaction.rollback().unwrap();
    drop(gate);
    wait_for_timer_action(&mut client);
    assert!(client.data.slots[0].running_since_epoch_millis.is_some());
    assert!(
        decode_data(&fs::read_to_string(&client.state_path).unwrap()).slots[0]
            .running_since_epoch_millis
            .is_some()
    );
    assert!(
        client.data.sessions.is_empty(),
        "one start creates no completed session"
    );
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_timer_failed_background_save_never_displays_a_false_run() {
    let dir = temp_test_dir("workspace_timer_background_failure");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_data::default_app_data_json(now_millis()),
    );
    client.settings.timer_bell_enabled = false;
    client.save_state().unwrap();
    let before = client.state_json.clone();
    let store = dir.join(DESKTOP_STATE_STORE_FILE);
    let hidden_store = dir.join("state-store-unavailable.db");
    fs::rename(&store, &hidden_store).unwrap();
    let ctx = egui::Context::default();
    let slot = client.selected_slot().unwrap();
    client.toggle_slot(&slot, &ctx);
    assert!(client.persistence.pending_timer_action.is_some());
    wait_for_timer_action(&mut client);
    fs::rename(&hidden_store, &store).unwrap();
    assert_eq!(client.state_json, before);
    assert!(client.data.slots[0].running_since_epoch_millis.is_none());
    assert!(
        decode_data(&fs::read_to_string(&client.state_path).unwrap()).slots[0]
            .running_since_epoch_millis
            .is_none()
    );
    assert!(client.status.contains("失败") || client.status.contains("保存"));
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_timer_intention_waits_for_draft_and_preserves_click_time_once() {
    let dir = temp_test_dir("timer_intention_after_draft");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_data::default_app_data_json(now_millis()),
    );
    client.settings.timer_bell_enabled = false;
    client.save_state().unwrap();
    client.ensure_selected_slot_draft();
    client.slot_title_draft = "first saved name".into();
    client.mark_slot_dirty();
    let mut gate = rusqlite::Connection::open(dir.join(DESKTOP_STATE_STORE_FILE)).unwrap();
    let transaction = gate
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let ctx = egui::Context::default();
    client.submit_draft_snapshot(&ctx, true);
    assert!(client.persistence.pending());
    let original_version = client.data_version;
    let slot = client.selected_slot().unwrap();
    client.toggle_slot(&slot, &ctx);
    let accepted = client
        .persistence
        .pending_timer_action
        .clone()
        .expect("busy writer must retain one intention");
    assert_eq!(accepted.base_version, original_version);
    assert!(client.persistence.consume_timer_priority_frame());
    client.toggle_slot(&slot, &ctx);
    assert_eq!(
        client.persistence.pending_timer_action.as_ref(),
        Some(&accepted)
    );
    client.slot_title_draft = "latest unsaved name".into();
    client.mark_slot_dirty();
    client.submit_draft_snapshot(&ctx, true);
    assert_eq!(
        client.persistence.pending_timer_action.as_ref(),
        Some(&accepted)
    );
    assert!(client.data.slots[0].running_since_epoch_millis.is_none());
    transaction.rollback().unwrap();
    drop(gate);
    wait_for_timer_action(&mut client);
    assert_eq!(
        client.data.slots[0].running_since_epoch_millis,
        Some(accepted.requested_at_epoch_millis)
    );
    assert_eq!(client.data.slots[0].title, "latest unsaved name");
    assert!(!client.slot_dirty);
    assert!(client.data.sessions.is_empty());
    assert!(client.persistence.consume_timer_priority_frame());
    assert!(client.data_version >= original_version + 2);
    let persisted = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    assert_eq!(
        persisted.slots[0].running_since_epoch_millis,
        Some(accepted.requested_at_epoch_millis)
    );
    assert_eq!(persisted.slots[0].title, client.data.slots[0].title);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_timer_queued_intention_is_cancelled_when_predecessor_fails() {
    let dir = temp_test_dir("timer_intention_failed_draft");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_data::default_app_data_json(now_millis()),
    );
    client.settings.timer_bell_enabled = false;
    client.save_state().unwrap();
    client.ensure_selected_slot_draft();
    client.slot_title_draft = "unsaved must survive".into();
    client.mark_slot_dirty();
    let store = dir.join(DESKTOP_STATE_STORE_FILE);
    let hidden = dir.join("unavailable-state.db");
    fs::rename(&store, &hidden).unwrap();
    let ctx = egui::Context::default();
    client.submit_draft_snapshot(&ctx, true);
    let slot = client.selected_slot().unwrap();
    client.toggle_slot(&slot, &ctx);
    assert!(client.persistence.pending_timer_action.is_some());
    wait_for_timer_action(&mut client);
    fs::rename(&hidden, &store).unwrap();
    assert!(client.persistence.failed());
    assert!(client.slot_dirty);
    assert_eq!(client.slot_title_draft, "unsaved must survive");
    assert!(client.data.slots[0].running_since_epoch_millis.is_none());
    assert!(
        decode_data(&fs::read_to_string(&client.state_path).unwrap()).slots[0]
            .running_since_epoch_millis
            .is_none()
    );
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_timer_queued_intention_rejects_changed_workspace() {
    let dir = temp_test_dir("timer_intention_changed_workspace");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_data::default_app_data_json(now_millis()),
    );
    client.settings.timer_bell_enabled = false;
    client.save_state().unwrap();
    client.ensure_selected_slot_draft();
    client.slot_title_draft = "saved predecessor".into();
    client.mark_slot_dirty();
    let mut gate = rusqlite::Connection::open(dir.join(DESKTOP_STATE_STORE_FILE)).unwrap();
    let transaction = gate
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let ctx = egui::Context::default();
    client.submit_draft_snapshot(&ctx, true);
    let slot = client.selected_slot().unwrap();
    client.toggle_slot(&slot, &ctx);
    assert!(client.persistence.pending_timer_action.is_some());
    client.sync.account_namespace = "different-workspace".into();
    transaction.rollback().unwrap();
    drop(gate);
    wait_for_timer_action(&mut client);
    assert!(client.data.slots[0].running_since_epoch_millis.is_none());
    assert!(
        decode_data(&fs::read_to_string(&client.state_path).unwrap()).slots[0]
            .running_since_epoch_millis
            .is_none()
    );
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_queued_pause_keeps_click_time_across_a_pending_phase_boundary() {
    let dir = temp_test_dir("timer_queued_pause_phase_boundary");
    let sample =
        app_data::start_slot_app_data_json(&app_data::default_app_data_json(1_000), 1, 1_000)
            .unwrap();
    let focus_millis = app_data::TimerProjector::parse(&sample)
        .unwrap()
        .project(1_000)
        .slots[0]
        .micro_break_phase_target_millis;
    let started_at = now_millis() - focus_millis - 60_000;
    let clicked_at = started_at + focus_millis - 1_000;
    let state = app_data::start_slot_app_data_json(
        &app_data::default_app_data_json(started_at),
        1,
        started_at,
    )
    .unwrap();
    let mut client = test_client_for_account_scope(&dir, "account-a", state);
    client.settings.timer_bell_enabled = false;
    client.save_state().unwrap();
    client.ensure_selected_slot_draft();
    client.slot_title_draft = "name saved before pause".into();
    client.mark_slot_dirty();
    let mut gate = rusqlite::Connection::open(dir.join(DESKTOP_STATE_STORE_FILE)).unwrap();
    let transaction = gate
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let ctx = egui::Context::default();
    client.submit_draft_snapshot(&ctx, true);
    assert!(client.persistence.pending());
    client.pause_running_slots(&ctx);
    // Model an earlier accepted click while the real autosave writer is blocked,
    // without sleeping through a several-minute focus phase.
    let action = client.persistence.pending_timer_action.as_mut().unwrap();
    assert_eq!(action.kind, TimerActionKind::Pause);
    action.requested_at_epoch_millis = clicked_at;
    let action_id = action.id;
    let slot = client.data.slots.iter().find(|slot| slot.id == 1).unwrap();
    client.desktop_ui.pending_phase_settlement = Some(PendingTimerPhaseSettlement {
        workspace: client.background_job_workspace_fingerprint(),
        slot_id: 1,
        run_id: desktop_slot_running_identity(slot).unwrap().to_owned(),
        observed_at_epoch_millis: started_at + focus_millis,
    });
    assert!(now_millis() > started_at + focus_millis);
    client.pause_running_slots(&ctx);
    assert_eq!(
        client.persistence.pending_timer_action.as_ref().unwrap().id,
        action_id
    );
    transaction.rollback().unwrap();
    drop(gate);
    wait_for_timer_action(&mut client);
    assert!(!client.persistence.failed(), "{}", client.status);
    assert!(!client.slot_dirty);
    let slot = client.data.slots.iter().find(|slot| slot.id == 1).unwrap();
    assert!(slot.running_since_epoch_millis.is_none());
    assert_eq!(slot.title, "name saved before pause");
    assert_eq!(slot.accumulated_millis, clicked_at - started_at);
    assert_eq!(client.data.sessions.len(), 1);
    assert_eq!(client.data.sessions[0].ended_at_epoch_millis, clicked_at);
    assert_eq!(
        client.data.sessions[0].duration_millis,
        clicked_at - started_at
    );
    client.retry_pending_timer_phase_settlement();
    assert!(client.desktop_ui.pending_phase_settlement.is_none());
    let persisted = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    assert_eq!(persisted.slots, client.data.slots);
    assert_eq!(persisted.sessions, client.data.sessions);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_async_other_slot_start_settles_rest_and_reopens_exactly() {
    let dir = temp_test_dir("timer_other_slot_rest_reopen");
    let sample =
        app_data::start_slot_app_data_json(&app_data::default_app_data_json(1_000), 1, 1_000)
            .unwrap();
    let focus_millis = app_data::TimerProjector::parse(&sample)
        .unwrap()
        .project(1_000)
        .slots[0]
        .micro_break_phase_target_millis;
    let started_at = now_millis() - focus_millis - 60_000;
    let clicked_at = started_at + focus_millis + 5_000;
    let state = app_data::start_slot_app_data_json(
        &app_data::default_app_data_json(started_at),
        1,
        started_at,
    )
    .unwrap();
    let mut client = startup_performance_client(&dir, state);
    client.settings.timer_bell_enabled = false;
    client.ensure_selected_slot_draft();
    client.slot_note_draft = "keep the preceding autosave".into();
    client.mark_slot_dirty();
    let mut gate = rusqlite::Connection::open(dir.join(DESKTOP_STATE_STORE_FILE)).unwrap();
    let transaction = gate
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let ctx = egui::Context::default();
    client.submit_draft_snapshot(&ctx, true);
    let other = client
        .data
        .slots
        .iter()
        .find(|slot| slot.id == 2)
        .unwrap()
        .clone();
    client.toggle_slot(&other, &ctx);
    client
        .persistence
        .pending_timer_action
        .as_mut()
        .unwrap()
        .requested_at_epoch_millis = clicked_at;
    transaction.rollback().unwrap();
    drop(gate);
    wait_for_timer_action(&mut client);
    assert!(!client.persistence.failed(), "{}", client.status);
    let first = client.data.slots.iter().find(|slot| slot.id == 1).unwrap();
    let second = client.data.slots.iter().find(|slot| slot.id == 2).unwrap();
    assert_eq!(first.note, "keep the preceding autosave");
    assert_eq!(first.accumulated_millis, focus_millis);
    assert!(first.running_since_epoch_millis.is_some());
    assert_eq!(second.running_since_epoch_millis, Some(clicked_at));
    let projection = app_data::TimerProjector::parse(&client.state_json)
        .unwrap()
        .project(clicked_at);
    let first_view = projection.slots.iter().find(|slot| slot.id == 1).unwrap();
    assert_eq!(
        first_view.micro_break_phase,
        app_data::TimerViewPhase::Break
    );
    assert_eq!(first_view.micro_break_phase_progress_millis, 5_000);
    assert_eq!(client.data.sessions.len(), 1);
    assert_eq!(client.data.sessions[0].slot_id, 1);
    assert_eq!(client.data.sessions[0].started_at_epoch_millis, started_at);
    assert_eq!(
        client.data.sessions[0].ended_at_epoch_millis,
        started_at + focus_millis
    );
    assert_eq!(client.data.sessions[0].duration_millis, focus_millis);
    let expected_slots = client.data.slots.clone();
    let expected_sessions = client.data.sessions.clone();
    let persisted = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    assert_eq!(persisted.slots, expected_slots);
    assert_eq!(persisted.sessions, expected_sessions);
    drop(client);
    let reopened =
        TimerWindowsClient::load_from_root(dir.clone(), Arc::new(AtomicBool::new(false)));
    assert!(reopened.workspace_persistence_ready, "{}", reopened.status);
    assert_eq!(reopened.data.slots, expected_slots);
    assert_eq!(reopened.data.sessions, expected_sessions);
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_timer_only_receipts_retain_note_and_session_allocations() {
    let dir = temp_test_dir("timer_receipt_retains_note_storage");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_state_with_note("n", "retained", &"n".repeat(128 * 1024), None),
    );
    client.settings.timer_bell_enabled = false;
    client.save_state().unwrap();
    let notes = client.data.notes.as_ptr();
    let text = client.data.notes[0].content.as_ptr();
    let ctx = egui::Context::default();
    let slot = client.selected_slot().unwrap();
    client.toggle_slot(&slot, &ctx);
    wait_for_timer_action(&mut client);
    assert_eq!(notes, client.data.notes.as_ptr());
    assert_eq!(text, client.data.notes[0].content.as_ptr());
    client.pause_running_slots(&ctx);
    wait_for_timer_action(&mut client);
    assert_eq!(notes, client.data.notes.as_ptr());
    assert_eq!(text, client.data.notes[0].content.as_ptr());
    assert_eq!(client.data.sessions.len(), 1);
    let sessions = client.data.sessions.as_ptr();
    let slot = client.selected_slot().unwrap();
    client.toggle_slot(&slot, &ctx);
    wait_for_timer_action(&mut client);
    assert_eq!(sessions, client.data.sessions.as_ptr());
    assert_eq!(notes, client.data.notes.as_ptr());
    client.pause_running_slots(&ctx);
    wait_for_timer_action(&mut client);
    assert_eq!(client.data.sessions.len(), 2);
    assert_eq!(notes, client.data.notes.as_ptr());
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_timer_mutations_keep_unsaved_names_and_notes() {
    let dir = temp_test_dir("workspace_timer_save_barrier");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_data::default_app_data_json(now_millis()),
    );
    client.settings.timer_bell_enabled = false;
    let save_ctx = egui::Context::default();
    client.ensure_selected_slot_draft();
    client.slot_title_draft = "回归测试项目".to_string();
    client.slot_note_draft = "刚输入且尚未到自动保存时间".to_string();
    client.mark_slot_dirty();
    let old_slot = client.selected_slot().unwrap();
    client.toggle_slot(&old_slot, &save_ctx);
    wait_for_timer_action(&mut client);
    assert_eq!(client.selected_slot().unwrap().title, "回归测试项目");
    client.slot_note_draft = "暂停前补充".to_string();
    client.mark_slot_dirty();
    client.pause_running_slots(&save_ctx);
    wait_for_timer_action(&mut client);
    assert_eq!(client.selected_slot().unwrap().note, "暂停前补充");
    client.slot_note_draft = "重置前补充".to_string();
    client.mark_slot_dirty();
    client.reset_slot(1);
    assert_eq!(client.selected_slot().unwrap().note, "重置前补充");
    let persisted = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    assert_eq!(persisted.slots[0].title, "回归测试项目");
    assert_eq!(persisted.slots[0].note, "重置前补充");
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_category_order_and_archive_roundtrip_preserve_other_slots() {
    let dir = temp_test_dir("workspace_archive_roundtrip");
    let now = now_millis();
    let state = app_data::update_slot_title_app_data_json(
        &app_data::default_app_data_json(now),
        1,
        "读书",
        now,
    )
    .unwrap();
    let state =
        app_data::add_category_and_assign_app_data_json(&state, 1, "study", "学习", now).unwrap();
    let state = app_data::update_slot_title_app_data_json(&state, 2, "不能覆盖", now).unwrap();
    let mut client = test_client_for_account_scope(&dir, "account-a", state);
    let assigned_category = client.data.slots[0].category_id.clone();
    client.move_slot_in_order(1, 1);
    client.refresh_timer_projection(now);
    assert_eq!(
        &client.desktop_ui.projection.as_ref().unwrap().slot_order[..2],
        &[2, 1]
    );
    client.archive_slot(1);
    assert_eq!(client.data.archived_tasks.len(), 1);
    let id = client.data.archived_tasks[0].id.clone();
    client.restore_timer_archive(&id);
    assert!(client.data.archived_tasks.is_empty());
    assert_eq!(client.data.slots[0].title, "读书");
    assert_eq!(client.data.slots[0].category_id, assigned_category);
    assert_eq!(client.data.slots[1].title, "不能覆盖");
    assert!(client.desktop_ui.slot_editor_open);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_readonly_failure_keeps_draft_and_does_not_start_timer() {
    let dir = temp_test_dir("workspace_failed_save");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_data::default_app_data_json(now_millis()),
    );
    client.settings.timer_bell_enabled = false;
    let save_ctx = egui::Context::default();
    client.ensure_selected_slot_draft();
    client.slot_title_draft = "保留这个草稿".to_string();
    client.mark_slot_dirty();
    client.workspace_persistence_ready = false;
    let slot = client.selected_slot().unwrap();
    client.toggle_slot(&slot, &save_ctx);
    assert_eq!(client.slot_title_draft, "保留这个草稿");
    assert!(client.slot_dirty);
    assert!(client
        .selected_slot()
        .unwrap()
        .running_since_epoch_millis
        .is_none());
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_ordinary_sync_allows_editing_but_account_replacement_is_locked() {
    let dir = temp_test_dir("workspace_sync_editing");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_data::default_app_data_json(now_millis()),
    );
    client.settings.timer_bell_enabled = false;
    let save_ctx = egui::Context::default();
    for kind in [SyncTaskKind::Sync, SyncTaskKind::Health] {
        client.sync_task = Some(SyncTaskState {
            kind,
            phase: SyncTaskPhase::AppData,
            started_at_epoch_millis: 1,
        });
        assert!(!client.workspace_edit_locked());
    }
    client.sync_task.as_mut().unwrap().kind = SyncTaskKind::Sync;
    let slot = client.selected_slot().unwrap();
    client.toggle_slot(&slot, &save_ctx);
    wait_for_timer_action(&mut client);
    assert!(client.sync_state_changed_since_request);
    assert!(client
        .selected_slot()
        .unwrap()
        .running_since_epoch_millis
        .is_some());
    for kind in [
        SyncTaskKind::Download,
        SyncTaskKind::Login,
        SyncTaskKind::Register,
        SyncTaskKind::Upload,
    ] {
        client.sync_task.as_mut().unwrap().kind = kind;
        assert!(client.workspace_edit_locked());
    }
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_all_pages_layout_at_supported_window_sizes() {
    let dir = temp_test_dir("workspace_layout_sizes");
    let state = app_state_with_note("layout-note", "布局测试便签", "只用于测试的内容", None);
    let mut client = test_client_for_account_scope(&dir, "account-a", state);
    client.settings.timer_bell_enabled = false;
    let ctx = workspace_test_context();
    for size in [
        egui::vec2(760.0, 480.0),
        egui::vec2(900.0, 600.0),
        egui::vec2(1240.0, 800.0),
        egui::vec2(1920.0, 1080.0),
    ] {
        for tab in AppTab::ALL {
            client.switch_tab(tab);
            for _ in 0..2 {
                workspace_test_frame(&mut client, &ctx, size, vec![]);
            }
            let output = workspace_test_frame(&mut client, &ctx, size, vec![]);
            assert!(!output.shapes.is_empty());
            for shape in &output.shapes {
                if let egui::epaint::Shape::Text(text) = &shape.shape {
                    let rect = text.visual_bounding_rect();
                    if shape.clip_rect.intersects(rect)
                        && rect.top() >= 0.0
                        && rect.bottom() <= size.y
                    {
                        assert!(
                            rect.left() >= -1.0 && rect.right() <= size.x + 1.0,
                            "{}: visible text exceeds window {:?}: {:?}",
                            tab.title(),
                            size,
                            text.galley.text()
                        );
                    }
                }
            }
        }
    }
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_local_day_windows_have_exact_midnight_boundaries() {
    for (y, m, d) in [(1970, 1, 1), (2000, 2, 29), (2026, 9, 8), (2025, 12, 31)] {
        let day = desktop_days_from_civil(y, m, d);
        assert_eq!(utc_ymd_from_epoch_millis(day * 86_400_000), (y, m, d));
        let start = desktop_local_midnight(day);
        let (today, tomorrow) = desktop_local_day_window(start + 3_600_000, 0);
        assert_eq!(today, start);
        assert!(tomorrow > today);
        assert_eq!(desktop_local_day_window(tomorrow, 0).0, tomorrow);
        assert!(desktop_local_timestamp(start).contains("00:00"));
    }
}

#[test]
fn workspace_multi_block_sticky_edits_preserve_ids_and_other_blocks() {
    let dir = temp_test_dir("workspace_multiblock_note");
    let mut note = desktop_note_save_value(
        "multi",
        DesktopNoteKind::Sticky,
        "分段便签",
        "第一段\n第二段",
        None,
        100,
    );
    note["document"]["blocks"] = json!([
        {"id":"first", "type":"TEXT", "text":"第一段"},
        {"id":"second", "type":"TEXT", "text":"第二段"}
    ]);
    let state = app_data::upsert_note_app_data_json(
        &app_data::default_app_data_json(100),
        &note.to_string(),
        100,
    )
    .unwrap();
    let mut client = test_client_for_account_scope(&dir, "account-a", state);
    client.switch_tab(AppTab::Notes);
    let note = client
        .data
        .notes
        .iter()
        .find(|n| n.id == "multi")
        .unwrap()
        .clone();
    client.select_note(&note);
    let first = client.note_blocks_draft[0].clone();
    client.apply_note_block_text_edit(&first, "第一段已修改".to_string());
    client.flush_note_draft().unwrap();
    let saved = client.data.notes.iter().find(|n| n.id == "multi").unwrap();
    assert_eq!(saved.document.blocks.len(), 2);
    assert_eq!(saved.document.blocks[0].id, "first");
    assert_eq!(saved.document.blocks[0].text, "第一段已修改");

    assert_eq!(saved.document.blocks[1].text, "第二段");
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_multi_block_sticky_keyboard_edit_preserves_undo_and_siblings() {
    let dir = temp_test_dir("workspace_multiblock_keyboard");
    let mut note = desktop_note_save_value(
        "multi-keyboard",
        DesktopNoteKind::Sticky,
        "分段便签",
        "第一段\n第二段",
        None,
        100,
    );
    note["document"]["blocks"] = json!([
        {"id":"first", "type":"TEXT", "text":"第一段"},
        {"id":"second", "type":"TEXT", "text":"第二段"}
    ]);
    let state = app_data::upsert_note_app_data_json(
        &app_data::default_app_data_json(100),
        &note.to_string(),
        100,
    )
    .unwrap();
    let mut client = test_client_for_account_scope(&dir, "account-a", state);
    client.switch_tab(AppTab::Notes);
    client.select_note_by_id("multi-keyboard");
    let ctx = workspace_test_context();
    let size = egui::vec2(1240.0, 900.0);
    for _ in 0..3 {
        workspace_test_frame(&mut client, &ctx, size, vec![]);
    }
    let editor_id = egui::Id::new((
        "knowledge_block_editor",
        client.desktop_ui.navigation.editor_epoch,
        "second",
    ));
    ctx.memory_mut(|memory| memory.request_focus(editor_id));
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![egui::Event::Text("追加".into())],
    );
    assert_eq!(client.note_blocks_draft[0].text, "第一段");
    assert_eq!(client.note_blocks_draft[1].text, "第二段追加");
    assert!(client.note_dirty);
    client.undo_note_draft();
    assert_eq!(client.note_blocks_draft[0].text, "第一段");
    assert_eq!(client.note_blocks_draft[1].text, "第二段");
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_history_draws_only_visible_rows_and_reuses_parsed_index() {
    let dir = temp_test_dir("workspace_history_virtual_rows");
    let now = now_millis();
    let mut raw: Value = serde_json::from_str(&app_data::default_app_data_json(now)).unwrap();
    raw["sessions"] = Value::Array((0..1200).map(|i| json!({
        "id":format!("history-{i}"), "slotId":1, "slotTitle":format!("记录 {i}"),
        "startedAtEpochMillis":now-2_000-i*3_000, "endedAtEpochMillis":now-1_000-i*3_000, "durationMillis":1000
    })).collect());
    let mut client = test_client_for_account_scope(&dir, "account-a", raw.to_string());
    client.tab = AppTab::History;
    let ctx = workspace_test_context();
    for _ in 0..3 {
        workspace_test_frame(&mut client, &ctx, egui::vec2(1240.0, 800.0), vec![]);
    }
    let cached = client.desktop_ui.history_index.clone().unwrap();
    let output = workspace_test_frame(&mut client, &ctx, egui::vec2(1240.0, 800.0), vec![]);
    assert_eq!(client.desktop_ui.history_session_rows.len(), 1200);
    assert!(Arc::ptr_eq(
        &cached,
        client.desktop_ui.history_index.as_ref().unwrap()
    ));
    assert!(
        output.shapes.len() < 500,
        "off-screen rows must not be laid out: {} shapes",
        output.shapes.len()
    );
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_completed_focus_is_recorded_while_timer_remains_running() {
    let dir = temp_test_dir("workspace_live_history");
    let now = now_millis();
    let state = app_data::start_slot_app_data_json(
        &app_data::default_app_data_json(now - 600_000),
        1,
        now - 600_000,
    )
    .unwrap();
    let mut client = test_client_for_account_scope(&dir, "account-a", state);
    client.settings.timer_bell_enabled = false;
    client.refresh_timer_projection(now);
    let elapsed_before = client.projected_elapsed(1);
    client.settle_timer_phase(1);
    assert!(client.data.sessions.is_empty());
    workspace_commit_pending_save(&mut client);
    assert!(!client.data.sessions.is_empty());
    assert!(client.data.slots[0].running_since_epoch_millis.is_some());
    assert!(
        client
            .projected_elapsed(1)
            .saturating_sub(elapsed_before)
            .abs()
            < 5_000
    );
    let count = client.data.sessions.len();
    client.settle_timer_phase(1);
    workspace_commit_pending_save(&mut client);
    assert_eq!(
        count,
        client.data.sessions.len(),
        "settlement must not duplicate completed phases"
    );
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

fn workspace_test_context() -> egui::Context {
    let ctx = egui::Context::default();
    install_ui_fonts(&ctx);
    install_ui_style(&ctx);
    ctx
}

#[test]
fn workspace_ctrl_f_focuses_search_and_typing_never_starts_a_timer() {
    let dir = temp_test_dir("workspace_keyboard_search");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_data::default_app_data_json(now_millis()),
    );
    client.settings.timer_bell_enabled = false;
    let ctx = workspace_test_context();
    let size = egui::vec2(1240.0, 800.0);
    for _ in 0..2 {
        workspace_test_frame(&mut client, &ctx, size, vec![]);
    }
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![egui::Event::Key {
            key: egui::Key::F,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::CTRL,
        }],
    );
    assert!(ctx.memory(|m| m.has_focus(egui::Id::new("board_search"))));
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![egui::Event::Text("学习 计划".to_string())],
    );
    assert_eq!(client.desktop_ui.board_query, "学习 计划");
    assert!(client
        .data
        .slots
        .iter()
        .all(|s| s.running_since_epoch_millis.is_none()));
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_phase_settlement_retries_after_locked_sync_and_real_pending_receipt() {
    let dir = temp_test_dir("workspace_phase_settlement_retry");
    let now = now_millis();
    let state = app_data::start_slot_app_data_json(
        &app_data::default_app_data_json(now - 600_000),
        1,
        now - 600_000,
    )
    .unwrap();
    let mut client = test_client_for_account_scope(&dir, "account-a", state);
    client.settings.timer_bell_enabled = false;
    client.open_slot_editor(1);
    assert_eq!(client.slot_draft_id, 1);
    client.slot_title_draft = "阶段切换时的标题草稿".to_string();
    client.mark_slot_dirty();
    workspace_submit_draft_and_wait_for_receipt(&mut client);
    client.refresh_timer_projection(now);
    let view = client
        .desktop_ui
        .projection
        .as_ref()
        .unwrap()
        .slots
        .iter()
        .find(|slot| slot.id == 1)
        .unwrap();
    let previous_phase = if view.micro_break_phase == app_data::TimerViewPhase::Focus {
        app_data::TimerViewPhase::Break
    } else {
        app_data::TimerViewPhase::Focus
    };
    client.desktop_ui.phase_markers.insert(
        1,
        (
            view.active_run_id.clone(),
            previous_phase,
            view.micro_break_cycle_index,
        ),
    );
    client.sync_task = Some(SyncTaskState {
        kind: SyncTaskKind::Download,
        phase: SyncTaskPhase::AppData,
        started_at_epoch_millis: now,
    });
    client.check_timer_bells();
    assert!(client.persistence.pending());
    assert!(client.desktop_ui.pending_phase_settlement.is_some());
    assert!(client.data.sessions.is_empty());
    let marker = client.desktop_ui.phase_markers.get(&1).unwrap().clone();
    client.sync_task = None;
    client.poll_draft_persistence();
    client.check_timer_bells();
    assert!(client.desktop_ui.pending_phase_settlement.is_some());
    workspace_commit_pending_save(&mut client);
    assert!(!client.persistence.pending());
    assert!(client.desktop_ui.pending_phase_settlement.is_none());
    assert_eq!(
        client
            .data
            .slots
            .iter()
            .find(|slot| slot.id == 1)
            .unwrap()
            .title,
        "阶段切换时的标题草稿"
    );
    assert!(
        !client.data.sessions.is_empty(),
        "the same phase must retry after the save barrier clears"
    );
    assert_eq!(client.desktop_ui.phase_markers.get(&1).unwrap(), &marker);
    let sessions = client.data.sessions.len();
    for _ in 0..3 {
        client.check_timer_bells();
    }
    assert_eq!(
        client.data.sessions.len(),
        sessions,
        "retry must not duplicate focus history"
    );
    assert!(client
        .data
        .slots
        .iter()
        .find(|slot| slot.id == 1)
        .unwrap()
        .running_since_epoch_millis
        .is_some());
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_pending_phase_settlement_cannot_restart_paused_timer_or_cross_workspace() {
    let dir = temp_test_dir("workspace_phase_settlement_scope");
    let now = now_millis();
    let state = app_data::start_slot_app_data_json(
        &app_data::default_app_data_json(now - 600_000),
        1,
        now - 600_000,
    )
    .unwrap();
    let mut client = test_client_for_account_scope(&dir, "account-a", state);
    client.settings.timer_bell_enabled = false;
    let slot = client.data.slots.iter().find(|slot| slot.id == 1).unwrap();
    let pending = PendingTimerPhaseSettlement {
        workspace: client.background_job_workspace_fingerprint(),
        slot_id: 1,
        run_id: desktop_slot_running_identity(slot).unwrap().to_string(),
        observed_at_epoch_millis: now,
    };
    client.desktop_ui.pending_phase_settlement = Some(PendingTimerPhaseSettlement {
        workspace: "different-workspace".into(),
        ..pending.clone()
    });
    let version = client.data_version;
    client.retry_pending_timer_phase_settlement();
    assert!(client.desktop_ui.pending_phase_settlement.is_none());
    assert_eq!(client.data_version, version);
    client.desktop_ui.pending_phase_settlement = Some(pending);
    assert!(client.replace_state(
        app_data::pause_slots_app_data_json(&client.state_json, &[1], now),
        "paused"
    ));
    let sessions = client.data.sessions.len();
    client.retry_pending_timer_phase_settlement();
    assert!(client.desktop_ui.pending_phase_settlement.is_none());
    assert!(!client.settle_timer_phase(1));
    assert!(client
        .data
        .slots
        .iter()
        .find(|slot| slot.id == 1)
        .unwrap()
        .running_since_epoch_millis
        .is_none());
    assert_eq!(client.data.sessions.len(), sessions);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

fn workspace_submit_draft_and_wait_for_receipt(client: &mut TimerWindowsClient) {
    let ctx = egui::Context::default();
    for _ in 0..5 {
        let _ = ctx.run(egui::RawInput::default(), |_| {});
    }
    assert!(!ctx.has_requested_repaint());
    let (wake_tx, wake_rx) = mpsc::channel();
    ctx.set_request_repaint_callback(move |_| {
        let _ = wake_tx.send(());
    });
    let version = client.data_version;
    client.submit_draft_snapshot(&ctx, true);
    assert!(client.persistence.pending(), "{}", client.status);
    // LatestWorker sends the real receipt before requesting this repaint.
    // Do not poll here: the operation under test must acknowledge it itself.
    wake_rx.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(client.persistence.pending());
    assert_eq!(client.data_version, version);
}

#[test]
fn workspace_phase_save_stays_pending_behind_database_lock_and_blocks_conflicting_commit() {
    let dir = temp_test_dir("workspace_phase_background_lock");
    let now = now_millis();
    let state = app_data::start_slot_app_data_json(
        &app_data::default_app_data_json(now - 600_000),
        1,
        now - 600_000,
    )
    .unwrap();
    let mut client = test_client_for_account_scope(&dir, "account-a", state);
    client.save_state().unwrap();
    let mut gate = rusqlite::Connection::open(
        client
            .sync_path
            .parent()
            .unwrap()
            .join(DESKTOP_STATE_STORE_FILE),
    )
    .unwrap();
    let transaction = gate
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let before = client.state_json.clone();
    assert!(client.settle_timer_phase(1));
    client.submit_draft_snapshot(&egui::Context::default(), false);
    assert!(
        client.persistence.pending(),
        "the GUI must return while disk work is blocked"
    );
    assert_eq!(client.state_json, before);
    client.switch_tab(AppTab::Knowledge);
    assert!(
        client.tab == AppTab::Knowledge,
        "clean navigation must remain available"
    );
    assert_eq!(
        client.save_state().unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    assert_eq!(
        client.state_json, before,
        "pending receipts must protect the committed base"
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    while client.persistence.pending() && Instant::now() < deadline {
        client.poll_draft_persistence();
        thread::sleep(Duration::from_millis(5));
    }
    assert!(!client.persistence.pending());
    assert_eq!(client.state_json, before);
    assert!(client.desktop_ui.pending_phase_settlement.is_some());
    transaction.rollback().unwrap();
    drop(gate);
    let ctx = egui::Context::default();
    let deadline = Instant::now() + Duration::from_secs(30);
    while client.desktop_ui.pending_phase_settlement.is_some() && Instant::now() < deadline {
        client.poll_draft_persistence();
        client.flush_due_saves(&ctx);
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        client.desktop_ui.pending_phase_settlement.is_none(),
        "{}",
        client.status
    );
    assert!(!client.data.sessions.is_empty());
    assert!(client.data.slots[0].running_since_epoch_millis.is_some());
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_old_phase_receipt_preserves_a_newer_pending_phase() {
    let dir = temp_test_dir("workspace_phase_receipt_order");
    let now = now_millis();
    let state = app_data::start_slot_app_data_json(
        &app_data::default_app_data_json(now - 600_000),
        1,
        now - 600_000,
    )
    .unwrap();
    let mut client = test_client_for_account_scope(&dir, "account-a", state);
    assert!(client.settle_timer_phase(1));
    workspace_submit_draft_and_wait_for_receipt(&mut client);
    let mut newer = client.desktop_ui.pending_phase_settlement.clone().unwrap();
    newer.observed_at_epoch_millis += 1;
    client.desktop_ui.pending_phase_settlement = Some(newer.clone());
    client.poll_draft_persistence();
    assert_eq!(
        client.desktop_ui.pending_phase_settlement.as_ref(),
        Some(&newer)
    );
    let completed = client.data.sessions.len();
    workspace_commit_pending_save(&mut client);
    assert!(client.desktop_ui.pending_phase_settlement.is_none());
    assert_eq!(client.data.sessions.len(), completed);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

fn workspace_commit_pending_save(client: &mut TimerWindowsClient) {
    client.submit_draft_snapshot(&egui::Context::default(), true);
    let deadline = Instant::now() + Duration::from_secs(30);
    while client.persistence.pending() && Instant::now() < deadline {
        client.poll_draft_persistence();
        thread::sleep(Duration::from_millis(5));
    }
    assert!(!client.persistence.pending(), "save worker did not finish");
    assert!(
        client.desktop_ui.pending_phase_settlement.is_none(),
        "{}",
        client.status
    );
}

#[test]
fn workspace_local_theme_keeps_note_receipt_acknowledged_inside_barrier() {
    let dir = temp_test_dir("workspace_theme_receipt_barrier");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_state_with_note("receipt-note", "before", "body", None),
    );
    client.select_note_by_id("receipt-note");
    client.note_title_draft = "saved by the worker".into();
    client.mark_note_dirty();
    let next = app_data::set_theme_mode_app_data_json(&client.state_json, 2, now_millis());
    workspace_submit_draft_and_wait_for_receipt(&mut client);
    assert_eq!(client.data.notes[0].title, "before");
    assert_eq!(
        decode_data(&fs::read_to_string(&client.state_path).unwrap()).notes[0].title,
        "saved by the worker"
    );

    let outcome = client.replace_state_with_source_outcome(next, "saved", "local_save");

    assert!(outcome.is_committed(), "{}", client.status);
    assert!(!client.note_dirty);
    assert!(!client.persistence.pending());
    assert_eq!(client.data.notes[0].title, "saved by the worker");
    assert_eq!(client.data.theme_mode, "DARK");
    let disk = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    assert_eq!(disk.notes[0].title, "saved by the worker");
    assert_eq!(disk.theme_mode, "DARK");
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_local_category_keeps_same_slot_fields_from_async_receipt() {
    let dir = temp_test_dir("workspace_slot_receipt_barrier");
    let mut client =
        test_client_for_account_scope(&dir, "account-a", app_data::default_app_data_json(100));
    client.ensure_selected_slot_draft();
    client.slot_title_draft = "asynchronous title".into();
    client.slot_note_draft = "asynchronous description".into();
    client.mark_slot_dirty();
    let next = app_data::add_category_and_assign_app_data_json(
        &client.state_json,
        1,
        "receipt-category",
        "收据分类",
        now_millis(),
    )
    .unwrap();
    let category = decode_data(&next).slots[0].category_id.clone();
    assert!(category.is_some());
    workspace_submit_draft_and_wait_for_receipt(&mut client);

    assert!(
        client.replace_state(Some(next), "saved"),
        "{}",
        client.status
    );

    assert!(!client.slot_dirty);
    assert_eq!(client.data.slots[0].title, "asynchronous title");
    assert_eq!(client.data.slots[0].note, "asynchronous description");
    assert_eq!(client.data.slots[0].category_id, category);
    let disk = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    assert_eq!(disk.slots[0].title, "asynchronous title");
    assert_eq!(disk.slots[0].note, "asynchronous description");
    assert_eq!(disk.slots[0].category_id, category);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_failed_barrier_merge_keeps_the_durable_receipt() {
    let dir = temp_test_dir("workspace_invalid_receipt_merge");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_state_with_note("receipt-note", "before", "body", None),
    );
    client.select_note_by_id("receipt-note");
    client.note_title_draft = "durable draft".into();
    client.mark_note_dirty();
    workspace_submit_draft_and_wait_for_receipt(&mut client);
    let durable = fs::read_to_string(&client.state_path).unwrap();

    let outcome = client.replace_state_with_source_outcome(
        Some("invalid-json".into()),
        "must not commit",
        "local_save",
    );

    assert!(!outcome.is_committed());
    assert!(client.status.contains("安全合并"));
    assert_eq!(client.state_json, durable);
    assert_eq!(fs::read_to_string(&client.state_path).unwrap(), durable);
    assert_eq!(client.data.notes[0].title, "durable draft");
    assert!(!client.note_dirty);
    assert!(!client.persistence.pending());
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_verified_download_retains_exact_replacement_semantics() {
    let dir = temp_test_dir("workspace_verified_receipt_replace");
    let initial = app_state_with_note("receipt-note", "server snapshot", "body", None);
    let mut client = test_client_for_account_scope(&dir, "account-a", initial.clone());
    client.select_note_by_id("receipt-note");
    client.note_title_draft = "local draft".into();
    client.mark_note_dirty();
    workspace_submit_draft_and_wait_for_receipt(&mut client);

    let outcome = client.replace_state_with_source_outcome(
        Some(initial),
        "verified replacement",
        "verified_download",
    );

    assert!(outcome.is_committed(), "{}", client.status);
    assert_eq!(client.data.notes[0].title, "server snapshot");
    assert_eq!(client.note_title_draft, "server snapshot");
    assert_eq!(
        decode_data(&fs::read_to_string(&client.state_path).unwrap()).notes[0].title,
        "server snapshot"
    );
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_sync_autosaves_and_retains_response_until_receipt_is_acknowledged() {
    let dir = temp_test_dir("workspace_sync_async_receipt");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_state_with_note("receipt-note", "before", "body", None),
    );
    client.select_note_by_id("receipt-note");
    let remote = app_data::set_theme_mode_app_data_json(&client.state_json, 2, now_millis());
    let response = sync_core::SyncClientResult {
        ok: true,
        current_committed: true,
        user_id: client.sync.user_id.clone(),
        token_id: client.sync.token_id.clone(),
        server_instance_id: client.sync.server_instance_id.clone(),
        account_namespace: client.sync.account_namespace.clone(),
        app_data_json: remote,
        current_generation: 0,
        mode: "merged".into(),
        ..Default::default()
    };
    let identity = bound_result_expectation(&client);
    client.sync_task = Some(SyncTaskState {
        kind: SyncTaskKind::Sync,
        phase: SyncTaskPhase::AppData,
        started_at_epoch_millis: now_millis(),
    });
    client.note_title_draft = "edited during ordinary sync".into();
    client.mark_note_dirty();
    assert!(client.sync_state_changed_since_request);
    workspace_submit_draft_and_wait_for_receipt(&mut client);
    let (tx, rx) = mpsc::channel();
    let background_job_id = client
        .stage_background_job(BackgroundJobKind::Sync)
        .unwrap();
    tx.send(SyncTaskResult {
        background_job_id: Some(background_job_id),
        kind: SyncTaskKind::Sync,
        results: vec![SyncTaskStepResult {
            raw: serde_json::to_string(&response).unwrap(),
            keep_password: false,
            identity,
        }],
    })
    .unwrap();
    client.sync_result_rx = Some(rx);

    client.poll_sync_result();
    assert!(client.sync_result_rx.is_some());
    assert!(client.sync_task.is_some());
    assert_eq!(client.data.notes[0].title, "before");
    client.poll_draft_persistence();
    assert!(!client.note_dirty);
    assert!(!client.persistence.pending());
    // Accept the already completed response while closing, without launching
    // a real network attachment task from this isolated regression test.
    client.shutdown_state = ClientShutdownState::Draining {
        started: Instant::now(),
    };
    client.poll_sync_result();

    assert!(client.sync_result_rx.is_none());
    wait_for_external_workspace_receipt(&mut client);
    assert!(client.sync_task.is_none());
    assert_eq!(client.data.notes[0].title, "edited during ordinary sync");
    assert_eq!(client.data.theme_mode, "DARK", "{}", client.status);
    assert!(client.startup_sync_due_epoch_millis > 0);
    let disk = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    assert_eq!(disk.notes[0].title, "edited during ordinary sync");
    assert_eq!(disk.theme_mode, "DARK");
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_media_sync_retains_response_until_receipt_is_acknowledged() {
    let dir = temp_test_dir("workspace_media_async_receipt");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_state_with_note("receipt-note", "before", "body", None),
    );
    client.select_note_by_id("receipt-note");
    let task = SyncTaskState {
        kind: SyncTaskKind::Sync,
        phase: SyncTaskPhase::NoteMedia,
        started_at_epoch_millis: now_millis(),
    };
    client.sync_task = Some(task);
    client.note_title_draft = "edited during media sync".into();
    client.mark_note_dirty();
    workspace_submit_draft_and_wait_for_receipt(&mut client);
    let (tx, rx) = mpsc::channel();
    let background_job_id = client
        .stage_background_job(BackgroundJobKind::MediaSync)
        .unwrap();
    tx.send(NoteMediaSyncTaskResult {
        background_job_id: Some(background_job_id),
        task,
        origin_workspace: client.ai_workspace_identity(),
        base_status: "media response retained".into(),
        result: Ok(Default::default()),
    })
    .unwrap();
    client.note_media_sync_result_rx = Some(rx);

    client.poll_note_media_sync_result();
    assert!(client.note_media_sync_result_rx.is_some());
    assert_eq!(client.sync_task, Some(task));
    client.poll_draft_persistence();
    client.poll_note_media_sync_result();

    assert!(client.note_media_sync_result_rx.is_none());
    wait_for_external_workspace_receipt(&mut client);
    assert!(client.sync_task.is_none());
    assert_eq!(client.data.notes[0].title, "edited during media sync");
    let disk = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    assert_eq!(disk.notes[0].title, "edited during media sync");
    assert!(client.startup_sync_due_epoch_millis > 0);
    assert!(!client.sync_state_changed_since_request);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_capability_reset_captures_state_after_async_receipt() {
    let dir = temp_test_dir("workspace_capability_async_receipt");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_state_with_note("receipt-note", "before", "body", None),
    );
    let server_id = client.sync.server_instance_id.clone();
    let namespace = client.sync.account_namespace.clone();
    let user_id = client.sync.user_id.clone();
    ensure_workspace_checkpoint(&mut client.sync, &server_id, &namespace, &user_id);
    let workspace_id =
        workspace_checkpoint_for_account(&client.sync, &server_id, &namespace, &user_id)
            .workspace_id;
    remember_account_generation_and_capability(
        &mut client.sync,
        &server_id,
        &namespace,
        &user_id,
        0,
        &workspace_id,
        &"e".repeat(64),
    );
    client.save_state().unwrap();
    let identity = bound_result_expectation(&client);
    let response = sync_core::SyncClientResult {
        ok: false,
        user_id: user_id.clone(),
        token_id: client.sync.token_id.clone(),
        server_instance_id: server_id.clone(),
        account_namespace: namespace.clone(),
        current_generation: 0,
        mode: "workspace_proof_invalid".into(),
        ..Default::default()
    };
    client.select_note_by_id("receipt-note");
    client.note_title_draft = "survives proof reset".into();
    client.mark_note_dirty();
    workspace_submit_draft_and_wait_for_receipt(&mut client);

    let applied =
        client.apply_sync_result(&serde_json::to_string(&response).unwrap(), false, &identity);

    assert_eq!(applied.mode, "workspace_proof_reset", "{}", client.status);
    assert_eq!(client.data.notes[0].title, "survives proof reset");
    assert_eq!(
        decode_data(&fs::read_to_string(&client.state_path).unwrap()).notes[0].title,
        "survives proof reset"
    );
    assert!(
        workspace_checkpoint_for_account(&client.sync, &server_id, &namespace, &user_id,)
            .workspace_capability
            .is_empty()
    );
    assert!(!client.note_dirty);
    assert!(!client.persistence.pending());
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_keyboard_edit_and_new_note_focus_real_text_inputs() {
    let dir = temp_test_dir("workspace_edit_keyboard_focus");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_data::default_app_data_json(now_millis()),
    );
    let ctx = workspace_test_context();
    let size = egui::vec2(1240.0, 800.0);
    let key = |key, modifiers| egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    };
    for _ in 0..2 {
        workspace_test_frame(&mut client, &ctx, size, vec![]);
    }
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![key(egui::Key::F2, egui::Modifiers::NONE)],
    );
    workspace_test_frame(&mut client, &ctx, size, vec![]);
    assert!(client.desktop_ui.slot_editor_open);
    assert!(ctx.memory(|m| m.has_focus(egui::Id::new("slot_title_editor"))));
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![egui::Event::Text("键盘录入名称".into())],
    );
    assert_eq!(client.slot_title_draft, "键盘录入名称");
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![key(egui::Key::S, egui::Modifiers::CTRL)],
    );
    assert_eq!(client.data.slots[0].title, "键盘录入名称");
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
    );
    assert!(!client.desktop_ui.slot_editor_open);
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![key(egui::Key::Num2, egui::Modifiers::CTRL)],
    );
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![key(egui::Key::N, egui::Modifiers::CTRL)],
    );
    workspace_test_frame(&mut client, &ctx, size, vec![]);
    assert!(ctx.memory(|m| m.has_focus(egui::Id::new("note_title_editor"))));
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![egui::Event::Text("键盘新便签".into())],
    );
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![key(egui::Key::S, egui::Modifiers::CTRL)],
    );
    assert!(client.data.notes.iter().any(|n| n.title == "键盘新便签"));
    for tab in [AppTab::Notes, AppTab::Knowledge] {
        client.switch_tab(tab);
        workspace_test_frame(
            &mut client,
            &ctx,
            size,
            vec![key(egui::Key::F, egui::Modifiers::CTRL)],
        );
        assert!(ctx.memory(|m| m.has_focus(egui::Id::new("notes_search"))));
    }
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

include!("history_ui_tests.rs");

#[test]
fn workspace_visible_timer_buttons_start_pause_and_keep_edit_separate() {
    let dir = temp_test_dir("workspace_visible_timer_buttons");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_data::default_app_data_json(now_millis()),
    );
    client.settings.timer_bell_enabled = false;
    let ctx = workspace_test_context();
    let size = egui::vec2(900.0, 600.0);
    for _ in 0..3 {
        workspace_test_frame(&mut client, &ctx, size, vec![]);
    }
    let toggle = ctx
        .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("timer_toggle", 1))))
        .unwrap();
    let edit = ctx
        .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("timer_edit", 1))))
        .unwrap();
    assert!(!toggle.intersects(edit));
    workspace_click(&mut client, &ctx, size, toggle.center());
    wait_for_timer_action(&mut client);
    assert!(client.data.slots[0].running_since_epoch_millis.is_some());
    assert!(!client.desktop_ui.slot_editor_open);
    workspace_click(&mut client, &ctx, size, toggle.center());
    wait_for_timer_action(&mut client);
    assert!(client.data.slots[0].running_since_epoch_millis.is_none());
    assert!(client
        .data
        .slots
        .iter()
        .all(|slot| slot.running_since_epoch_millis.is_none()));
    let persisted = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    assert!(persisted.slots[0].running_since_epoch_millis.is_none());
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_timer_reset_rejects_changed_record_and_account() {
    let dir = temp_test_dir("workspace_timer_reset_identity");
    let now = now_millis();
    let mut state: Value = serde_json::from_str(&app_data::default_app_data_json(now)).unwrap();
    state["slots"][0]["title"] = Value::from("原项目");
    state["slots"][0]["accumulatedMillis"] = Value::from(120_000);
    let mut client = test_client_for_account_scope(&dir, "account-a", state.to_string());
    client.request_timer_reset(1);
    assert!(client.desktop_ui.pending_reset.is_some());
    assert_eq!(client.data.slots[0].accumulated_millis, 120_000);
    assert!(client.replace_state(
        app_data::update_slot_title_app_data_json(
            &client.state_json,
            1,
            "另一设备修改的项目",
            now + 1
        ),
        "测试并发修改",
    ));
    assert!(!client.confirm_timer_reset());
    assert!(client.desktop_ui.pending_reset.is_none());
    assert_eq!(client.data.slots[0].accumulated_millis, 120_000);
    client.request_timer_reset(1);
    let original_account = client.sync.user_id.clone();
    client.sync.user_id = "account-b".into();
    assert!(!client.confirm_timer_reset());
    client.sync.user_id = original_account;
    assert_eq!(client.data.slots[0].accumulated_millis, 120_000);
    assert!(client.desktop_ui.pending_reset.is_none());
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_timer_reset_keeps_confirmation_after_save_failure_and_preserves_drafts() {
    let dir = temp_test_dir("workspace_timer_reset_save_failure");
    let now = now_millis();
    let mut state: Value = serde_json::from_str(&app_data::default_app_data_json(now)).unwrap();
    state["slots"][0]["accumulatedMillis"] = Value::from(120_000);
    let mut client = test_client_for_account_scope(&dir, "account-a", state.to_string());
    client.ensure_selected_slot_draft();
    client.slot_title_draft = "重置仍须保留名称".into();
    client.slot_note_draft = "重置仍须保留备注".into();
    client.mark_slot_dirty();
    client.request_timer_reset(1);
    assert_eq!(client.data.slots[0].title, "重置仍须保留名称");
    client.select_slot(2);
    client.slot_title_draft = "另一格未保存的草稿".into();
    client.mark_slot_dirty();
    client.workspace_persistence_ready = false;
    assert!(!client.confirm_timer_reset());
    assert!(client.desktop_ui.pending_reset.is_some());
    assert_eq!(client.data.slots[0].accumulated_millis, 120_000);
    client.workspace_persistence_ready = true;
    assert!(client.confirm_timer_reset(), "{}", client.status);
    assert!(client.desktop_ui.pending_reset.is_none());
    assert!(!client.confirm_timer_reset());
    let persisted = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    assert_eq!(persisted.slots[0].accumulated_millis, 0);
    assert_eq!(persisted.slots[0].title, "重置仍须保留名称");
    assert_eq!(persisted.slots[0].note, "重置仍须保留备注");
    assert_eq!(persisted.slots[1].title, "另一格未保存的草稿");
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_board_search_matches_independent_keywords_category_and_slot_number() {
    let state = app_data::default_app_data_json(now_millis());
    let projection = app_data::project_timer_views(&state, now_millis()).unwrap();
    let mut view = projection.slots[0].clone();
    view.title = "Rust 学习".into();
    view.note = "第七章 练习".into();
    assert!(timer_board_matches_query(&view, "编程", "练习 RUST"));
    assert!(timer_board_matches_query(&view, "编程", "编程 格子1"));
    assert!(timer_board_matches_query(&view, "编程", "01 学习"));
    assert!(timer_board_matches_query(&view, "编程", " \t\n "));
    assert!(!timer_board_matches_query(&view, "编程", "学习 音乐"));
}

#[test]
fn workspace_timer_shortcuts_ignore_hidden_selection_and_work_for_visible_selection() {
    let dir = temp_test_dir("workspace_filtered_timer_keyboard");
    let now = now_millis();
    let state = app_data::update_slot_title_app_data_json(
        &app_data::default_app_data_json(now),
        2,
        "可见项目",
        now,
    )
    .unwrap();
    let state =
        app_data::add_category_and_assign_app_data_json(&state, 2, "visible-category", "学习", now)
            .unwrap();
    let state = app_data::start_slot_app_data_json(&state, 2, now).unwrap();
    let mut client = test_client_for_account_scope(&dir, "account-a", state);
    client.settings.timer_bell_enabled = false;
    client.select_slot(1);
    let category = client
        .data
        .slots
        .iter()
        .find(|slot| slot.id == 2)
        .unwrap()
        .category_id
        .clone()
        .unwrap();
    let ctx = workspace_test_context();
    let size = egui::vec2(900.0, 600.0);
    let key_event = |key, pressed| egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    for filter in 0..3 {
        client.desktop_ui.board_query = if filter == 0 { "可见项目" } else { "" }.into();
        client.desktop_ui.board_category = if filter == 1 {
            category.clone()
        } else {
            String::new()
        };
        client.desktop_ui.running_only = filter == 2;
        for _ in 0..2 {
            workspace_test_frame(&mut client, &ctx, size, vec![]);
        }
        let original = client.state_json.clone();
        for key in [egui::Key::Space, egui::Key::F2] {
            workspace_test_frame(&mut client, &ctx, size, vec![key_event(key, true)]);
            workspace_test_frame(&mut client, &ctx, size, vec![key_event(key, false)]);
            assert_eq!(client.state_json, original, "filter {filter}, key {key:?}");
            assert!(!client.desktop_ui.slot_editor_open);
            assert_eq!(
                client.selected_slot_id, 1,
                "filtering must not silently change selection"
            );
        }
    }
    client.desktop_ui.board_query.clear();
    client.desktop_ui.board_category.clear();
    client.desktop_ui.running_only = false;
    workspace_test_frame(&mut client, &ctx, size, vec![]);
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![key_event(egui::Key::Space, true)],
    );
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![key_event(egui::Key::Space, false)],
    );
    wait_for_timer_action(&mut client);
    assert!(client
        .data
        .slots
        .iter()
        .find(|slot| slot.id == 1)
        .unwrap()
        .running_since_epoch_millis
        .is_some());
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![key_event(egui::Key::F2, true)],
    );
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![key_event(egui::Key::F2, false)],
    );
    assert!(client.desktop_ui.slot_editor_open);
    assert_eq!(client.selected_slot_id, 1);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}
