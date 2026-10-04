// v2.22.41 - Exercise knowledge editing, navigation and destructive-operation boundaries.

fn knowledge_fixture_state() -> String {
    let now = now_millis();
    let mut raw = app_data::default_app_data_json(now);
    for (id, name) in [("folder-a", "项目资料"), ("folder-b", "会议记录")] {
        raw = app_data::create_note_folder_app_data_json(&raw, id, name, now).unwrap();
    }
    for (id, kind, folder, title, body) in [
        (
            "doc-a",
            DesktopNoteKind::Document,
            Some("folder-a"),
            "项目方案",
            "原始正文",
        ),
        (
            "doc-b",
            DesktopNoteKind::Document,
            Some("folder-b"),
            "会议纪要",
            "第二份资料",
        ),
        (
            "sticky-a",
            DesktopNoteKind::Sticky,
            None,
            "随手记",
            "便签正文",
        ),
    ] {
        let mut note = desktop_note_save_value(id, kind, title, body, None, now);
        note["folderId"] = json!(folder);
        raw = app_data::upsert_note_app_data_json(&raw, &note.to_string(), now).unwrap();
    }
    raw
}

fn knowledge_test_client(dir: &Path) -> TimerWindowsClient {
    let mut client =
        test_client_for_account_scope(dir, "knowledge-test", knowledge_fixture_state());
    client.switch_tab(AppTab::Knowledge);
    client.select_note_by_id("doc-a");
    client
}

#[test]
fn knowledge_literal_slash_and_whitespace_survive_edit_save_and_reload() {
    let dir = temp_test_dir("knowledge_literal_slash");
    let mut client = knowledge_test_client(&dir);
    for body in ["/", " / ", "\n/\n", "  第一行\n\n第二行  \n"] {
        let block = client.note_blocks_draft[0].clone();
        client.apply_note_block_text_edit(&block, body.to_string());
        assert_eq!(client.note_content_draft, body);
        assert!(client.save_note());
        client.clear_note_draft_without_flush();
        client.select_note_by_id("doc-a");
        assert_eq!(client.note_content_draft, body);
        let disk = decode_data(&fs::read_to_string(&client.state_path).unwrap());
        assert_eq!(
            disk.notes
                .iter()
                .find(|n| n.id == "doc-a")
                .unwrap()
                .document
                .blocks[0]
                .text,
            body
        );
    }
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn knowledge_reselect_keeps_undo_redo_and_active_block() {
    let dir = temp_test_dir("knowledge_reselect_history");
    let mut client = knowledge_test_client(&dir);
    client.insert_note_text_block("第二段");
    client.insert_note_text_block("第三段");
    client.undo_note_draft();
    let undo = client.note_undo_stack.clone();
    let redo = client.note_redo_stack.clone();
    let active = client.note_active_block_id.clone();
    client.select_note_by_id("doc-a");
    assert!(!client.note_dirty);
    assert_eq!(client.note_undo_stack, undo);
    assert_eq!(client.note_redo_stack, redo);
    assert_eq!(client.note_active_block_id, active);
    client.redo_note_draft();
    assert!(client.note_content_draft.contains("第三段"));
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn knowledge_new_document_inherits_folder_and_clears_hiding_filters() {
    let dir = temp_test_dir("knowledge_new_folder");
    let mut client = knowledge_test_client(&dir);
    client.set_knowledge_folder_filter(Some("folder-a".into()));
    client.note_search_draft = "不匹配的新文档".into();
    client.knowledge_quick_filter = KnowledgeQuickFilter::Image;
    assert!(client.new_note_with_kind(DesktopNoteKind::Document));
    assert_eq!(client.note_folder_draft.as_deref(), Some("folder-a"));
    assert!(client.note_search_draft.is_empty());
    assert_eq!(client.knowledge_quick_filter, KnowledgeQuickFilter::All);
    client.set_note_canvas_text("刚创建的内容");
    assert!(client.save_note());
    client.rebuild_view_cache();
    assert!(client
        .view_cache
        .notes
        .iter()
        .any(|n| n.id == client.selected_note_id));
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn knowledge_trash_shows_all_deleted_documents_and_returns_to_browse_state() {
    let dir = temp_test_dir("knowledge_trash_scope");
    let mut client = knowledge_test_client(&dir);
    for id in ["doc-b", "sticky-a"] {
        let next = app_data::delete_note_app_data_json(&client.state_json, id, now_millis());
        assert!(client.replace_state(next, "fixture"));
    }
    client.set_knowledge_folder_filter(Some("folder-a".into()));
    client.note_search_draft = "项目方案".into();
    client.knowledge_quick_filter = KnowledgeQuickFilter::Pinned;
    assert!(client.toggle_knowledge_trash_mode());
    assert_eq!(
        client
            .view_cache
            .notes
            .iter()
            .map(|n| n.id.as_str())
            .collect::<Vec<_>>(),
        vec!["doc-b"]
    );
    assert!(client.note_search_draft.is_empty());
    assert!(client.toggle_knowledge_trash_mode());
    assert_eq!(client.note_search_draft, "项目方案");
    assert_eq!(client.selected_note_id, "doc-a");
    assert_eq!(client.knowledge_quick_filter, KnowledgeQuickFilter::Pinned);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn knowledge_restore_reveals_document_despite_old_folder_and_quick_filter() {
    let dir = temp_test_dir("knowledge_restore_visible");
    let mut client = knowledge_test_client(&dir);
    let next = app_data::delete_note_app_data_json(&client.state_json, "doc-b", now_millis());
    assert!(client.replace_state(next, "fixture"));
    client.set_knowledge_folder_filter(Some("folder-a".into()));
    client.knowledge_quick_filter = KnowledgeQuickFilter::Image;
    assert!(client.toggle_knowledge_trash_mode());
    client.restore_knowledge_note("doc-b");
    assert!(!client.knowledge_trash_mode);
    assert_eq!(client.selected_note_id, "doc-b");
    assert!(client.view_cache.notes.iter().any(|n| n.id == "doc-b"));
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn knowledge_permanent_delete_rejects_active_documents_and_sticky_notes() {
    let dir = temp_test_dir("knowledge_delete_boundary");
    let mut client = knowledge_test_client(&dir);
    let next = app_data::delete_note_app_data_json(&client.state_json, "sticky-a", now_millis());
    assert!(client.replace_state(next, "fixture"));
    for id in ["doc-a", "sticky-a", "missing"] {
        let before = client.state_json.clone();
        client.permanently_delete_knowledge_note(id);
        assert_eq!(
            before, client.state_json,
            "rejected deletion must leave the entire workspace intact"
        );
    }
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn knowledge_folder_delete_saves_draft_and_refreshes_its_membership() {
    let dir = temp_test_dir("knowledge_delete_folder_draft");
    let mut client = knowledge_test_client(&dir);
    client.knowledge_folder_edit_id = Some("folder-a".into());
    client.set_note_canvas_text("删除文件夹前刚输入的内容");
    client.delete_knowledge_folder();
    assert!(client.note_folder_draft.is_none());
    assert!(!client.note_dirty);
    client.clear_note_draft_without_flush();
    client.select_note_by_id("doc-a");
    assert_eq!(client.note_content_draft, "删除文件夹前刚输入的内容");
    assert!(client.selected_note().unwrap().folder_id.is_none());
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn knowledge_failed_new_document_retains_draft_and_filters() {
    let dir = temp_test_dir("knowledge_new_failure");
    let mut client = knowledge_test_client(&dir);
    client.note_search_draft = "保留查询".into();
    client.knowledge_quick_filter = KnowledgeQuickFilter::Pinned;
    client.set_note_canvas_text("保存失败也不能丢");
    client.workspace_persistence_ready = false;
    let before = client.current_note_draft_snapshot();
    assert!(!client.new_note_with_kind(DesktopNoteKind::Document));
    assert_eq!(before, client.current_note_draft_snapshot());
    assert_eq!(client.selected_note_id, "doc-a");
    assert_eq!(client.note_search_draft, "保留查询");
    assert_eq!(client.knowledge_quick_filter, KnowledgeQuickFilter::Pinned);
    client.workspace_persistence_ready = true;
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "Creates a new explicitly requested synthetic profile for native UI acceptance"]
fn knowledge_native_fixture() {
    let root = PathBuf::from(
        std::env::var_os("KNOWLEDGE_NATIVE_FIXTURE_ROOT").expect("explicit new fixture directory"),
    );
    fs::create_dir(&root).expect("fixture directory must not already exist");
    let legacy = root.join(LEGACY_APP_NAMESPACE_DIRECTORY);
    let current = root.join(APP_NAMESPACE_DIRECTORY);
    ensure_isolated_app_namespace(&legacy, &current).unwrap();
    let raw = knowledge_fixture_state();
    atomic_save_text(&current.join("timer_state.json"), &raw, |s| {
        app_data::sanitize_app_data_json(s, now_millis())
    })
    .unwrap();
    fs::write(
        root.join("fixture_identity.txt"),
        "SYNTHETIC_KNOWLEDGE_ACCEPTANCE_V2_22_41",
    )
    .unwrap();
}

#[test]
fn knowledge_new_document_keyboard_focus_survives_its_first_save() {
    for size in [egui::vec2(760.0, 480.0), egui::vec2(1240.0, 800.0)] {
        let dir = temp_test_dir("knowledge_keyboard_focus");
        let mut client = knowledge_test_client(&dir);
        let ctx = workspace_test_context();
        assert!(client.new_note_with_kind(DesktopNoteKind::Document));
        for _ in 0..3 {
            workspace_test_frame(&mut client, &ctx, size, vec![]);
        }
        workspace_test_frame(
            &mut client,
            &ctx,
            size,
            vec![egui::Event::Text("键盘文档".into())],
        );
        assert_eq!(
            client.note_title_draft, "键盘文档",
            "new documents must receive keyboard input immediately"
        );
        let body_id = egui::Id::new((
            "knowledge_block_editor",
            client.desktop_ui.navigation.editor_epoch,
            &client.note_blocks_draft[0].id,
        ));
        ctx.memory_mut(|m| m.request_focus(body_id));
        workspace_test_frame(&mut client, &ctx, size, vec![egui::Event::Text("/".into())]);
        assert_eq!(client.note_content_draft, "/");
        assert!(client.save_note());
        workspace_test_frame(
            &mut client,
            &ctx,
            size,
            vec![egui::Event::Text("下一段".into())],
        );
        assert_eq!(
            client.note_content_draft, "/下一段",
            "first save must not detach keyboard focus"
        );
        drop(client);
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn knowledge_keyboard_punctuation_key_events_insert_text() {
    let dir = temp_test_dir("knowledge_keyboard_punctuation");
    let mut client = knowledge_test_client(&dir);
    let ctx = workspace_test_context();
    let size = egui::vec2(1240.0, 800.0);
    for _ in 0..3 {
        workspace_test_frame(&mut client, &ctx, size, vec![]);
    }
    let body_id = egui::Id::new((
        "knowledge_block_editor",
        client.desktop_ui.navigation.editor_epoch,
        &client.note_blocks_draft[0].id,
    ));
    ctx.memory_mut(|memory| memory.request_focus(body_id));
    let punctuation = [
        egui::Key::Comma,
        egui::Key::Period,
        egui::Key::Semicolon,
        egui::Key::Colon,
        egui::Key::Slash,
        egui::Key::OpenBracket,
        egui::Key::CloseBracket,
    ]
    .into_iter()
    .map(|key| egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    })
    .collect();
    workspace_test_frame(&mut client, &ctx, size, punctuation);
    assert_eq!(client.note_content_draft, "原始正文,.;:/[]");
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn knowledge_keyboard_standalone_ime_punctuation_at_sentence_end_is_preserved() {
    let dir = temp_test_dir("knowledge_keyboard_ime_punctuation");
    let mut client = knowledge_test_client(&dir);
    let ctx = workspace_test_context();
    let size = egui::vec2(1240.0, 800.0);
    for _ in 0..3 {
        workspace_test_frame(&mut client, &ctx, size, vec![]);
    }
    let body_id = egui::Id::new((
        "knowledge_block_editor",
        client.desktop_ui.navigation.editor_epoch,
        &client.note_blocks_draft[0].id,
    ));
    ctx.memory_mut(|memory| memory.request_focus(body_id));
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![egui::Event::CompositionEnd("。".to_string())],
    );
    assert_eq!(client.note_content_draft, "原始正文。");
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn knowledge_keyboard_ime_commit_does_not_duplicate_punctuation() {
    let dir = temp_test_dir("knowledge_keyboard_ime_commit");
    let mut client = knowledge_test_client(&dir);
    let ctx = workspace_test_context();
    let size = egui::vec2(1240.0, 800.0);
    for _ in 0..3 {
        workspace_test_frame(&mut client, &ctx, size, vec![]);
    }
    let body_id = egui::Id::new((
        "knowledge_block_editor",
        client.desktop_ui.navigation.editor_epoch,
        &client.note_blocks_draft[0].id,
    ));
    ctx.memory_mut(|memory| memory.request_focus(body_id));
    workspace_test_frame(&mut client, &ctx, size, vec![egui::Event::CompositionStart]);
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![egui::Event::CompositionUpdate("。".to_string())],
    );
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![egui::Event::CompositionEnd("。".to_string())],
    );
    assert_eq!(client.note_content_draft, "原始正文。");
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn knowledge_save_buttons_disable_empty_and_clean_submissions() {
    let dir = temp_test_dir("knowledge_submit_states");
    let mut client = knowledge_test_client(&dir);
    let ctx = workspace_test_context();
    let size = egui::vec2(1240.0, 800.0);
    for _ in 0..3 {
        workspace_test_frame(&mut client, &ctx, size, vec![]);
    }
    let (rect, enabled) = ctx
        .data(|d| d.get_temp::<(egui::Rect, bool)>(egui::Id::new("knowledge_save")))
        .unwrap();
    assert!(!enabled);
    let before = client.state_json.clone();
    workspace_click(&mut client, &ctx, size, rect.center());
    assert!(
        before == client.state_json,
        "clean save must not create a new revision"
    );
    client.set_note_canvas_text("需要保存");
    workspace_test_frame(&mut client, &ctx, size, vec![]);
    assert!(
        ctx.data(|d| d.get_temp::<(egui::Rect, bool)>(egui::Id::new("knowledge_save")))
            .unwrap()
            .1
    );
    client.knowledge_folder_manager_open = true;
    client.knowledge_folder_name_draft = "   ".into();
    workspace_test_frame(&mut client, &ctx, size, vec![]);
    assert!(
        !ctx.data(|d| d.get_temp::<(egui::Rect, bool)>(egui::Id::new("knowledge_folder_save")))
            .unwrap()
            .1
    );
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn knowledge_encryption_lock_wrong_password_and_reopen_preserve_content() {
    let dir = temp_test_dir("knowledge_encryption_lifecycle");
    let mut client = knowledge_test_client(&dir);
    client.set_note_canvas_text("仅在解锁后可见的资料");
    client.note_crypto_password_draft = "synthetic-knowledge-password".into();
    client.enable_note_encryption();
    assert!(client
        .data
        .notes
        .iter()
        .find(|n| n.id == "doc-a")
        .unwrap()
        .encryption
        .is_some());
    client.lock_note_encryption();
    assert!(client.selected_note_is_locked());
    client.note_search_draft = "仅在解锁后可见".into();
    client.rebuild_view_cache();
    assert!(client.view_cache.notes.is_empty());
    client.note_crypto_password_draft = "wrong-password".into();
    client.unlock_note_encryption();
    assert!(client.selected_note_is_locked());
    client.note_crypto_password_draft = "synthetic-knowledge-password".into();
    client.unlock_note_encryption();
    assert!(!client.selected_note_is_locked());
    assert_eq!(client.note_content_draft, "仅在解锁后可见的资料");
    client.set_note_canvas_text("解锁后的修改");
    client.switch_tab(AppTab::Board);
    client.switch_tab(AppTab::Knowledge);
    assert!(client.selected_note_is_locked());
    client.note_crypto_password_draft = "synthetic-knowledge-password".into();
    client.unlock_note_encryption();
    assert_eq!(client.note_content_draft, "解锁后的修改");
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn knowledge_question_save_failure_never_starts_a_model_request() {
    let dir = temp_test_dir("knowledge_question_save_barrier");
    let mut client = knowledge_test_client(&dir);
    client.sync.ai_api_key = "synthetic-never-sent".into();
    client.knowledge_ai_question_draft = "总结刚输入的内容".into();
    client.set_note_canvas_text("不能绕过保存失败");
    // Break only this synthetic profile's journal path, while keeping the UI enabled.
    let journal = dir.join(DESKTOP_STATE_STORE_FILE);
    if journal.is_file() {
        fs::remove_file(&journal).unwrap();
    }
    fs::create_dir(&journal).unwrap();
    client.launch_knowledge_ai();
    assert!(!client.knowledge_ai_pending);
    assert!(client.knowledge_ai_result_rx.is_none());
    assert!(client.note_dirty);
    assert_eq!(client.note_content_draft, "不能绕过保存失败");
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}
