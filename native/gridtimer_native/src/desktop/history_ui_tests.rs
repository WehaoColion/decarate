// v2.22.39 - Verify archive notes and restore targets alongside durable history actions.

fn history_test_state(now: i64) -> String {
    let mut value: Value = serde_json::from_str(&app_data::default_app_data_json(now)).unwrap();
    value["slots"][0]["title"] = Value::from("当前项目");
    value["slots"][0]["accumulatedMillis"] = Value::from(180_000);
    value["sessions"] = serde_json::json!([
        {"id":"history-a", "slotId":1, "slotTitle":"学习记录", "startedAtEpochMillis":now-90_000, "endedAtEpochMillis":now-30_000, "durationMillis":60_000},
        {"id":"history-b", "slotId":2, "slotTitle":"写作记录", "startedAtEpochMillis":now-180_000, "endedAtEpochMillis":now-90_000, "durationMillis":90_000}
    ]);
    value["archivedTasks"] = serde_json::json!([
        {"id":"archive-a", "originalSlotId":1, "title":"旧学习项目", "note":"归档备注", "accumulatedMillis":120_000, "archivedAtEpochMillis":now-20_000, "updatedAtEpochMillis":now-20_000},
        {"id":"archive-b", "originalSlotId":2, "title":"旧写作项目", "accumulatedMillis":240_000, "archivedAtEpochMillis":now-10_000, "updatedAtEpochMillis":now-10_000}
    ]);
    app_data::sanitize_app_data_json(&value.to_string(), now).unwrap()
}

#[test]
fn history_delete_requires_confirmation_and_only_removes_the_selected_record() {
    let dir = temp_test_dir("history_confirm_delete");
    let mut client =
        test_client_for_account_scope(&dir, "account-a", history_test_state(now_millis()));
    let original: Value = serde_json::from_str(&client.state_json).unwrap();
    client.request_history_delete(HistoryRecordKind::Session, "history-a");
    assert_eq!(
        client.data.sessions.len(),
        2,
        "requesting deletion must not mutate data"
    );
    assert!(client.desktop_ui.pending_history_delete.is_some());
    assert!(client.confirm_history_delete());
    let persisted: Value =
        serde_json::from_str(&fs::read_to_string(&client.state_path).unwrap()).unwrap();
    assert_eq!(persisted["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(persisted["sessions"][0]["id"], "history-b");
    assert_eq!(persisted["slots"], original["slots"]);
    assert_eq!(persisted["archivedTasks"], original["archivedTasks"]);
    assert!(persisted["tombstones"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| { item["entityId"] == "history-a" && item["entityType"] == "session" }));
    assert!(
        !client.confirm_history_delete(),
        "a consumed confirmation must not be replayed"
    );
    client.request_history_delete(HistoryRecordKind::Archive, "archive-a");
    assert!(client.confirm_history_delete());
    let persisted: Value =
        serde_json::from_str(&fs::read_to_string(&client.state_path).unwrap()).unwrap();
    assert_eq!(persisted["archivedTasks"].as_array().unwrap().len(), 1);
    assert_eq!(persisted["archivedTasks"][0]["id"], "archive-b");
    assert_eq!(persisted["sessions"][0]["id"], "history-b");
    assert_eq!(persisted["slots"], original["slots"]);
    assert!(persisted["tombstones"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| { item["entityId"] == "archive-a" && item["entityType"] == "archivedTask" }));
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn history_delete_blocks_changed_records_and_account_switches() {
    let dir = temp_test_dir("history_stale_confirmation");
    let mut client =
        test_client_for_account_scope(&dir, "account-a", history_test_state(now_millis()));
    client.request_history_delete(HistoryRecordKind::Archive, "archive-a");
    let mut newer: Value = serde_json::from_str(&client.state_json).unwrap();
    let archive = newer["archivedTasks"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|record| record["id"] == "archive-a")
        .unwrap();
    archive["note"] = Value::from("另一台设备刚补充的备注");
    assert!(client.replace_state(Some(newer.to_string()), "测试并发修改"));
    assert!(!client.confirm_history_delete());
    assert_eq!(client.data.archived_tasks.len(), 2);
    assert!(client.desktop_ui.pending_history_delete.is_none());

    client.request_history_delete(HistoryRecordKind::Session, "history-a");
    let initial_user = client.sync.user_id.clone();
    client.sync.user_id = "account-b".to_string();
    assert!(!client.confirm_history_delete());
    client.sync.user_id = initial_user;
    assert_eq!(client.data.sessions.len(), 2);
    assert!(client.desktop_ui.pending_history_delete.is_none());
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn history_delete_preserves_dirty_drafts_and_retries_after_save_failure() {
    let dir = temp_test_dir("history_save_barrier");
    let mut client =
        test_client_for_account_scope(&dir, "account-a", history_test_state(now_millis()));
    client.request_history_delete(HistoryRecordKind::Session, "history-a");
    client.ensure_selected_slot_draft();
    client.slot_title_draft = "删除前未保存的名称".to_string();
    client.mark_slot_dirty();
    client.workspace_persistence_ready = false;
    assert!(!client.confirm_history_delete());
    assert_eq!(client.data.sessions.len(), 2);
    assert_eq!(client.slot_title_draft, "删除前未保存的名称");
    assert!(client.desktop_ui.pending_history_delete.is_some());
    client.workspace_persistence_ready = true;
    assert!(client.confirm_history_delete());
    assert_eq!(client.data.slots[0].title, "删除前未保存的名称");
    assert_eq!(client.data.sessions.len(), 1);
    let persisted = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    assert_eq!(persisted.slots[0].title, "删除前未保存的名称");
    assert_eq!(persisted.sessions.len(), 1);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn history_slot_filter_updates_rows_and_totals_without_reparsing_state() {
    let dir = temp_test_dir("history_slot_filter");
    let now = now_millis();
    let mut client = test_client_for_account_scope(&dir, "account-a", history_test_state(now));
    client.refresh_history_index(now);
    assert_eq!(client.desktop_ui.history_summary.total_millis, 150_000);
    let original_index = client.desktop_ui.history_index.clone().unwrap();
    client.desktop_ui.history_slot = Some(1);
    client.refresh_history_index(now);
    assert_eq!(client.desktop_ui.history_summary.total_millis, 60_000);
    assert_eq!(
        client.desktop_ui.history_summary.session_ids,
        vec!["history-a"]
    );
    assert_eq!(
        client.desktop_ui.history_summary.archived_task_ids,
        vec!["archive-a"]
    );
    assert_eq!(client.desktop_ui.history_session_rows.len(), 1);
    assert_eq!(client.desktop_ui.history_archive_rows.len(), 1);
    assert!(Arc::ptr_eq(
        &original_index,
        client.desktop_ui.history_index.as_ref().unwrap()
    ));
    client.desktop_ui.history_slot = Some(2);
    client.refresh_history_index(now);
    assert_eq!(client.desktop_ui.history_summary.total_millis, 90_000);
    assert_eq!(
        client.desktop_ui.history_summary.session_ids,
        vec!["history-b"]
    );
    client.desktop_ui.history_slot = None;
    client.refresh_history_index(now);
    assert_eq!(client.desktop_ui.history_summary.total_millis, 150_000);
    client.desktop_ui.history_slot = Some(99);
    client.refresh_history_index(now);
    assert_eq!(client.desktop_ui.history_summary.total_millis, 0);
    assert!(client.desktop_ui.history_session_rows.is_empty());
    assert!(client.desktop_ui.history_archive_rows.is_empty());

    // Reordering the board must not turn stable slot ids into display indices.
    assert!(client.replace_state(
        app_data::set_slot_order_app_data_json(&client.state_json, &[2, 1], now),
        "测试格子排序"
    ));
    client.desktop_ui.history_slot = Some(1);
    client.refresh_history_index(now);
    assert_eq!(
        client.desktop_ui.history_summary.session_ids,
        vec!["history-a"]
    );
    assert_eq!(
        client.desktop_ui.history_summary.archived_task_ids,
        vec!["archive-a"]
    );
    assert_eq!(client.desktop_ui.history_summary.total_millis, 60_000);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn history_open_slot_clears_board_filters_and_never_starts_a_timer() {
    let dir = temp_test_dir("history_open_slot");
    let mut client =
        test_client_for_account_scope(&dir, "account-a", history_test_state(now_millis()));
    client.switch_tab(AppTab::History);
    client.desktop_ui.board_query = "不会匹配的筛选".to_string();
    client.desktop_ui.board_category = "study".to_string();
    client.desktop_ui.running_only = true;
    client.open_history_slot(2);
    assert!(client.tab == AppTab::Board);
    assert_eq!(client.selected_slot_id, 2);
    assert!(client.desktop_ui.slot_editor_open);
    assert!(client.desktop_ui.board_query.is_empty());
    assert!(client.desktop_ui.board_category.is_empty());
    assert!(!client.desktop_ui.running_only);
    assert!(client
        .data
        .slots
        .iter()
        .all(|slot| slot.running_since_epoch_millis.is_none()));
    client.switch_tab(AppTab::History);
    client.open_history_slot(99);
    assert!(client.tab == AppTab::History);
    assert_eq!(client.data.sessions.len(), 2);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn history_delete_click_cancel_and_confirmation_use_separate_targets() {
    let dir = temp_test_dir("history_delete_clicks");
    let mut client =
        test_client_for_account_scope(&dir, "account-a", history_test_state(now_millis()));
    client.switch_tab(AppTab::History);
    let ctx = workspace_test_context();
    let size = egui::vec2(1000.0, 800.0);
    for _ in 0..3 {
        workspace_test_frame(&mut client, &ctx, size, vec![]);
    }
    let delete = ctx
        .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("history_delete", "history-a"))))
        .unwrap();
    workspace_click(&mut client, &ctx, size, delete.center());
    assert!(client.desktop_ui.pending_history_delete.is_some());
    assert_eq!(client.data.sessions.len(), 2);
    for _ in 0..3 {
        workspace_test_frame(&mut client, &ctx, size, vec![]);
    }
    let cancel = ctx
        .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("history_delete_cancel")))
        .unwrap();
    workspace_click(&mut client, &ctx, size, cancel.center());
    assert!(client.desktop_ui.pending_history_delete.is_none());
    assert_eq!(client.data.sessions.len(), 2);
    for _ in 0..3 {
        workspace_test_frame(&mut client, &ctx, size, vec![]);
    }
    workspace_click(&mut client, &ctx, size, delete.center());
    for _ in 0..3 {
        workspace_test_frame(&mut client, &ctx, size, vec![]);
    }
    let confirm = ctx
        .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("history_delete_confirm")))
        .unwrap();
    workspace_click(&mut client, &ctx, size, confirm.center());
    assert!(client.desktop_ui.pending_history_delete.is_none());
    assert_eq!(client.data.sessions.len(), 1);
    assert_eq!(client.data.sessions[0].id, "history-b");
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn history_archive_cards_show_notes_and_actual_restore_destination() {
    let dir = temp_test_dir("history_archive_details");
    let now = now_millis();
    let mut client = test_client_for_account_scope(&dir, "account-a", history_test_state(now));
    client.switch_tab(AppTab::History);
    client.desktop_ui.history_archives = true;
    client.desktop_ui.history_query = "归档备注".into();
    let ctx = workspace_test_context();
    let size = egui::vec2(900.0, 600.0);
    for _ in 0..3 {
        workspace_test_frame(&mut client, &ctx, size, vec![]);
    }
    let output = workspace_test_frame(&mut client, &ctx, size, vec![]);
    let labels = output
        .shapes
        .iter()
        .filter_map(|shape| {
            if let egui::epaint::Shape::Text(text) = &shape.shape {
                Some(text.galley.text().to_string())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert!(labels.iter().any(|label| label == "归档备注"));
    assert!(
        labels.iter().any(|label| label == "恢复到格子 02"),
        "{labels:?}"
    );
    let restore = ctx
        .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("history_restore", "archive-a"))))
        .unwrap();
    workspace_click(&mut client, &ctx, size, restore.center());
    assert!(client.tab == AppTab::Board);
    assert_eq!(client.selected_slot_id, 2);
    assert_eq!(client.data.slots[0].title, "当前项目");
    assert_eq!(client.data.slots[1].title, "旧学习项目");
    assert_eq!(client.data.slots[1].note, "归档备注");
    assert_eq!(client.data.slots[1].accumulated_millis, 120_000);
    assert!(client.data.slots[1].running_since_epoch_millis.is_none());
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn history_archive_restore_is_disabled_when_every_slot_is_occupied() {
    let dir = temp_test_dir("history_archive_full_board");
    let now = now_millis();
    let mut state: Value = serde_json::from_str(&history_test_state(now)).unwrap();
    for slot in state["slots"].as_array_mut().unwrap() {
        slot["title"] = Value::from("已有项目");
    }
    let mut client = test_client_for_account_scope(&dir, "account-a", state.to_string());
    client.switch_tab(AppTab::History);
    client.desktop_ui.history_archives = true;
    client.desktop_ui.history_query = "归档备注".into();
    let ctx = workspace_test_context();
    let size = egui::vec2(900.0, 600.0);
    for _ in 0..3 {
        workspace_test_frame(&mut client, &ctx, size, vec![]);
    }
    let restore = ctx
        .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("history_restore", "archive-a"))))
        .unwrap();
    let before = client.state_json.clone();
    workspace_click(&mut client, &ctx, size, restore.center());
    assert_eq!(client.state_json, before);
    assert!(client.tab == AppTab::History);
    assert_eq!(client.data.archived_tasks.len(), 2);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn history_board_archive_shortcut_clears_filters_and_shows_all_archives() {
    let dir = temp_test_dir("history_archive_shortcut");
    let now = now_millis();
    let mut client = test_client_for_account_scope(&dir, "account-a", history_test_state(now));
    client.desktop_ui.history_query = "不匹配的旧筛选".into();
    client.desktop_ui.history_category = "study".into();
    client.desktop_ui.history_slot = Some(99);
    client.desktop_ui.history_period = 1;
    client.open_timer_archives();
    client.refresh_history_index(now);
    assert!(client.tab == AppTab::History);
    assert!(client.desktop_ui.history_archives);
    assert_eq!(client.desktop_ui.history_archive_rows.len(), 2);
    assert!(client.desktop_ui.history_query.is_empty());
    assert!(client.desktop_ui.history_category.is_empty());
    assert!(client.desktop_ui.history_slot.is_none());
    assert_eq!(client.desktop_ui.history_period, 0);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}
