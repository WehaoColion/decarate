// v2.22.54 - Keep mutable fixture overlays separate from shared read caches.
// v2.22.53 - Focused knowledge workspace, contextual controls and safe navigation.
// v2.22.52 - Portable files and an opt-in, fully isolated native workspace.

#[test]
fn knowledge_portable_file_roundtrip_and_failed_import_leave_original_intact() {
    let dir = temp_test_dir("knowledge_portable_file");
    let mut client = knowledge_test_client(&dir);
    let source = dir.join("source.txt");
    let bytes = "附件原文\n不会因工作区导出而丢失".as_bytes();
    fs::write(&source, bytes).unwrap();
    let store = client.note_media_store().unwrap();
    let pending = store.begin_file_import(&source, "FILE").unwrap();
    let attachment: DesktopNoteAttachment =
        serde_json::from_str(&pending.attachment_json().unwrap()).unwrap();
    let mut note = client.selected_note().unwrap();
    note.document.knowledge = Some(Default::default());
    let mut file = new_desktop_text_block("source.txt");
    file.knowledge = Some(knowledge::KnowledgeBlock {
        kind: knowledge::BlockKind::File,
        ..Default::default()
    });
    file.attachment_id = Some(attachment.id.clone());
    note.document.blocks.push(file);
    note.attachments.push(attachment.clone());
    assert!(client.replace_state(
        app_data::upsert_note_app_data_json(
            &client.state_json,
            &serde_json::to_string(&note).unwrap(),
            now_millis()
        ),
        "file added"
    ));
    store.commit_import(&pending).unwrap();
    client.refresh_selected_note_after_local_mutation(&note.id);
    let encoded = client
        .knowledge_transfer_json(client.knowledge_subtree_pages(Some(&note.id)))
        .unwrap();
    let value: Value = serde_json::from_str(&encoded).unwrap();
    let (pages, media) = prepare_knowledge_transfer(&value).unwrap();
    assert_ne!(media[0].attachment.id, attachment.id);
    let original = client.state_json.clone();
    client.workspace_persistence_ready = false;
    assert!(!client.commit_knowledge_import(&pages, &media));
    assert_eq!(client.state_json, original);
    assert!(store
        .read_blob(
            &media[0].attachment.id,
            &media[0].attachment.sha256,
            media[0].attachment.size_bytes
        )
        .is_err());
    client.workspace_persistence_ready = true;
    assert!(
        client.commit_knowledge_import(&pages, &media),
        "{}",
        client.status
    );
    assert_eq!(
        store
            .read_blob(
                &media[0].attachment.id,
                &media[0].attachment.sha256,
                media[0].attachment.size_bytes
            )
            .unwrap(),
        bytes
    );
    let imported_id = pages[0]["id"].as_str().unwrap();
    let imported = client
        .data
        .notes
        .iter()
        .find(|p| p.id == imported_id)
        .unwrap();
    assert!(imported
        .document
        .blocks
        .iter()
        .any(|b| b.attachment_id.as_deref() == Some(&media[0].attachment.id)));
    let html = knowledge_complete_html(imported, |a| {
        store
            .read_blob(&a.id, &a.sha256, a.size_bytes)
            .map_err(|e| e.to_string())
    })
    .unwrap();
    assert!(html.contains("data:text/plain;base64,"));
    let mut corrupt = value.clone();
    corrupt["media"][0]["contentBase64"] = json!("YWJj");
    assert!(prepare_knowledge_transfer(&corrupt).is_err());
    corrupt = value.clone();
    corrupt["pages"][0]["attachments"][0]["sha256"] = json!("0".repeat(64));
    assert!(prepare_knowledge_transfer(&corrupt).is_err());
    corrupt = value.clone();
    corrupt["pages"][0]["attachments"][0]["width"] = json!(123);
    assert!(prepare_knowledge_transfer(&corrupt).is_err());
    let deleted = app_data::delete_note_attachment_app_data_json(
        &client.state_json,
        imported_id,
        &media[0].attachment.id,
        now_millis(),
    )
    .unwrap();
    let deleted = decode_data(&deleted);
    let deleted_note = deleted.notes.iter().find(|p| p.id == imported_id).unwrap();
    assert!(!deleted_note
        .document
        .blocks
        .iter()
        .any(|b| b.attachment_id.as_deref() == Some(&media[0].attachment.id)));
    let original_bytes = store
        .read_blob(&attachment.id, &attachment.sha256, attachment.size_bytes)
        .unwrap();
    assert_eq!(original_bytes, bytes);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn knowledge_structured_page_cannot_be_flattened_by_the_legacy_rich_editor() {
    let dir = temp_test_dir("knowledge_editor_guard");
    let mut client = knowledge_test_client(&dir);
    client.insert_knowledge_block(knowledge::BlockKind::Table, None, 0, None);
    assert!(client.save_note());
    let before = client.current_note_draft_snapshot();
    client.open_rich_editor();
    assert!(!client.rich_editor.active);
    assert_eq!(client.current_note_draft_snapshot(), before);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(target_os = "windows")]
#[test]
#[ignore = "Interactive native acceptance uses only a new synthetic workspace"]
fn knowledge_workspace_native_acceptance() {
    use winit::platform::windows::EventLoopBuilderExtWindows;
    let output = PathBuf::from(
        std::env::var_os("KNOWLEDGE_NATIVE_ACCEPTANCE_ROOT")
            .expect("explicit isolated output root"),
    );
    fs::create_dir(&output).expect("acceptance directory must be new");
    let mut client = knowledge_test_client(&output);
    client.sync.token.clear();
    client.sync.server_url.clear();
    client.sync.ai_api_key.clear();
    client.settings.timer_bell_enabled = false;
    client.settings.close_to_tray = false;
    let mut pages = vec![
        json!({"id":"workspace","kind":"DOCUMENT","title":"知识工作区","pinned":true,"document":{"knowledge":{"icon":"◈","cover":"海盐","tags":["项目","资料"]},"blocks":knowledge::markdown_blocks("# 从想法到交付\n这里保存项目资料、会议记录和需要继续研究的问题。\n\n## 本周重点\n- [ ] 整理下一阶段需求\n- [x] 完成资料归档\n\n> 记录结论，也保留判断的依据。",new_desktop_note_block_id)}}),
    ];
    let mut blocks =
        knowledge::markdown_blocks("# 研究笔记\n## 公式与代码\n", new_desktop_note_block_id);
    blocks.push(json!({"id":"equation-demo","type":"TEXT","text":"\\int_0^1 x^2\\,dx = \\frac{1}{3}","knowledge":{"kind":"equation"}}));
    blocks.push(json!({"id":"code-demo","type":"TEXT","text":"fn main() {\n    println!(\"Hello\");\n}","knowledge":{"kind":"code","language":"Rust"}}));
    blocks.push(
        json!({"id":"columns-demo","type":"TEXT","knowledge":{"kind":"columns","columns":2}}),
    );
    blocks.push(json!({"id":"left-demo","type":"TEXT","text":"问题\n现象与原因", "knowledge":{"kind":"callout","parentId":"columns-demo","column":0}}));
    blocks.push(json!({"id":"right-demo","type":"TEXT","text":"下一步\n做一轮有边界的验证", "knowledge":{"kind":"callout","parentId":"columns-demo","column":1}}));
    pages.push(json!({"id":"research","kind":"DOCUMENT","title":"研究笔记","document":{"knowledge":{"parentId":"workspace","icon":"☷"},"blocks":blocks}}));
    let database = knowledge::KnowledgeDatabase::task_database();
    pages.push(json!({"id":"tasks","kind":"DOCUMENT","title":"项目任务","document":{"knowledge":{"parentId":"workspace","icon":"▦","database":database},"blocks":[]}}));
    for (i, title) in [
        "用户访谈",
        "梳理需求",
        "原型走查",
        "整理测试记录",
        "更新交付说明",
        "下一阶段计划",
    ]
    .iter()
    .enumerate()
    {
        let status = ["未开始", "进行中", "已完成"][i % 3];
        let day = 16 + i;
        pages.push(json!({"id":format!("row-{i}"),"kind":"DOCUMENT","title":title,"document":{"knowledge":{"parentId":"tasks","properties":{"status":{"kind":"select","value":status},"priority":{"kind":"select","value":(["高","中","低"][i%3])},"date":{"kind":"date","value":{"start":format!("2026-09-{day:02}"),"end":format!("2026-09-{:02}",day+2)}}}},"blocks":knowledge::markdown_blocks("## 交付标准\n- [ ] 记录验收结果",new_desktop_note_block_id)}}));
    }
    assert!(client.replace_state(
        app_data::upsert_knowledge_pages_app_data_json(
            &client.state_json,
            &serde_json::to_string(&pages).unwrap(),
            now_millis()
        ),
        "测试资料已就绪"
    ));
    assert!(client.replace_state(
        app_data::set_selected_note_folder_app_data_json(&client.state_json, "", now_millis()),
        "测试资料已就绪"
    ));
    client.select_note_by_id("workspace");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1240.0, 820.0])
            .with_position([30.0, 30.0]),
        event_loop_builder: Some(Box::new(|b| {
            b.with_any_thread(true);
        })),
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        "十倍率 · 知识工作区验收",
        options,
        Box::new(move |cc| {
            install_ui_fonts(&cc.egui_ctx);
            install_ui_style(&cc.egui_ctx);
            if std::env::var_os("KNOWLEDGE_NATIVE_CAPTURE").is_some() {
                Box::new(KnowledgeWindowReview {
                    client,
                    output,
                    scene: 0,
                    frames: 0,
                    started: Instant::now(),
                })
            } else {
                Box::new(client)
            }
        }),
    )
    .unwrap();
}

#[cfg(target_os = "windows")]
struct KnowledgeWindowReview {
    client: TimerWindowsClient,
    output: PathBuf,
    scene: usize,
    frames: usize,
    started: Instant,
}

#[cfg(target_os = "windows")]
impl eframe::App for KnowledgeWindowReview {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        const SCENES: &[(&str, &str, Option<usize>, u8, f32, f32)] = &[
            ("library", "workspace", None, 1, 1240.0, 820.0),
            ("library_board", "workspace", None, 1, 1240.0, 820.0),
            ("library_folders", "workspace", None, 1, 1240.0, 820.0),
            ("database_options", "tasks", Some(0), 1, 1240.0, 820.0),
            ("navigator_compact", "workspace", None, 1, 760.0, 560.0),
            ("workspace", "workspace", None, 1, 1240.0, 820.0),
            ("outline", "workspace", None, 1, 1440.0, 900.0),
            ("properties", "workspace", None, 1, 1440.0, 900.0),
            ("focus", "workspace", None, 1, 1240.0, 820.0),
            ("quick_switch", "workspace", None, 1, 1240.0, 820.0),
            ("blocks", "research", None, 1, 1240.0, 820.0),
            ("table", "tasks", Some(0), 1, 1240.0, 820.0),
            ("board", "tasks", Some(1), 1, 1240.0, 820.0),
            ("list", "tasks", Some(2), 1, 1240.0, 820.0),
            ("gallery", "tasks", Some(3), 1, 1240.0, 820.0),
            ("calendar", "tasks", Some(4), 1, 1240.0, 820.0),
            ("timeline", "tasks", Some(5), 1, 1240.0, 820.0),
            ("form", "tasks", Some(6), 1, 1240.0, 820.0),
            ("workspace_dark", "workspace", None, 2, 1240.0, 820.0),
            ("database_compact", "tasks", Some(0), 1, 760.0, 560.0),
            ("page_compact", "workspace", None, 1, 760.0, 560.0),
            ("palette_compact", "workspace", None, 1, 760.0, 560.0),
            ("inspector_compact", "workspace", None, 1, 760.0, 560.0),
        ];
        if self.scene == SCENES.len() {
            fs::write(self.output.join("window_render_result.json"), json!({"passed":true,"scenes":SCENES.iter().map(|s|s.0).collect::<Vec<_>>(),"capture":"complete native client window","mouseVerified":false}).to_string()).unwrap();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        assert!(
            self.started.elapsed() < Duration::from_secs(150),
            "native knowledge window capture timeout"
        );
        if self.frames > 0 {
            let image = ctx.input(|i| {
                i.events.iter().find_map(|e| {
                    if let egui::Event::Screenshot { image, .. } = e {
                        Some(image.clone())
                    } else {
                        None
                    }
                })
            });
            if let Some(image) = image {
                let pixels = image
                    .pixels
                    .iter()
                    .flat_map(|p| p.to_array())
                    .collect::<Vec<_>>();
                image::save_buffer(
                    self.output.join(format!("{}.png", SCENES[self.scene].0)),
                    &pixels,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.scene += 1;
                self.frames = 0;
                ctx.request_repaint();
                return;
            }
        }
        let (scene, page, view, theme, width, height) = SCENES[self.scene];
        if self.frames == 0 {
            self.client.select_note_by_id(page);
            self.client.observe_knowledge_navigation();
            self.client.desktop_ui.experience.focus_mode = scene == "focus";
            self.client.desktop_ui.experience.palette_open = false;
            self.client.desktop_ui.experience.inspector = match scene {
                "outline" => Some(KnowledgeInspector::Outline),
                "properties" | "inspector_compact" => Some(KnowledgeInspector::Properties),
                _ => None,
            };
            self.client.desktop_ui.experience.navigator_drawer_open = scene == "navigator_compact";
            self.client.desktop_ui.knowledge.options_open = scene == "database_options";
            self.client.knowledge_view = match scene {
                "library_board" => KnowledgeCollectionView::Board,
                "library_folders" => KnowledgeCollectionView::Library,
                _ => KnowledgeCollectionView::Recent,
            };
            if scene.starts_with("library") {
                self.client.return_to_document_list();
            }
            if scene == "quick_switch" || scene == "palette_compact" {
                self.client.open_knowledge_quick_switch();
            }
            if let Some(view) = view {
                self.client.desktop_ui.knowledge.active_view = format!("view-{view}");
            }
            self.client.data.theme_mode = if theme == 2 { "DARK" } else { "LIGHT" }.into();
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(width, height)));
        }
        apply_active_desktop_theme(ctx, &self.client.data, frame.info().system_theme, None);
        self.client.refresh_timer_projection(now_millis());
        ctx.input_mut(|i| i.pointer = Default::default());
        self.client.ui_workspace(ctx);
        self.frames += 1;
        if self.frames == 8 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
        }
        ctx.request_repaint_after(Duration::from_millis(80));
    }
}

#[test]
fn knowledge_experience_navigation_preserves_failed_draft_and_history_branch() {
    let dir = temp_test_dir("knowledge_experience_navigation");
    let mut client = knowledge_test_client(&dir);
    client.observe_knowledge_navigation();
    client.select_note_by_id("doc-b");
    client.observe_knowledge_navigation();
    assert_eq!(client.desktop_ui.experience.trail, vec!["doc-a", "doc-b"]);
    client.set_note_canvas_text("未保存的会议结论");
    client.workspace_persistence_ready = false;
    assert!(!client.navigate_knowledge_trail(-1));
    assert_eq!(client.selected_note_id, "doc-b");
    assert_eq!(client.note_content_draft, "未保存的会议结论");
    assert_eq!(client.desktop_ui.experience.cursor, 1);
    assert!(!client.execute_knowledge_quick_choice(KnowledgeQuickChoice::Page("doc-a".into())));
    assert_eq!(client.note_content_draft, "未保存的会议结论");
    client.workspace_persistence_ready = true;
    assert!(client.navigate_knowledge_trail(-1));
    assert_eq!(client.selected_note_id, "doc-a");
    assert!(client.navigate_knowledge_trail(1));
    assert_eq!(client.note_content_draft, "未保存的会议结论");
    assert!(client.navigate_knowledge_trail(-1));
    assert!(client.execute_knowledge_quick_choice(KnowledgeQuickChoice::Home));
    assert!(client.knowledge_trail_target(1).is_none());
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn knowledge_experience_palette_keeps_private_titles_out_of_search_and_resets_scope() {
    let dir = temp_test_dir("knowledge_experience_scope");
    let mut client = knowledge_test_client(&dir);
    client.open_knowledge_quick_switch();
    let note = client
        .data
        .notes
        .iter_mut()
        .find(|p| p.id == "doc-b")
        .unwrap();
    note.title = "Sensitive journal".into();
    note.encryption = Some(Default::default());
    client.desktop_ui.experience.palette_query = "Sensitive".into();
    assert!(client.knowledge_quick_choices().is_empty());
    client.desktop_ui.experience.palette_query.clear();
    let choices = client.knowledge_quick_choices();
    assert!(choices
        .iter()
        .any(|(c, t, _)| *c == KnowledgeQuickChoice::Page("doc-b".into()) && t == "加密页面"));
    assert!(!choices
        .iter()
        .any(|(c, _, _)| *c == KnowledgeQuickChoice::Page("sticky-a".into())));
    client
        .desktop_ui
        .experience
        .trail
        .push("private-old-scope".into());
    client.sync.user_id = "different-user".into();
    client.observe_knowledge_navigation();
    assert!(!client.desktop_ui.experience.palette_open);
    assert!(!client
        .desktop_ui
        .experience
        .trail
        .contains(&"private-old-scope".into()));
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn knowledge_experience_keyboard_palette_and_escape_do_not_edit_or_leave_page() {
    let dir = temp_test_dir("knowledge_experience_keyboard");
    let mut client = knowledge_test_client(&dir);
    let ctx = workspace_test_context();
    let size = egui::vec2(1240.0, 820.0);
    let original = client.state_json.clone();
    for _ in 0..3 {
        workspace_test_frame(&mut client, &ctx, size, vec![]);
    }
    client.open_knowledge_quick_switch();
    workspace_test_frame(&mut client, &ctx, size, vec![]);
    workspace_test_frame(
        &mut client,
        &ctx,
        size,
        vec![egui::Event::Text("会议".into())],
    );
    assert_eq!(client.desktop_ui.experience.palette_query, "会议");
    let key = |key| egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    workspace_test_frame(&mut client, &ctx, size, vec![key(egui::Key::Enter)]);
    assert_eq!(client.selected_note_id, "doc-b");
    assert!(!client.desktop_ui.experience.palette_open);
    client.desktop_ui.experience.focus_mode = true;
    client.desktop_ui.experience.inspector = Some(KnowledgeInspector::Outline);
    workspace_test_frame(&mut client, &ctx, size, vec![key(egui::Key::Escape)]);
    assert!(client.desktop_ui.experience.inspector.is_none());
    assert!(client.desktop_ui.experience.focus_mode);
    workspace_test_frame(&mut client, &ctx, size, vec![key(egui::Key::Escape)]);
    assert!(!client.desktop_ui.experience.focus_mode);
    assert_eq!(client.selected_note_id, "doc-b");
    assert!(client.desktop_ui.navigation.editor_visible);
    assert_eq!(client.state_json, original);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn knowledge_experience_record_search_keeps_query_state_and_excludes_private_rows() {
    let dir = temp_test_dir("knowledge_experience_record_search");
    let client = knowledge_test_client(&dir);
    let mut records = client.knowledge_records().as_ref().clone();
    let rows = vec!["doc-a".to_string(), "doc-b".to_string()];
    let original = rows.clone();
    assert_eq!(
        knowledge_filter_rows(&rows, &records, "会议"),
        vec!["doc-b"]
    );
    assert_eq!(rows, original);
    records
        .iter_mut()
        .find(|p| p.id == "doc-b")
        .unwrap()
        .encrypted = true;
    assert!(knowledge_filter_rows(&rows, &records, "会议").is_empty());
    assert_eq!(knowledge_filter_rows(&rows, &records, ""), original);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn knowledge_experience_insert_focus_respects_page_lock() {
    let dir = temp_test_dir("knowledge_experience_insert");
    let mut client = knowledge_test_client(&dir);
    assert!(client.insert_knowledge_block(knowledge::BlockKind::Paragraph, None, 0, None));
    assert_eq!(
        client.desktop_ui.parity.outline_target.as_deref(),
        Some(client.note_active_block_id.as_str())
    );
    assert!(client.save_note());
    let mut meta = client.desktop_ui.knowledge.page.clone().unwrap();
    meta.locked = true;
    assert!(client.update_knowledge_page(meta));
    let original = client.note_blocks_draft.clone();
    client.desktop_ui.parity.outline_target = None;
    assert!(!client.insert_knowledge_block(knowledge::BlockKind::Paragraph, None, 0, None));
    assert!(client.desktop_ui.parity.outline_target.is_none());
    assert_eq!(client.note_blocks_draft, original);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn knowledge_experience_open_view_options_does_not_dirty_or_resize_database() {
    let dir = temp_test_dir("knowledge_experience_view_options");
    let mut client = knowledge_test_client(&dir);
    assert!(client.create_knowledge_page(None, true));
    assert!(client.save_note());
    let original = client.state_json.clone();
    let database = client.desktop_ui.knowledge.page.clone().unwrap();
    let ctx = workspace_test_context();
    let size = egui::vec2(1240.0, 820.0);
    client.desktop_ui.knowledge.options_open = true;
    for _ in 0..4 {
        workspace_test_frame(&mut client, &ctx, size, vec![]);
    }
    assert_eq!(client.desktop_ui.knowledge.page.as_ref(), Some(&database));
    assert!(
        !client.note_dirty,
        "opening options must not create an edit"
    );
    assert_eq!(client.state_json, original);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}
