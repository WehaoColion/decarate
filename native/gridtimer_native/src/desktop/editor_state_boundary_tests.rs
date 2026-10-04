// Windows - Exercise real egui undo input and release document-owned edit histories.

fn editor_boundary_frame(
    client: &mut TimerWindowsClient,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) {
    let time = ctx.input(|input| input.time) + 0.01;
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1240.0, 900.0),
            )),
            time: Some(time),
            events,
            ..Default::default()
        },
        |ctx| {
            client.handle_workspace_shortcuts(ctx);
            egui::CentralPanel::default().show(ctx, |ui| {
                if client.tab == AppTab::Notes {
                    client.ui_sticky_note_editor(ui);
                } else {
                    client.ui_note_editor(ui);
                }
            });
        },
    );
}

fn editor_boundary_focus(client: &mut TimerWindowsClient, ctx: &egui::Context, id: egui::Id) {
    client.desktop_ui.note_editor_focus = false;
    ctx.memory_mut(|memory| memory.request_focus(id));
    editor_boundary_frame(client, ctx, vec![]);
    assert!(ctx.memory(|memory| memory.has_focus(id)));
    assert!(egui::text_edit::TextEditState::load(ctx, id).is_some());
}

fn editor_boundary_command(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        },
    }
}

fn editor_boundary_crypto_frame(
    client: &mut TimerWindowsClient,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) {
    let time = ctx.input(|input| input.time) + 0.01;
    let _ = ctx.run(
        egui::RawInput {
            time: Some(time),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| client.ui_note_crypto(ui));
        },
    );
}

#[test]
fn editor_boundary_ctrl_z_never_copies_the_previous_document_title() {
    let dir = temp_test_dir("editor_boundary_title");
    let mut client = knowledge_test_client(&dir);
    let ctx = workspace_test_context();
    let title_id = egui::Id::new("note_title_editor");
    editor_boundary_frame(&mut client, &ctx, vec![]);
    editor_boundary_focus(&mut client, &ctx, title_id);
    let first_title = client.note_title_draft.clone();

    client.select_note_by_id("doc-b");
    let second_title = client.note_title_draft.clone();
    assert_ne!(first_title, second_title);
    client.desktop_ui.note_editor_focus = false;
    ctx.memory_mut(|memory| memory.request_focus(title_id));
    editor_boundary_frame(
        &mut client,
        &ctx,
        vec![editor_boundary_command(egui::Key::Z)],
    );
    assert_eq!(client.note_title_draft, second_title);
    assert!(
        !client.note_dirty,
        "opening another page must not create an edit"
    );
    client.flush_all_pending_saves().unwrap();
    let disk = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    assert_eq!(
        disk.notes.iter().find(|n| n.id == "doc-b").unwrap().title,
        second_title
    );
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn editor_boundary_sticky_body_cannot_undo_into_another_note() {
    let dir = temp_test_dir("editor_boundary_sticky");
    let state = add_note_to_state(
        &app_state_with_note("sticky-a", "First", "PRIVATE FIRST BODY", None),
        "sticky-b",
        "Second",
        "SECOND BODY",
        200,
    );
    let mut client = test_client_for_account_scope(&dir, "editor-owner", state);
    client.tab = AppTab::Notes;
    client.select_note_by_id("sticky-a");
    let ctx = workspace_test_context();
    let body_id = egui::Id::new("sticky_note_body");
    editor_boundary_frame(&mut client, &ctx, vec![]);
    editor_boundary_focus(&mut client, &ctx, body_id);

    client.select_note_by_id("sticky-b");
    ctx.memory_mut(|memory| memory.request_focus(body_id));
    editor_boundary_frame(
        &mut client,
        &ctx,
        vec![editor_boundary_command(egui::Key::Z)],
    );
    assert_eq!(client.note_content_draft, "SECOND BODY");
    assert!(!client.note_dirty);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn editor_boundary_same_document_keeps_normal_undo_and_redo_after_save() {
    let dir = temp_test_dir("editor_boundary_same_document");
    let mut client = knowledge_test_client(&dir);
    let ctx = workspace_test_context();
    let title_id = egui::Id::new("note_title_editor");
    editor_boundary_frame(&mut client, &ctx, vec![]);
    editor_boundary_focus(&mut client, &ctx, title_id);
    let original = client.note_title_draft.clone();
    editor_boundary_frame(&mut client, &ctx, vec![egui::Event::Text(" added".into())]);
    let edited = client.note_title_draft.clone();
    assert_ne!(edited, original);
    assert!(client.save_note());
    client.select_note_by_id("doc-a");
    editor_boundary_frame(
        &mut client,
        &ctx,
        vec![editor_boundary_command(egui::Key::Z)],
    );
    assert_eq!(client.note_title_draft, original);
    editor_boundary_frame(
        &mut client,
        &ctx,
        vec![editor_boundary_command(egui::Key::Y)],
    );
    assert_eq!(client.note_title_draft, edited);
    client.flush_all_pending_saves().unwrap();
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn editor_boundary_lock_removes_plaintext_widget_histories_immediately() {
    let dir = temp_test_dir("editor_boundary_lock");
    let state = app_state_with_note("sealed", "PRIVATE TITLE", "PRIVATE BODY", None);
    let mut client = test_client_for_account_scope(&dir, "editor-lock-owner", state);
    client.tab = AppTab::Notes;
    client.select_note_by_id("sealed");
    client.note_crypto_password_draft = "editor-boundary-password".into();
    client.enable_note_encryption();
    assert!(
        !client.note_crypto_session_token.is_empty(),
        "{}",
        client.status
    );
    let ctx = workspace_test_context();
    let mut ids = vec![
        egui::Id::new("note_title_editor"),
        egui::Id::new("sticky_note_body"),
    ];
    editor_boundary_frame(&mut client, &ctx, vec![]);
    for &id in &ids {
        editor_boundary_focus(&mut client, &ctx, id);
    }
    editor_boundary_crypto_frame(&mut client, &ctx, vec![]);
    let password_id = (0..8)
        .find_map(|_| {
            editor_boundary_crypto_frame(
                &mut client,
                &ctx,
                vec![egui::Event::Key {
                    key: egui::Key::Tab,
                    physical_key: Some(egui::Key::Tab),
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            ctx.memory(|memory| memory.focused())
                .filter(|id| egui::text_edit::TextEditState::load(&ctx, *id).is_some())
        })
        .expect("the real crypto form exposes a keyboard-focusable password input");
    editor_boundary_crypto_frame(
        &mut client,
        &ctx,
        vec![egui::Event::Text("synthetic-next-password".into())],
    );
    assert_eq!(
        client.note_crypto_new_password_draft,
        "synthetic-next-password"
    );
    ids.push(password_id);
    client.lock_note_encryption();
    assert!(client.selected_note_is_locked(), "{}", client.status);
    for id in ids {
        assert!(egui::text_edit::TextEditState::load(&ctx, id).is_none());
        assert!(!ctx.memory(|memory| memory.has_focus(id)));
    }
    assert!(client.note_undo_stack.is_empty());
    assert!(client.note_redo_stack.is_empty());
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn editor_boundary_remote_encryption_of_same_id_releases_plaintext_history() {
    let dir = temp_test_dir("editor_boundary_remote_lock");
    let mut client = test_client_for_account_scope(
        &dir,
        "editor-remote-owner",
        app_state_with_note("remote-sealed", "OLD PLAIN TITLE", "OLD PLAIN BODY", None),
    );
    client.tab = AppTab::Notes;
    client.select_note_by_id("remote-sealed");
    let ctx = workspace_test_context();
    let ids = [
        egui::Id::new("note_title_editor"),
        egui::Id::new("sticky_note_body"),
    ];
    editor_boundary_frame(&mut client, &ctx, vec![]);
    for id in ids {
        editor_boundary_focus(&mut client, &ctx, id);
    }
    assert!(client.note_unlocked_record.is_none());
    let plain = serde_json::to_string(client.selected_note_ref().unwrap()).unwrap();
    let (sealed, setup_token) =
        gridtimer_native::encrypt_desktop_note_json(&plain, "synthetic-remote-password").unwrap();
    assert!(gridtimer_native::close_desktop_note_session(&setup_token));
    let replacement =
        app_data::upsert_note_app_data_json(&client.state_json, &sealed, now_millis());
    assert!(client.replace_state_with_source(replacement, "Synthetic remote encryption", "sync"));
    assert_eq!(client.selected_note_id, "remote-sealed");
    assert!(client.selected_note_is_locked());
    for id in ids {
        assert!(egui::text_edit::TextEditState::load(&ctx, id).is_none());
        assert!(!ctx.memory(|memory| memory.has_focus(id)));
    }
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn editor_boundary_logout_clears_history_before_same_id_in_another_account() {
    let dir = temp_test_dir("editor_boundary_logout");
    let mut client = test_client_for_account_scope(
        &dir,
        "editor-owner-a",
        app_state_with_note("shared-id", "ACCOUNT A PRIVATE", "ACCOUNT A BODY", None),
    );
    client.tab = AppTab::Notes;
    client.select_note_by_id("shared-id");
    let ctx = workspace_test_context();
    let ids = [
        egui::Id::new("note_title_editor"),
        egui::Id::new("sticky_note_body"),
    ];
    editor_boundary_frame(&mut client, &ctx, vec![]);
    for id in ids {
        editor_boundary_focus(&mut client, &ctx, id);
    }
    // Keep logout entirely local: this synthetic session has no revocable token.
    client.sync.token.clear();
    client.sync.token_id.clear();
    client.logout();
    assert!(client.sync.user_id.is_empty(), "{}", client.status);
    for id in ids {
        assert!(egui::text_edit::TextEditState::load(&ctx, id).is_none());
    }
    let mut target = test_client_for_account_scope(
        &dir,
        "editor-owner-b",
        app_state_with_note("shared-id", "ACCOUNT B TITLE", "ACCOUNT B BODY", None),
    )
    .sync
    .clone();
    client.switch_state_scope(&mut target, false).unwrap();
    client.select_note_by_id("shared-id");
    ctx.memory_mut(|memory| memory.request_focus(ids[0]));
    editor_boundary_frame(
        &mut client,
        &ctx,
        vec![editor_boundary_command(egui::Key::Z)],
    );
    assert_eq!(client.note_title_draft, "ACCOUNT B TITLE");
    assert_eq!(client.note_content_draft, "ACCOUNT B BODY");
    assert!(!client.note_dirty);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn editor_boundary_repeated_document_visits_reclaim_body_history_states() {
    let dir = temp_test_dir("editor_boundary_body_retention");
    let mut raw: Value = serde_json::from_str(&knowledge_fixture_state()).unwrap();
    for note in raw["notes"].as_array_mut().unwrap() {
        if note["id"].as_str().is_some_and(|id| id.starts_with("doc-")) {
            let body = format!("{}\n", "synthetic content ".repeat(64)).repeat(8);
            note["content"] = json!(body);
            note["document"]["blocks"][0]["text"] = json!(body);
            note["document"]["richTextPlainText"] = json!(body);
        }
    }
    let state = app_data::sanitize_app_data_json(&raw.to_string(), now_millis()).unwrap();
    let mut client = test_client_for_account_scope(&dir, "editor-body-owner", state);
    client.tab = AppTab::Knowledge;
    let ctx = workspace_test_context();
    let mut previous_body = None;
    let mut maximum_current_states = 0;
    for visit in 0..20 {
        client.select_note_by_id(if visit % 2 == 0 { "doc-a" } else { "doc-b" });
        if let Some(id) = previous_body {
            assert!(
                egui::text_edit::TextEditState::load(&ctx, id).is_none(),
                "a retired body's undo buffer must be removed, not only hidden by a new ID"
            );
        }
        let body_id = egui::Id::new((
            "knowledge_block_editor",
            client.desktop_ui.navigation.editor_epoch,
            &client.note_blocks_draft[0].id,
        ));
        editor_boundary_frame(&mut client, &ctx, vec![]);
        editor_boundary_focus(&mut client, &ctx, body_id);
        let count = ctx.data(|data| data.count::<egui::text_edit::TextEditState>());
        if visit < 2 {
            maximum_current_states = maximum_current_states.max(count);
        } else {
            assert!(
                count <= maximum_current_states,
                "edit-state count grew at visit {visit}: {count}"
            );
        }
        previous_body = Some(body_id);
    }
    client.clear_note_draft_without_flush();
    assert!(egui::text_edit::TextEditState::load(&ctx, previous_body.unwrap()).is_none());
    assert!(
        egui::text_edit::TextEditState::load(&ctx, egui::Id::new("note_title_editor")).is_none()
    );
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}
