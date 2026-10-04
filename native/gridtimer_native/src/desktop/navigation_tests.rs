// v2.22.48 - Verify timer record save barriers, account boundaries and return navigation.
// v2.22.39 - Check restored-document visibility, continuity, small windows and modal isolation.

fn navigation_key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

#[test]
fn navigation_timer_history_preserves_drafts_and_returns_without_stopping_timer() {
    let dir = temp_test_dir("navigation_timer_history_return");
    let mut client =
        test_client_for_account_scope(&dir, "account-a", history_test_state(now_millis()));
    client.settings.timer_bell_enabled = false;
    let ctx = workspace_test_context();
    client.toggle_slot(&client.selected_slot().unwrap(), &ctx);
    wait_for_timer_action(&mut client);
    let running_since = client.selected_slot().unwrap().running_since_epoch_millis;
    client.desktop_ui.board_query = "当前".into();
    client.open_slot_editor(1);
    client.slot_title_draft = "当前项目已修改".into();
    client.mark_slot_dirty();
    client.workspace_persistence_ready = false;
    assert!(!client.open_timer_history(Some(1), false));
    assert!(client.tab == AppTab::Board);
    assert!(client.desktop_ui.slot_editor_open);
    assert_eq!(client.slot_title_draft, "当前项目已修改");

    client.workspace_persistence_ready = true;
    assert!(client.open_timer_history(Some(1), false));
    assert!(client.tab == AppTab::History);
    assert!(client.tab.primary_destination() == AppTab::Board);
    assert!(!client.desktop_ui.slot_editor_open);
    assert_eq!(client.desktop_ui.history_slot, Some(1));
    assert_eq!(client.data.slots[0].title, "当前项目已修改");

    let size = egui::vec2(760.0, 480.0);
    client.request_history_delete(HistoryRecordKind::Session, "history-a");
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![navigation_key(egui::Key::Escape, egui::Modifiers::NONE)],
    );
    assert!(client.desktop_ui.pending_history_delete.is_none());
    assert!(
        client.tab == AppTab::History,
        "Escape first dismisses the deletion confirmation"
    );
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![navigation_key(egui::Key::Escape, egui::Modifiers::NONE)],
    );
    assert!(client.tab == AppTab::Board);
    assert!(client.desktop_ui.slot_editor_open);
    assert_eq!(client.desktop_ui.board_query, "当前");
    assert_eq!(client.selected_slot_id, 1);
    assert_eq!(
        client.selected_slot().unwrap().running_since_epoch_millis,
        running_since
    );
    assert!(client
        .data
        .sessions
        .iter()
        .any(|session| session.id == "history-a"));
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn navigation_timer_history_return_does_not_reopen_another_accounts_detail() {
    let dir = temp_test_dir("navigation_history_account_boundary");
    let mut client =
        test_client_for_account_scope(&dir, "account-a", history_test_state(now_millis()));
    client.open_slot_editor(1);
    assert!(client.open_timer_history(Some(1), false));
    client.sync.user_id = "account-b".into();
    client.return_from_timer_history();
    assert!(client.tab == AppTab::Board);
    assert!(!client.desktop_ui.slot_editor_open);
    assert!(client.desktop_ui.navigation.history_return.is_none());
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn navigation_primary_shortcuts_open_risk_overview_and_clear_history_return() {
    let dir = temp_test_dir("navigation_primary_shortcuts");
    let mut client =
        test_client_for_account_scope(&dir, "account-a", history_test_state(now_millis()));
    let before = client.state_json.clone();
    client.open_slot_editor(1);
    assert!(client.open_timer_history(Some(1), false));
    let ctx = workspace_test_context();
    let size = egui::vec2(760.0, 480.0);
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![navigation_key(egui::Key::Num4, egui::Modifiers::CTRL)],
    );
    assert!(client.tab == AppTab::Finance);
    assert_eq!(client.finance_workbench.tab, 5);
    assert!(client.desktop_ui.navigation.history_return.is_none());
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![navigation_key(egui::Key::Num5, egui::Modifiers::CTRL)],
    );
    assert!(client.tab == AppTab::My);
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![navigation_key(egui::Key::Num1, egui::Modifiers::CTRL)],
    );
    assert!(client.tab == AppTab::Board);
    assert!(!client.desktop_ui.slot_editor_open);
    assert_eq!(
        client.state_json, before,
        "navigation must not modify timer or account data"
    );
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn navigation_switching_modules_restores_saved_document_and_latest_edits() {
    let dir = temp_test_dir("navigation_restore_document");
    let state = app_state_with_note("nav-note", "原题", "正文", None);
    let mut client = test_client_for_account_scope(&dir, "account-a", state);
    client.switch_tab(AppTab::Notes);
    client.select_note_by_id("nav-note");
    client.set_note_canvas_text("最后一行也要保留");
    client.switch_tab(AppTab::Board);
    assert!(client.tab == AppTab::Board);
    client.switch_tab(AppTab::Notes);
    assert_eq!(client.selected_note_id, "nav-note");
    assert_eq!(client.note_content_draft, "最后一行也要保留");
    assert!(client.desktop_ui.navigation.editor_visible);
    let saved = fs::read_to_string(&client.state_path).unwrap();
    assert!(saved.contains("最后一行也要保留"));
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn navigation_selection_is_not_restored_across_accounts() {
    let dir = temp_test_dir("navigation_account_scope");
    let state = app_state_with_note("shared-id", "账户甲", "甲内容", None);
    let mut client = test_client_for_account_scope(&dir, "account-a", state);
    client.switch_tab(AppTab::Notes);
    client.select_note_by_id("shared-id");
    client.switch_tab(AppTab::Board);
    client.sync.user_id = "account-b".to_string();
    client.switch_tab(AppTab::Notes);
    assert!(client.selected_note_id.is_empty());
    assert!(!client.desktop_ui.navigation.editor_visible);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn navigation_compact_editor_starts_above_the_fold_and_returns_to_search() {
    let dir = temp_test_dir("navigation_compact_editor");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_state_with_note("nav-note", "短窗口", "可见正文", None),
    );
    client.switch_tab(AppTab::Notes);
    let ctx = workspace_test_context();
    let size = egui::vec2(760.0, 480.0);
    for _ in 0..3 {
        workspace_test_frame(&mut client, &ctx, size, vec![]);
    }
    client.select_note_by_id("nav-note");
    for _ in 0..3 {
        workspace_test_frame(&mut client, &ctx, size, vec![]);
    }
    let body = ctx
        .data(|data| data.get_temp::<egui::Rect>(egui::Id::new("sticky_body_rect")))
        .unwrap();
    assert!(
        body.top() < 395.0,
        "first lines must be visible without scrolling: {body:?}"
    );
    let back = ctx
        .data(|data| data.get_temp::<egui::Rect>(egui::Id::new("document_back")))
        .unwrap();
    workspace_click(&mut client, &ctx, size, back.center());
    assert!(!client.desktop_ui.navigation.editor_visible);
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![navigation_key(egui::Key::F, egui::Modifiers::CTRL)],
    );
    assert!(ctx.memory(|memory| memory.has_focus(egui::Id::new("notes_search"))));
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn navigation_new_note_and_search_switch_the_compact_surface() {
    let dir = temp_test_dir("navigation_new_search");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_data::default_app_data_json(now_millis()),
    );
    client.switch_tab(AppTab::Notes);
    let ctx = workspace_test_context();
    let size = egui::vec2(760.0, 480.0);
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![navigation_key(egui::Key::N, egui::Modifiers::CTRL)],
    );
    assert!(client.desktop_ui.navigation.editor_visible);
    workspace_test_frame(&mut client, &ctx, size, vec![]);
    assert!(ctx.memory(|memory| memory.has_focus(egui::Id::new("note_title_editor"))));
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![navigation_key(egui::Key::F, egui::Modifiers::CTRL)],
    );
    assert!(!client.desktop_ui.navigation.editor_visible);
    assert!(ctx.memory(|memory| memory.has_focus(egui::Id::new("notes_search"))));
    assert!(
        client.data.notes.is_empty(),
        "opening and leaving must not create a blank note"
    );
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn navigation_timer_editor_blocks_background_clicks_and_global_tab_shortcuts() {
    let dir = temp_test_dir("navigation_modal_focus");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_data::default_app_data_json(now_millis()),
    );
    let ctx = workspace_test_context();
    let size = egui::vec2(1240.0, 800.0);
    for _ in 0..3 {
        workspace_test_frame(&mut client, &ctx, size, vec![]);
    }
    let clock = ctx
        .data(|data| data.get_temp::<egui::Rect>(egui::Id::new(("timer_clock", 1))))
        .unwrap();
    client.open_slot_editor(2);
    for _ in 0..3 {
        workspace_test_frame(&mut client, &ctx, size, vec![]);
    }
    workspace_click(&mut client, &ctx, size, clock.center());
    assert_eq!(client.selected_slot_id, 2);
    assert!(client
        .data
        .slots
        .iter()
        .all(|slot| slot.running_since_epoch_millis.is_none()));
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![navigation_key(egui::Key::Num2, egui::Modifiers::CTRL)],
    );
    assert!(client.tab == AppTab::Board);
    assert!(client.desktop_ui.slot_editor_open);
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![navigation_key(egui::Key::Escape, egui::Modifiers::NONE)],
    );
    assert!(!client.desktop_ui.slot_editor_open);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn navigation_failed_save_keeps_the_editor_open() {
    let dir = temp_test_dir("navigation_failed_return");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_state_with_note("nav-note", "标题", "原文", None),
    );
    client.switch_tab(AppTab::Notes);
    client.select_note_by_id("nav-note");
    client.set_note_canvas_text("需要保留的编辑");
    client.workspace_persistence_ready = false;
    assert!(!client.return_to_document_list());
    assert!(client.desktop_ui.navigation.editor_visible);
    assert_eq!(client.note_content_draft, "需要保留的编辑");
    client.workspace_persistence_ready = true;
    assert!(client.return_to_document_list());
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn navigation_delete_returns_to_list_and_restore_reopens_the_document() {
    let dir = temp_test_dir("navigation_delete_restore");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_state_with_note("nav-note", "标题", "原文", None),
    );
    client.switch_tab(AppTab::Notes);
    client.select_note_by_id("nav-note");
    client.delete_note();
    assert!(!client.desktop_ui.navigation.editor_visible);
    assert!(client.selected_note_id.is_empty());
    client.note_trash_mode = true;
    client.note_search_draft = "原文".into();
    client.restore_sticky_note("nav-note");
    assert!(client.desktop_ui.navigation.editor_visible);
    assert!(!client.note_trash_mode);
    assert!(client.note_search_draft.is_empty());
    assert_eq!(client.selected_note_id, "nav-note");
    assert_eq!(client.note_content_draft, "原文");
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn navigation_search_is_visible_at_the_single_column_width_boundary() {
    for width in [900.0, 910.0] {
        let dir = temp_test_dir("navigation_width_boundary");
        let mut client = test_client_for_account_scope(
            &dir,
            "account-a",
            app_state_with_note("nav-note", "长文档", &"正文\n".repeat(100), None),
        );
        client.switch_tab(AppTab::Notes);
        client.select_note_by_id("nav-note");
        let ctx = workspace_test_context();
        let size = egui::vec2(width, 600.0);
        for _ in 0..3 {
            workspace_test_frame(&mut client, &ctx, size, vec![]);
        }
        workspace_test_frame(
            &mut client,
            &ctx,
            size,
            vec![
                egui::Event::PointerMoved(egui::pos2(width - 30.0, 400.0)),
                egui::Event::Scroll(egui::vec2(0.0, -600.0)),
            ],
        );
        workspace_test_frame(
            &mut client,
            &ctx,
            size,
            vec![navigation_key(egui::Key::F, egui::Modifiers::CTRL)],
        );
        let output = workspace_test_frame(&mut client, &ctx, size, vec![]);
        assert!(!client.desktop_ui.navigation.editor_visible);
        assert!(ctx.memory(|memory| memory.has_focus(egui::Id::new("notes_search"))));
        assert!(
            output.shapes.iter().any(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) =>
                    text.galley.text() == "搜索便签"
                        && shape.clip_rect.intersects(text.visual_bounding_rect())
                        && text.pos.y < 200.0,
                _ => false,
            }),
            "search must be visible after leaving a scrolled editor at width {width}"
        );
        drop(client);
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn navigation_knowledge_restore_leaves_trash_and_failed_save_keeps_it_unchanged() {
    let dir = temp_test_dir("navigation_knowledge_restore");
    let mut state: Value =
        serde_json::from_str(&app_state_with_note("doc", "恢复文档", "中文正文", None)).unwrap();
    state["notes"][0]["kind"] = Value::String("DOCUMENT".into());
    let mut client = test_client_for_account_scope(&dir, "account-a", state.to_string());
    client.switch_tab(AppTab::Knowledge);
    client.select_note_by_id("doc");
    client.delete_note();
    client.knowledge_trash_mode = true;
    client.note_search_draft = "恢复".into();
    client.workspace_persistence_ready = false;
    client.restore_knowledge_note("doc");
    assert!(client.knowledge_trash_mode);
    assert!(client.data.notes[0].deleted_at_epoch_millis.is_some());
    client.workspace_persistence_ready = true;
    client.restore_knowledge_note("doc");
    assert!(client.desktop_ui.navigation.editor_visible);
    assert!(!client.knowledge_trash_mode);
    assert!(client.note_search_draft.is_empty());
    assert_eq!("doc", client.selected_note_id);
    assert_eq!("中文正文", client.note_content_draft);
    assert!(client.data.notes[0].deleted_at_epoch_millis.is_none());
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}
