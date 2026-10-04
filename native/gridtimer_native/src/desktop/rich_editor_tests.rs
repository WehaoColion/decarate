// v2.22.52 - Enforce knowledge page locks for rich text open and late commits.
// v2.22.41 - Exercise the property expander before editing metadata.
// v2.22.39 - Verify and reuse shared document formatting for Windows.
// v2.22.38 - Verify unchanged opens, rejected snapshots, journal saves and preserved metadata.
#[test]
fn rich_editor_page_escapes_script_content_and_uses_local_images() {
    let page = build_desktop_rich_editor_page(
        "</script><script>alert(1)</script>",
        "<p>正文</p><img src=\"note-image://image-a\">",
    );
    assert!(!page.contains("</script><script>alert(1)"));
    assert!(page.contains("\\u003c/script\\u003e"));
    assert!(page.contains("http://note-image.localhost/image-a"));
    assert!(page.contains("(?:note-image:"));
    assert!(!page.contains("__RICH_EDITOR_INITIAL__"));
    assert!(page.contains("requestCommitSnapshot"));
    assert!(page.contains("event.isComposing"));
    assert!(page.contains("default-src 'none'"));
}

#[test]
fn rich_editor_preserves_nontext_blocks_and_rejects_foreign_media() {
    let mut document = DesktopNoteDocument::default();
    document.blocks = vec![
        DesktopNoteBlock {
            id: "text-a".into(),
            block_type: "TEXT".into(),
            text: "before".into(),
            ..Default::default()
        },
        DesktopNoteBlock {
            id: "call-a".into(),
            block_type: "CALL".into(),
            call_contact_name: "联系人".into(),
            ..Default::default()
        },
    ];
    document
        .extra
        .insert("futureDocumentFlag".into(), json!(true));
    document.blocks[0]
        .extra
        .insert("futureTextFlag".into(), json!({"source": "android"}));
    let ids = HashSet::from(["image-a".to_string()]);
    let after = rich_editor_document(
        &document,
        "<p><b>内容</b></p><img src=\"http://note-image.localhost/image-a\">",
        "内容",
        &ids,
    )
    .unwrap();
    assert!(after.rich_text_enabled);
    assert_eq!(after.blocks[0].id, "text-a");
    assert_eq!(after.blocks[0].extra, document.blocks[0].extra);
    assert_eq!(after.blocks[1], document.blocks[1]);
    assert_eq!(after.extra, document.extra);
    assert!(after.blocks[0].text.contains("note-image://image-a"));
    assert!(
        rich_editor_document(&document, "<img src=\"note-image://foreign\">", "", &ids).is_err()
    );
    assert!(rich_editor_document(&document, &"x".repeat(1_048_577), "", &ids).is_err());
}

#[test]
fn rich_editor_opening_unchanged_saved_document_does_not_write_a_new_revision() {
    let dir = temp_test_dir("unchanged_rich_editor");
    let html = "<p><strong>保留格式的正文</strong></p>";
    let mut note = desktop_note_save_value(
        "unchanged-rich",
        DesktopNoteKind::Document,
        "原有标题",
        "保留格式的正文",
        None,
        100,
    );
    note["document"] = json!({
        "markdownEnabled": false, "richTextEnabled": true,
        "richTextPlainText": "保留格式的正文",
        "blocks": [{"id": "text-a", "type": "TEXT", "text": html}]
    });
    let initial = app_data::upsert_note_app_data_json(
        &app_data::default_app_data_json(100),
        &note.to_string(),
        200,
    )
    .unwrap();
    let mut client = test_client_for_account_scope(&dir, "account-a", initial);
    client.tab = AppTab::Knowledge;
    client.select_note_by_id("unchanged-rich");
    client.open_rich_editor();
    let state = client.state_json.clone();
    let revision = client.persistence.committed_revision;
    let (sender, receiver) = mpsc::channel();
    client.rich_editor.receiver = Some(receiver);
    for (kind, request) in [("change", ""), ("commit", "save")] {
        sender
            .send(
                json!({
                    "kind": kind, "request_id": request, "title": "原有标题",
                    "html": html, "plain_text": "保留格式的正文"
                })
                .to_string(),
            )
            .unwrap();
        client.poll_rich_editor_events(&egui::Context::default());
        assert!(!client.note_dirty);
        assert!(!client.persistence.pending());
        assert_eq!(client.persistence.committed_revision, revision);
        assert_eq!(client.rich_editor.last_status, "已保存");
        assert_eq!(client.state_json, state);
    }
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rich_editor_rejected_snapshot_keeps_error_visible_until_valid_snapshot_is_saved() {
    let dir = temp_test_dir("rejected_rich_editor");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_state_with_note("rich-a", "原有标题", "原有正文", None),
    );
    client.select_note_by_id("rich-a");
    client.open_rich_editor();
    let state = client.state_json.clone();
    let (sender, receiver) = mpsc::channel();
    client.rich_editor.receiver = Some(receiver);
    sender
        .send(
            json!({
                "kind": "commit", "request_id": "close", "title": "尚未保存的标题",
                "html": "<p>当前正文</p><img src=\"note-image://foreign\">",
                "plain_text": "当前正文"
            })
            .to_string(),
        )
        .unwrap();
    let context = egui::Context::default();
    client.poll_rich_editor_events(&context);
    for _ in 0..2 {
        assert!(client.rich_editor.active);
        assert!(client
            .rich_editor
            .last_status
            .contains("图片不属于当前文档"));
        assert!(!client.rich_editor.close_after_save);
        assert!(!client.rich_editor.close_application);
        assert!(!client.note_dirty);
        assert!(!client.persistence.pending());
        assert_eq!(client.state_json, state);
        client.poll_rich_editor_events(&context);
    }
    sender
        .send(
            json!({
                "kind": "commit", "request_id": "save", "title": "修正后的标题",
                "html": "<p><strong>修正后的正文</strong></p>", "plain_text": "修正后的正文"
            })
            .to_string(),
        )
        .unwrap();
    client.poll_rich_editor_events(&context);
    drain_draft_writer(&mut client);
    client.poll_rich_editor_events(&context);
    assert!(client.rich_editor.message_error.is_none());
    assert_eq!(client.rich_editor.last_status, "已保存");
    let disk = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    let saved = disk.notes.iter().find(|note| note.id == "rich-a").unwrap();
    assert_eq!(saved.title, "修正后的标题");
    assert_eq!(saved.document.rich_text_plain_text, "修正后的正文");
    assert!(saved.document.blocks[0]
        .text
        .contains("<strong>修正后的正文</strong>"));
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rich_editor_sync_save_and_async_autosave_survive_reopen() {
    let dir = temp_test_dir("rich_editor_journal");
    let initial = app_state_with_note("rich-a", "初始标题", "原文", None);
    let mut client = test_client_for_account_scope(&dir, "account-a", initial);
    client.select_note_by_id("rich-a");
    client.open_rich_editor();
    assert!(client.rich_editor.active, "{}", client.status);
    let message = |title: &str, html: &str, plain: &str| RichEditorMessage {
        kind: "change".into(),
        html: html.into(),
        plain_text: plain.into(),
        title: title.into(),
        request_id: String::new(),
    };
    client
        .accept_rich_editor_message(message(
            "编辑后的标题",
            "<p><b>第一版正文</b></p>",
            "第一版正文",
        ))
        .unwrap();
    assert!(client.save_note(), "{}", client.status);
    let first = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    let saved = first.notes.iter().find(|n| n.id == "rich-a").unwrap();
    assert!(saved.document.rich_text_enabled);
    assert!(saved.document.blocks[0].text.contains("第一版正文"));
    client
        .accept_rich_editor_message(message(
            "自动保存标题",
            "<h2>第二版正文</h2><ul><li>事项</li></ul>",
            "第二版正文\n事项",
        ))
        .unwrap();
    client.submit_draft_snapshot(&egui::Context::default(), true);
    drain_draft_writer(&mut client);
    assert!(!client.note_dirty, "{}", client.status);
    let second = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    let saved = second.notes.iter().find(|n| n.id == "rich-a").unwrap();
    assert_eq!(saved.title, "自动保存标题");
    assert!(saved.document.rich_text_enabled);
    assert!(saved.document.blocks[0]
        .text
        .contains("<h2>第二版正文</h2>"));
    assert_eq!(saved.document.rich_text_plain_text, "第二版正文\n事项");
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rich_editor_does_not_accept_content_after_workspace_switch() {
    let dir = temp_test_dir("rich_editor_workspace");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_state_with_note("rich-a", "标题", "正文", None),
    );
    client.select_note_by_id("rich-a");
    client.open_rich_editor();
    client.rich_editor.workspace = "different-workspace".into();
    let state = client.state_json.clone();
    assert!(client
        .accept_rich_editor_message(RichEditorMessage {
            kind: "change".into(),
            html: "<p>外部内容</p>".into(),
            plain_text: "外部内容".into(),
            title: "其他账户".into(),
            request_id: String::new()
        })
        .is_err());
    assert_eq!(state, client.state_json);
    assert!(!client.note_dirty);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rich_editor_encrypted_ipc_async_save_keeps_disk_sealed_and_reopens_formatting() {
    let dir = temp_test_dir("rich_editor_encrypted_ipc_journal");
    let password = "rich-editor-fixture-password-7291";
    let old_title = "仅解锁可见的初始私密标题";
    let old_text = "仅解锁可见的初始私密正文";
    let new_title = "富文本异步保存后的私密中文标题";
    let new_text = "私密计划\n\n只在解锁后显示的中文正文\n\n- [x] 完成账目核验";
    let new_html = "<h2>私密计划</h2><p><b>只在解锁后显示的中文正文</b></p><ul data-note-todo=\"true\"><li data-done=\"true\">完成账目核验</li></ul>";
    let note_id = "encrypted-rich-document";
    let mut plain = desktop_note_save_value(
        note_id,
        DesktopNoteKind::Document,
        old_title,
        old_text,
        None,
        100,
    );
    plain["document"] = json!({
        "markdownEnabled": false,
        "richTextEnabled": true,
        "richTextPlainText": old_text,
        "blocks": [{
            "id": "existing-rich-text",
            "type": "TEXT",
            "text": format!("<p><b>{old_text}</b></p>")
        }]
    });
    let (sealed, setup_token) =
        gridtimer_native::encrypt_desktop_note_json(&plain.to_string(), password)
            .expect("seal the existing rich document before anything is written to disk");
    assert!(gridtimer_native::close_desktop_note_session(&setup_token));
    let initial =
        app_data::upsert_note_app_data_json(&app_data::default_app_data_json(100), &sealed, 200)
            .expect("existing encrypted document fixture");
    let mut client = test_client_for_account_scope(&dir, "account-a", initial);
    client.tab = AppTab::Knowledge;
    client.select_note_by_id(note_id);
    assert!(client.selected_note_is_locked());
    client.note_crypto_password_draft = password.to_string();
    client.unlock_note_encryption();
    assert!(!client.selected_note_is_locked(), "{}", client.status);
    assert_eq!(client.note_title_draft, old_title);
    assert!(client.selected_note().unwrap().document.rich_text_enabled);
    client.open_rich_editor();
    assert!(client.rich_editor.active, "{}", client.status);

    // Deliver the same serialized IPC message and receiver used by WebView2;
    // poll_rich_editor_events must submit the real background commit_drafts.
    let (sender, receiver) = mpsc::channel();
    client.rich_editor.receiver = Some(receiver);
    let before_revision = client.persistence.committed_revision;
    sender
        .send(
            json!({
                "kind": "commit",
                "request_id": "save",
                "title": new_title,
                "html": new_html,
                "plain_text": new_text
            })
            .to_string(),
        )
        .unwrap();
    client.poll_rich_editor_events(&egui::Context::default());
    drain_draft_writer(&mut client);
    assert!(
        client.persistence.committed_revision > before_revision,
        "IPC did not complete an asynchronous journal commit: {}",
        client.status
    );
    assert!(!client.note_dirty, "{}", client.status);
    assert_ne!(
        client.note_save_state,
        DesktopDocumentSaveState::Failed,
        "{}",
        client.status
    );
    let state_path = client.state_path.clone();
    client.close_note_crypto_session();
    drop(client);

    // Scan every file in this isolated test workspace, including SQLite pages,
    // WAL, mirrors and backups. The fixture was encrypted before initial save,
    // so neither the original content nor the edited content may appear there.
    let markers = [
        old_title,
        old_text,
        new_title,
        new_text,
        new_html,
        "只在解锁后显示的中文正文",
        password,
    ];
    let mut pending = vec![dir.clone()];
    let mut scanned_files = 0;
    while let Some(folder) = pending.pop() {
        for entry in fs::read_dir(&folder).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            assert!(
                !kind.is_symlink(),
                "test fixture must not escape its workspace"
            );
            if kind.is_dir() {
                pending.push(entry.path());
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            let bytes = fs::read(entry.path()).unwrap();
            scanned_files += 1;
            for marker in markers {
                assert!(
                    !bytes
                        .windows(marker.len())
                        .any(|part| part == marker.as_bytes()),
                    "plaintext leaked into {}",
                    entry.path().display()
                );
                let utf16 = marker
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes)
                    .collect::<Vec<_>>();
                assert!(
                    !bytes
                        .windows(utf16.len())
                        .any(|part| part == utf16.as_slice()),
                    "UTF-16 plaintext leaked into {}",
                    entry.path().display()
                );
            }
        }
    }
    assert!(
        scanned_files >= 2,
        "must verify the production state and journal artifacts"
    );

    let disk = decode_data(&fs::read_to_string(&state_path).unwrap());
    let stored = disk.notes.iter().find(|note| note.id == note_id).unwrap();
    assert!(stored.encryption.is_some());
    assert_ne!(stored.title, new_title);
    let (reopened_json, verify_token) = gridtimer_native::unlock_desktop_note_json(
        &serde_json::to_string(stored).unwrap(),
        password,
    )
    .expect("decrypt the newly persisted envelope in a fresh session");
    let reopened: DesktopNote = serde_json::from_str(&reopened_json).unwrap();
    assert!(gridtimer_native::close_desktop_note_session(&verify_token));
    assert_eq!(desktop_note_kind(&reopened), DesktopNoteKind::Document);
    assert_eq!(reopened.title, new_title);
    assert_eq!(reopened.content, new_text);
    assert!(reopened.document.rich_text_enabled);
    assert!(!reopened.document.markdown_enabled);
    assert_eq!(reopened.document.rich_text_plain_text, new_text);
    assert_eq!(reopened.document.blocks[0].id, "existing-rich-text");
    let html = &reopened.document.blocks[0].text;
    assert!(html.contains("<h2>私密计划</h2>"));
    assert!(html.contains("<strong>只在解锁后显示的中文正文</strong>"));
    assert!(html.contains("data-note-todo=\"true\""));
    assert!(html.contains("data-done=\"true\""));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rich_editor_navigation_accepts_only_the_internal_page() {
    assert!(rich_editor_navigation_allowed(RICH_EDITOR_PAGE_URL));
    assert!(rich_editor_navigation_allowed("about:blank"));
    for url in [
        "https://example.com",
        "javascript:alert(1)",
        "file:///C:/a.html",
        "data:text/html,<script>bad()</script>",
        "http://note-editor.localhost/foreign",
        "http://note-editor.localhost.evil/",
    ] {
        assert!(!rich_editor_navigation_allowed(url), "{url}");
    }
}

#[test]
fn empty_text_document_has_no_internal_call_marker_in_preview_or_search() {
    let note: DesktopNote = serde_json::from_value(json!({
        "id":"empty-text", "title":"空白文档", "content":"", "kind":"DOCUMENT",
        "document":{"blocks":[{"id":"text","type":"TEXT","text":"","callDirection":"UNKNOWN"}]}
    }))
    .unwrap();
    assert_eq!(note_body_text(&note), "");
    assert_eq!(
        desktop_note_document_text(&note.content, &note.document),
        ""
    );
    assert!(!note_requires_protected_edit(&note));
}

#[test]
fn leaving_an_untouched_new_rich_editor_does_not_create_an_empty_document() {
    let dir = temp_test_dir("empty_rich_editor_return");
    let mut client =
        test_client_for_account_scope(&dir, "account-a", app_data::default_app_data_json(100));
    client.tab = AppTab::Knowledge;
    client.open_rich_editor();
    assert!(client.rich_editor.active);
    let (sender, receiver) = mpsc::channel();
    client.rich_editor.receiver = Some(receiver);
    sender.send(json!({"kind":"commit","request_id":"close","title":"","html":"<p><br></p>","plain_text":""}).to_string()).unwrap();
    client.poll_rich_editor_events(&egui::Context::default());
    assert!(!client.rich_editor.active);
    assert!(!client.note_dirty);
    assert!(!client.persistence.pending());
    assert!(client.data.notes.is_empty());
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rich_editor_conversion_renders_markdown_without_duplicate_media_preview_text() {
    let note: DesktopNote = serde_json::from_value(json!({
        "id": "conversion", "kind": "DOCUMENT", "title": "示例",
        "content": "# 标题\n\n[图片] 截图\n\n[联系人] 张三\n\n[通话] 李四",
        "document": {
            "markdownEnabled": true,
            "blocks": [
                {"id":"t1", "type":"TEXT", "text":"# 标题\n\n**重点**\n\n- 项目"},
                {"id":"i1", "type":"IMAGE", "attachmentId":"image-a", "caption":"截图"},
                {"id":"c1", "type":"CONTACT", "contactName":"张三"},
                {"id":"call1", "type":"CALL", "callContactName":"李四"}
            ]
        }
    }))
    .unwrap();
    let html = rich_editor_initial_html(Some(&note), &note.content);
    assert!(html.contains("<h1>标题</h1>"), "{html}");
    assert!(html.contains("<strong>重点</strong>"), "{html}");
    assert!(html.contains("<li>项目</li>"), "{html}");
    assert!(!html.contains("[图片]"));
    assert!(!html.contains("[联系人]"));
    assert!(!html.contains("[通话]"));
    let saved =
        rich_editor_document(&note.document, &html, "标题\n重点\n项目", &HashSet::new()).unwrap();
    assert_eq!(&saved.blocks[1..], &note.document.blocks[1..]);
    assert!(!saved.markdown_enabled);
    assert!(saved.rich_text_enabled);
    let plain = rich_editor_initial_html(None, "第一行\n\n2 < 3");
    assert!(plain.contains("<p><br/></p>"));
    assert!(plain.contains("2 &lt; 3"));
}

fn rich_editor_usability_frame(
    client: &mut TimerWindowsClient,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    preview: bool,
) -> egui::FullOutput {
    ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(760.0, 600.0),
            )),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                if preview {
                    client.ui_note_rich_text_preview(ui);
                } else {
                    client.ui_note_canvas_header(ui);
                }
            });
        },
    )
}

#[test]
fn rich_editor_metadata_controls_save_without_rewriting_body_and_preview_opens_editor() {
    let dir = temp_test_dir("rich_metadata_controls");
    let mut value = desktop_note_save_value(
        "rich-ui",
        DesktopNoteKind::Document,
        "原有标题",
        "保留格式",
        None,
        100,
    );
    value["document"] = json!({
        "richTextEnabled": true, "richTextPlainText": "保留格式",
        "blocks": [{"id":"text-a", "type":"TEXT", "text":"<p><strong>保留格式</strong></p>"}]
    });
    let state = app_data::upsert_note_app_data_json(
        &app_data::default_app_data_json(100),
        &value.to_string(),
        200,
    )
    .unwrap();
    let mut client = test_client_for_account_scope(&dir, "account-a", state);
    client.switch_tab(AppTab::Knowledge);
    client.select_note_by_id("rich-ui");
    let original = client.selected_note().unwrap().document;
    let ctx = workspace_test_context();
    rich_editor_usability_frame(&mut client, &ctx, vec![], false);
    ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("note_title_editor")));
    rich_editor_usability_frame(
        &mut client,
        &ctx,
        vec![egui::Event::Text("新标题".into())],
        false,
    );
    assert!(
        client.note_title_draft.contains("新标题"),
        "rich document title must accept typing"
    );
    let properties = ctx
        .data(|data| data.get_temp::<egui::Rect>(egui::Id::new("knowledge_properties_toggle")))
        .expect("properties expander")
        .center();
    for pressed in [true, false] {
        rich_editor_usability_frame(
            &mut client,
            &ctx,
            vec![
                egui::Event::PointerMoved(properties),
                egui::Event::PointerButton {
                    pos: properties,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            false,
        );
    }
    for _ in 0..6 {
        rich_editor_usability_frame(&mut client, &ctx, vec![], false);
    }
    let pinned = ctx
        .data(|data| data.get_temp::<egui::Rect>(egui::Id::new("knowledge_pin")))
        .expect("expanded pin control")
        .center();
    for pressed in [true, false] {
        rich_editor_usability_frame(
            &mut client,
            &ctx,
            vec![
                egui::Event::PointerMoved(pinned),
                egui::Event::PointerButton {
                    pos: pinned,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            false,
        );
    }
    assert!(
        client.note_pinned_draft,
        "rich document properties must accept clicks"
    );
    client.undo_note_draft();
    assert!(
        !client.note_pinned_draft,
        "undo must restore the unpinned metadata"
    );
    assert!(client.note_title_draft.contains("新标题"));
    client.redo_note_draft();
    assert!(
        client.note_pinned_draft,
        "redo must restore the pinned metadata"
    );
    assert!(client.save_note(), "{}", client.status);
    let disk = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    let saved = disk.notes.iter().find(|note| note.id == "rich-ui").unwrap();
    assert!(saved.title.contains("新标题"));
    assert!(saved.pinned);
    assert_eq!(
        saved.document, original,
        "metadata saves must preserve the exact rich body"
    );

    rich_editor_usability_frame(&mut client, &ctx, vec![], true);
    let pos = ctx
        .data(|data| data.get_temp::<egui::Rect>(egui::Id::new("rich_preview_edit")))
        .expect("edit action next to the preview")
        .center();
    for pressed in [true, false] {
        rich_editor_usability_frame(
            &mut client,
            &ctx,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            true,
        );
    }
    assert!(
        client.rich_editor.active,
        "the visible edit button must open the editor"
    );
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

fn plain_note_editor_frame(
    client: &mut TimerWindowsClient,
    context: &egui::Context,
    events: Vec<egui::Event>,
) {
    let _ = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 800.0),
            )),
            events,
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                client.ui_sticky_note_editor(ui);
            });
        },
    );
}

fn plain_note_editor_enter(client: &mut TimerWindowsClient, context: &egui::Context) {
    // Use actual key events in separate frames, never a newline in Event::Text.
    // The key-up/idle frame used to rebuild TextEdit from a trimmed draft.
    for pressed in [true, false] {
        plain_note_editor_frame(
            client,
            context,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: Some(egui::Key::Enter),
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }
}

#[test]
fn plain_note_editor_enter_survives_frame_rebuild_and_second_line_input() {
    let dir = temp_test_dir("plain_enter_frames");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_data::default_app_data_json(now_millis()),
    );
    client.switch_tab(AppTab::Notes);
    client.new_note();
    let context = workspace_test_context();
    plain_note_editor_frame(&mut client, &context, vec![]);
    context.memory_mut(|memory| memory.request_focus(egui::Id::new("sticky_note_body")));
    plain_note_editor_frame(
        &mut client,
        &context,
        vec![egui::Event::Text("第一行".into())],
    );
    assert_eq!(client.note_content_draft, "第一行");
    plain_note_editor_enter(&mut client, &context);
    assert_eq!(
        client.note_blocks_draft[0].text, "第一行\n",
        "the Enter event reached the editor"
    );
    assert_eq!(
        client.note_content_draft, "第一行\n",
        "the next frame must retain the Enter"
    );
    plain_note_editor_frame(&mut client, &context, vec![]);
    plain_note_editor_frame(
        &mut client,
        &context,
        vec![egui::Event::Text("第二行".into())],
    );
    assert_eq!(client.note_content_draft, "第一行\n第二行");
    assert!(client.save_note(), "{}", client.status);
    let id = client.selected_note_id.clone();
    let disk = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    let saved = disk.notes.iter().find(|note| note.id == id).unwrap();
    assert_eq!(saved.content, "第一行\n第二行");
    assert_eq!(saved.document.blocks[0].text, "第一行\n第二行");
    client.load_note_draft_without_flush(saved);
    assert_eq!(client.note_content_draft, "第一行\n第二行");
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn plain_note_editor_preserves_leading_and_consecutive_blank_lines_across_save_and_tabs() {
    let dir = temp_test_dir("plain_whitespace_navigation");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_data::default_app_data_json(now_millis()),
    );
    client.switch_tab(AppTab::Notes);
    client.new_note();
    let context = workspace_test_context();
    plain_note_editor_frame(&mut client, &context, vec![]);
    context.memory_mut(|memory| memory.request_focus(egui::Id::new("sticky_note_body")));
    plain_note_editor_enter(&mut client, &context);
    plain_note_editor_enter(&mut client, &context);
    assert_eq!(
        client.note_content_draft, "\n\n",
        "leading blank lines are editable content"
    );
    plain_note_editor_frame(
        &mut client,
        &context,
        vec![egui::Event::Text("  第一行".into())],
    );
    plain_note_editor_frame(&mut client, &context, vec![egui::Event::Text("  ".into())]);
    assert_eq!(
        client.note_content_draft, "\n\n  第一行  ",
        "spaces survive a separate input frame"
    );
    plain_note_editor_enter(&mut client, &context);
    plain_note_editor_enter(&mut client, &context);
    plain_note_editor_frame(
        &mut client,
        &context,
        vec![egui::Event::Text("第二行".into())],
    );
    plain_note_editor_enter(&mut client, &context);
    let expected = "\n\n  第一行  \n\n第二行\n";
    assert_eq!(client.note_content_draft, expected);
    client.switch_tab(AppTab::Board);
    let disk = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    let saved = disk
        .notes
        .iter()
        .find(|note| {
            note.document
                .blocks
                .iter()
                .any(|block| block.text == expected)
        })
        .expect("tab switch must save every line including trailing whitespace");
    let id = saved.id.clone();
    assert_eq!(saved.document.blocks[0].text, expected);
    client.switch_tab(AppTab::Notes);
    assert_eq!(client.selected_note_id, id);
    assert_eq!(client.note_content_draft, expected);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn plain_note_editor_storage_keeps_whitespace_blocks_and_media_descriptions() {
    let blocks = vec![
        new_desktop_text_block("\n  "),
        DesktopNoteBlock {
            id: "image-a".into(),
            block_type: "IMAGE".into(),
            caption: "截图".into(),
            ..Default::default()
        },
        new_desktop_text_block("末行\n"),
    ];
    assert_eq!(
        note_canvas_storage_text(&blocks),
        "\n  \n\n[图片] 截图\n\n末行\n"
    );
}

#[test]
fn rich_editor_plain_conversion_preserves_ascii_whitespace_and_trailing_empty_paragraphs() {
    let source = "  第一行  \n\n第二行\t\n\n";
    let html = rich_editor_initial_html(None, source);
    assert_eq!(
        html,
        "<p>  第一行  </p><p><br/></p><p>第二行\t</p><p><br/></p><p><br/></p>"
    );
    assert!(
        !html.contains("&nbsp;"),
        "spaces must remain original ASCII characters"
    );
    let document = rich_editor_document(
        &DesktopNoteDocument::default(),
        &html,
        "第一行\n\n第二行",
        &HashSet::new(),
    )
    .unwrap();
    assert!(document.blocks[0].text.contains("<p>  第一行  </p>"));
    assert!(document.blocks[0].text.contains("<p>第二行\t</p>"));
    assert_eq!(document.blocks[0].text.matches("<p>").count(), 5);
    let note = DesktopNote {
        document,
        ..Default::default()
    };
    let reopened =
        rich_editor_initial_html(Some(&note), "a plain-text digest must not replace HTML");
    assert!(reopened.contains("<p>  第一行  </p>"));
    assert_eq!(reopened.matches("<p>").count(), 5);
    assert_eq!(rich_editor_initial_html(None, ""), "");
}

#[test]
fn knowledge_rich_text_page_lock_is_enforced_at_open_and_commit() {
    let dir = temp_test_dir("knowledge_rich_page_lock");
    let note = json!({
        "id": "locked-rich",
        "kind": "DOCUMENT",
        "title": "原有标题",
        "document": {
            "richTextEnabled": true,
            "richTextPlainText": "原有正文",
            "knowledge": {"locked": true, "tags": ["资料"], "icon": "R"},
            "blocks": [{"id": "rich-body", "type": "TEXT", "text": "<p><strong>原有正文</strong></p>"}]
        }
    });
    let initial = app_data::upsert_note_app_data_json(
        &app_data::default_app_data_json(100),
        &note.to_string(),
        200,
    )
    .unwrap();
    let mut client = test_client_for_account_scope(&dir, "account-a", initial);
    client.tab = AppTab::Knowledge;
    client.select_note_by_id("locked-rich");
    let original_state = client.state_json.clone();
    let original_draft = client.current_note_draft_snapshot();
    client.open_rich_editor();
    assert!(!client.rich_editor.active);
    assert_eq!(client.state_json, original_state);
    assert_eq!(client.current_note_draft_snapshot(), original_draft);

    let mut page = client.desktop_ui.knowledge.page.clone().unwrap();
    page.locked = false;
    assert!(client.update_knowledge_page(page.clone()));
    assert!(client.save_note(), "{}", client.status);
    client.open_rich_editor();
    assert!(client.rich_editor.active);
    let message = || {
        serde_json::from_value(json!({
            "kind": "commit", "request_id": "save", "title": "更新标题",
            "html": "<p><strong>更新正文</strong></p>", "plain_text": "更新正文"
        }))
        .unwrap()
    };
    client.accept_rich_editor_message(message()).unwrap();
    assert!(client.save_note(), "{}", client.status);
    assert_eq!(
        client.selected_note().unwrap().document.knowledge,
        Some(page)
    );
    assert!(client.selected_note().unwrap().document.rich_text_enabled);

    client.desktop_ui.knowledge.page.as_mut().unwrap().locked = true;
    let committed = client.state_json.clone();
    let locked_draft = client.current_note_draft_snapshot();
    assert!(client.accept_rich_editor_message(message()).is_err());
    assert_eq!(client.state_json, committed);
    assert_eq!(client.current_note_draft_snapshot(), locked_draft);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}
