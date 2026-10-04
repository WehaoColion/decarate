// v1.0.3.16 Windows - Verify deep knowledge home rows remain clickable after viewport culling.
// v1.0.2.1 Windows - Preserve collapsed, orphaned and cyclic page navigation when virtualized.
// v2.22.52 - Assert durable workspace behavior, independent of labels and styling.

#[test]
#[ignore = "Explicit isolated navigation benchmark; requires a selected evidence file"]
fn knowledge_navigation_viewport_benchmark() {
    // Preserve the previous all-row traversal for a same-build rendering
    // comparison. Both paths use the production row renderer, but this baseline
    // visits/layouts every row and rebuilds its cycle/collapse set each frame.
    fn legacy_item(
        client: &mut TimerWindowsClient,
        ui: &mut egui::Ui,
        all: &KnowledgeNavigation,
        index: usize,
        depth: usize,
        seen: &mut HashSet<String>,
    ) {
        let page = &all.pages[index];
        if depth > 32 || !seen.insert(page.id.clone()) {
            return;
        }
        client.ui_knowledge_tree_item(ui, all, page, depth);
        if client.desktop_ui.knowledge.collapsed.contains(&page.id) {
            let mut hidden = vec![page.id.clone()];
            while let Some(parent) = hidden.pop() {
                for &child in all.children_of(Some(&parent)) {
                    if seen.insert(all.pages[child].id.clone()) {
                        hidden.push(all.pages[child].id.clone());
                    }
                }
            }
        } else {
            for &child in all.children_of(Some(&page.id)) {
                legacy_item(client, ui, all, child, depth + 1, seen);
            }
        }
    }
    fn legacy_tree(client: &mut TimerWindowsClient, ui: &mut egui::Ui) {
        let all = client.knowledge_navigation();
        egui::ScrollArea::vertical()
            .id_source("knowledge_page_tree")
            .max_height(560.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.set_height(32.0);
                    ui.label(egui::RichText::new("页面").small().color(palette().muted));
                    if ui
                        .small_button("＋")
                        .on_hover_text("新建顶层页面")
                        .clicked()
                    {
                        client.create_knowledge_page(None, false);
                    }
                    if ui.small_button("＋ 数据库").clicked() {
                        client.create_knowledge_page(None, true);
                    }
                });
                let mut seen = HashSet::new();
                for &index in all.children_of(None) {
                    legacy_item(client, ui, &all, index, 0, &mut seen);
                }
                for index in 0..all.pages.len() {
                    if !seen.contains(&all.pages[index].id) {
                        legacy_item(client, ui, &all, index, 0, &mut seen);
                    }
                }
            });
    }
    fn frame(client: &mut TimerWindowsClient, ctx: &egui::Context, legacy: bool, time: f64) {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(320.0, 620.0),
                )),
                time: Some(time),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    if legacy {
                        legacy_tree(client, ui);
                    } else {
                        client.ui_knowledge_tree(ui, 560.0);
                    }
                });
            },
        );
    }
    fn summary(mut samples: Vec<u64>) -> Value {
        samples.sort_unstable();
        json!({"iterations":samples.len(), "medianMicros":samples[samples.len()/2] as f64 / 1000.0,
            "p95Micros":samples[samples.len()*95/100] as f64 / 1000.0})
    }
    let output = PathBuf::from(
        std::env::var_os("DESKTOP_NAVIGATION_PERFORMANCE_OUTPUT").expect("output path"),
    );
    let dir = temp_test_dir("knowledge_navigation_viewport_benchmark");
    let mut client = knowledge_test_client(&dir);
    client.data.notes = (0..512)
        .map(|index| DesktopNote {
            id: format!("page-{index:04}"),
            title: format!("Page {index:04}"),
            kind: "DOCUMENT".into(),
            document: DesktopNoteDocument {
                knowledge: Some(knowledge::KnowledgePage {
                    parent_id: (index % 8 != 0).then(|| format!("page-{:04}", index / 8 * 8)),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        })
        .collect();
    client.note_search_draft.clear();
    client.knowledge_quick_filter = KnowledgeQuickFilter::All;
    client.data.note_preferences.selected_folder_id = None;
    client.desktop_ui.knowledge.collapsed.clear();
    client.data_version += 1;
    let navigation = client.knowledge_navigation();
    assert_eq!(navigation.pages.len(), 512);
    assert_eq!(navigation.roots.len(), 64);
    assert!(navigation.pages.iter().all(|page| !page.pinned));
    let cold_start = Instant::now();
    assert_eq!(client.knowledge_tree_rows(&navigation).len(), 513);
    let cold_flatten_micros = cold_start.elapsed().as_nanos() as f64 / 1000.0;
    let legacy_ctx = workspace_test_context();
    let viewport_ctx = workspace_test_context();
    for warmup in 0..6 {
        frame(&mut client, &legacy_ctx, true, warmup as f64 * 0.02);
        frame(&mut client, &viewport_ctx, false, warmup as f64 * 0.02);
    }
    let mut legacy_times = Vec::with_capacity(64);
    let mut viewport_times = Vec::with_capacity(64);
    for iteration in 0..64 {
        // Alternate ordering to avoid assigning a systematic warmup/load bias
        // to one mode; both contexts render the same stationary viewport.
        for legacy in [iteration % 2 == 0, iteration % 2 != 0] {
            let ctx = if legacy { &legacy_ctx } else { &viewport_ctx };
            let start = Instant::now();
            frame(&mut client, ctx, legacy, 1.0 + iteration as f64 * 0.02);
            let elapsed = start.elapsed().as_nanos() as u64;
            if legacy {
                legacy_times.push(elapsed);
            } else {
                viewport_times.push(elapsed);
            }
        }
    }
    let report = json!({
        "version":WINDOWS_CLIENT_VERSION,
        "profile":"same debug-test executable, warmed contexts, alternating measurement order",
        "scope":"Knowledge sidebar egui CPU layout/render preparation only; excludes GPU presentation, startup, search and filtered lists. Legacy baseline preserves the old full-tree traversal using the current row renderer.",
        "pages":512, "rootPages":64, "viewport":[320,620], "scrollHeight":560,
        "coldFlatRowsMicros":cold_flatten_micros,
        "legacyAllRows":summary(legacy_times), "cachedViewportRows":summary(viewport_times),
    });
    fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!("{report}");
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn knowledge_navigation_collapsed_subtrees_stay_hidden_and_orphans_remain_reachable() {
    let make_page = |id: &str, parent: Option<&str>, pinned| DesktopNote {
        id: id.into(),
        title: id.into(),
        kind: "DOCUMENT".into(),
        pinned,
        document: DesktopNoteDocument {
            knowledge: Some(knowledge::KnowledgePage {
                parent_id: parent.map(str::to_owned),
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    let navigation = KnowledgeNavigation::from_notes(&[
        make_page("root", None, false),
        make_page("child", Some("root"), true),
        make_page("grandchild", Some("child"), false),
        make_page("orphan", Some("deleted-parent"), false),
        make_page("cycle-a", Some("cycle-b"), false),
        make_page("cycle-b", Some("cycle-a"), false),
    ]);
    let collapsed = HashSet::from(["root".to_owned()]);
    let rows = navigation.tree_rows(&collapsed);
    let page_ids = |rows: &[KnowledgeTreeRow]| {
        rows.iter()
            .filter_map(|row| match row {
                KnowledgeTreeRow::Page { index, .. } => Some(navigation.pages[*index].id.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(page_ids(&rows), ["root", "orphan", "cycle-a", "cycle-b"]);
    assert!(rows.iter().any(|row| matches!(row, KnowledgeTreeRow::Pinned(index) if navigation.pages[*index].id == "child")));
    let expanded = navigation.tree_rows(&HashSet::new());
    assert_eq!(
        page_ids(&expanded),
        [
            "root",
            "child",
            "grandchild",
            "orphan",
            "cycle-a",
            "cycle-b"
        ]
    );
    let cycle_collapsed = navigation.tree_rows(&HashSet::from(["cycle-a".to_owned()]));
    assert!(!page_ids(&cycle_collapsed).contains(&"cycle-b".to_owned()));
}

#[test]
fn knowledge_navigation_scrolled_pages_stay_clickable() {
    let dir = temp_test_dir("knowledge_navigation_scrolled_click");
    let mut client = knowledge_test_client(&dir);
    let seed = client
        .data
        .notes
        .iter()
        .find(|note| note.id == "doc-a")
        .unwrap()
        .clone();
    client.data.notes = (0..512)
        .map(|index| DesktopNote {
            id: format!("page-{index:04}"),
            title: format!("Page {index:04}"),
            pinned: false,
            folder_id: None,
            document: DesktopNoteDocument::default(),
            ..seed.clone()
        })
        .collect();
    client.data.note_preferences.selected_folder_id = None;
    client.data_version += 1;
    let ctx = workspace_test_context();
    let render = |client: &mut TimerWindowsClient, scroll_to_row: Option<usize>, events| {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(320.0, 220.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    if let Some(row) = scroll_to_row {
                        let id = ui.make_persistent_id(egui::Id::new("knowledge_page_tree"));
                        let mut state = egui::scroll_area::State::load(ctx, id).unwrap_or_default();
                        state.offset.y = row as f32 * (32.0 + ui.spacing().item_spacing.y);
                        state.store(ctx, id);
                    }
                    client.ui_knowledge_tree(ui, 180.0);
                });
            },
        );
    };
    render(&mut client, None, vec![]);
    render(&mut client, None, vec![]);
    assert!(
        ctx.data(
            |data| data.get_temp::<egui::Rect>(egui::Id::new(("knowledge_tree_row", "page-0000")))
        )
        .is_some()
    );
    assert!(
        ctx.data(
            |data| data.get_temp::<egui::Rect>(egui::Id::new(("knowledge_tree_row", "page-0350")))
        )
        .is_none()
    );
    render(&mut client, Some(351), vec![]);
    render(&mut client, None, vec![]);
    let rect = ctx
        .data(|data| {
            data.get_temp::<egui::Rect>(egui::Id::new(("knowledge_tree_row", "page-0350")))
        })
        .expect("scrolled page must be rendered");
    let pos = rect.center();
    render(
        &mut client,
        None,
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
    render(
        &mut client,
        None,
        vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert_eq!(client.selected_note_id, "page-0350");
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn knowledge_home_scrolled_rows_stay_clickable_without_building_every_row() {
    let dir = temp_test_dir("knowledge_home_scrolled_click");
    let mut client = knowledge_test_client(&dir);
    let seed = client
        .data
        .notes
        .iter()
        .find(|note| note.id == "doc-a")
        .unwrap()
        .clone();
    client.data.notes = (0..512)
        .map(|index| DesktopNote {
            id: format!("page-{index:04}"),
            title: format!("Page {index:04}"),
            pinned: false,
            folder_id: None,
            document: DesktopNoteDocument::default(),
            ..seed.clone()
        })
        .collect();
    client.data.note_preferences.selected_folder_id = None;
    client.desktop_ui.experience.trail.clear();
    client.data_version += 1;
    client.rebuild_view_cache();
    let ctx = workspace_test_context();
    let render = |client: &mut TimerWindowsClient, scroll: Option<f32>, events| {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600.0, 300.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    if let Some(offset) = scroll {
                        let id = ui.make_persistent_id(egui::Id::new("knowledge_home_scroll"));
                        let mut state = egui::scroll_area::State::load(ctx, id).unwrap_or_default();
                        state.offset.y = offset;
                        state.store(ctx, id);
                    }
                    egui::ScrollArea::vertical()
                        .id_source("knowledge_home_scroll")
                        .show(ui, |ui| client.ui_knowledge_library(ui));
                });
            },
        );
    };
    render(&mut client, None, vec![]);
    render(&mut client, None, vec![]);
    assert!(
        ctx.data(
            |data| data.get_temp::<egui::Rect>(egui::Id::new(("knowledge_home_row", "page-0000")))
        )
        .is_some()
    );
    assert!(
        ctx.data(
            |data| data.get_temp::<egui::Rect>(egui::Id::new(("knowledge_home_row", "page-0350")))
        )
        .is_none()
    );
    render(&mut client, Some(350.0 * 62.0 + 220.0), vec![]);
    render(&mut client, None, vec![]);
    let visible = (340..360).find_map(|index| {
        let id = format!("page-{index:04}");
        ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::new(("knowledge_home_row", &id))))
            .filter(|rect| ctx.screen_rect().contains(rect.center()))
            .map(|rect| (id, rect))
    });
    let (id, rect) = visible.expect("a deep list row should be rendered after scrolling");
    let pos = rect.center();
    render(
        &mut client,
        None,
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
    render(
        &mut client,
        None,
        vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert_eq!(client.selected_note_id, id);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn knowledge_navigation_rows_refresh_after_collapse_and_data_changes() {
    let dir = temp_test_dir("knowledge_navigation_rows");
    let mut client = knowledge_test_client(&dir);
    let child = client
        .data
        .notes
        .iter_mut()
        .find(|note| note.id == "doc-b")
        .unwrap();
    child
        .document
        .knowledge
        .get_or_insert_with(Default::default)
        .parent_id = Some("doc-a".into());
    client.data_version += 1;
    let navigation = client.knowledge_navigation();
    let before = client.knowledge_tree_rows(&navigation);
    assert!(Arc::ptr_eq(
        &before,
        &client.knowledge_tree_rows(&navigation)
    ));
    assert_eq!(before.len(), 3);
    client.desktop_ui.knowledge.collapsed.insert("doc-a".into());
    let collapsed = client.knowledge_tree_rows(&navigation);
    assert!(!Arc::ptr_eq(&before, &collapsed));
    assert_eq!(collapsed.len(), 2);
    client.desktop_ui.knowledge.collapsed.clear();
    let expanded = client.knowledge_tree_rows(&navigation);
    assert_eq!(*before, *expanded);
    client
        .data
        .notes
        .iter_mut()
        .find(|note| note.id == "doc-b")
        .unwrap()
        .deleted_at_epoch_millis = Some(now_millis());
    client.data_version += 1;
    let refreshed = client.knowledge_navigation();
    assert_eq!(client.knowledge_tree_rows(&refreshed).len(), 2);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn knowledge_workspace_block_subtree_undo_save_reopen_and_lock() {
    let dir = temp_test_dir("knowledge_workspace_blocks");
    let mut client = knowledge_test_client(&dir);
    client.insert_knowledge_block(knowledge::BlockKind::Columns, None, 0, None);
    let columns = client.note_active_block_id.clone();
    client.insert_knowledge_block(knowledge::BlockKind::Todo, Some(columns.clone()), 1, None);
    let todo = client.note_active_block_id.clone();
    let mut edited = client
        .note_blocks_draft
        .iter()
        .find(|b| b.id == todo)
        .unwrap()
        .clone();
    edited.text = "关键子内容".into();
    edited.knowledge.as_mut().unwrap().checked = true;
    client.update_knowledge_block(edited, false);
    assert!(!client.move_knowledge_block(&columns, &todo, false));
    assert!(client.save_note(), "{}", client.status);
    let snapshot = client.current_note_draft_snapshot();
    client.duplicate_knowledge_block(&columns);
    assert_eq!(
        client
            .note_blocks_draft
            .iter()
            .filter(|b| b.text == "关键子内容")
            .count(),
        2
    );
    client.undo_note_draft();
    assert_eq!(client.current_note_draft_snapshot(), snapshot);
    client.delete_knowledge_block(&columns);
    assert!(!client.note_blocks_draft.iter().any(|b| b.id == todo));
    client.undo_note_draft();
    assert_eq!(client.current_note_draft_snapshot(), snapshot);
    assert!(client.save_note());
    client.clear_note_draft_without_flush();
    client.select_note_by_id("doc-a");
    assert!(client.note_blocks_draft.iter().any(|b| b.id == todo
        && b.knowledge
            .as_ref()
            .is_some_and(|m| m.checked && m.parent_id.as_deref() == Some(&columns))));
    let mut meta = client.desktop_ui.knowledge.page.clone().unwrap();
    meta.locked = true;
    assert!(client.update_knowledge_page(meta));
    assert!(client.save_note(), "{}", client.status);
    let locked = client.note_blocks_draft.clone();
    assert!(!client.insert_knowledge_block(knowledge::BlockKind::Paragraph, None, 0, None));
    client.delete_knowledge_block(&columns);
    assert_eq!(client.note_blocks_draft, locked);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn knowledge_workspace_database_properties_are_shared_by_all_views_and_survive_sync() {
    let dir = temp_test_dir("knowledge_workspace_database");
    let mut client = knowledge_test_client(&dir);
    assert!(
        client.create_knowledge_page(None, true),
        "{}",
        client.status
    );
    let db_id = client.selected_note_id.clone();
    let record = json!({"id":"row-a","kind":"DOCUMENT","title":"任务","document":{"knowledge":knowledge::KnowledgePage{parent_id:Some(db_id.clone()),properties:BTreeMap::from([("status".into(),knowledge::CellValue::Select("未开始".into()))]),..Default::default()},"blocks":[]}});
    assert!(client.replace_state(
        app_data::upsert_note_app_data_json(&client.state_json, &record.to_string(), now_millis()),
        "added"
    ));
    assert!(client.set_knowledge_cell(
        "row-a",
        "status",
        knowledge::CellValue::Select("进行中".into())
    ));
    assert!(!client.set_knowledge_cell(
        "row-a",
        "status",
        knowledge::CellValue::Select("invalid".into())
    ));
    let records = client.knowledge_records();
    let database = records
        .iter()
        .find(|p| p.id == db_id)
        .unwrap()
        .meta
        .database
        .clone()
        .unwrap();
    let mut engine = knowledge::QueryEngine::new(&records, knowledge_today());
    for view in &database.views {
        assert_eq!(engine.rows(&db_id, view), vec!["row-a"]);
    }
    assert_eq!(
        engine.value("row-a", "status").unwrap(),
        knowledge::CellValue::Select("进行中".into())
    );
    let remote = app_data::update_slot_title_app_data_json(
        &client.state_json,
        2,
        "另一端修改计时标题",
        now_millis(),
    )
    .unwrap();
    audit_apply_remote(&mut client, remote);
    let after = client.knowledge_records();
    assert_eq!(
        after
            .iter()
            .find(|p| p.id == "row-a")
            .unwrap()
            .meta
            .properties,
        records
            .iter()
            .find(|p| p.id == "row-a")
            .unwrap()
            .meta
            .properties
    );
    assert!(!client.move_knowledge_page(&db_id, Some("row-a"), 0));
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn knowledge_workspace_form_required_fields_disable_actual_submit() {
    let dir = temp_test_dir("knowledge_workspace_form");
    let mut client = knowledge_test_client(&dir);
    assert!(client.create_knowledge_page(None, true));
    let ctx = workspace_test_context();
    let size = egui::vec2(1300.0, 900.0);
    let meta = client.desktop_ui.knowledge.page.as_mut().unwrap();
    let db = meta.database.as_mut().unwrap();
    db.fields[0].required = true;
    client.desktop_ui.knowledge.active_view = db
        .views
        .iter()
        .find(|v| v.kind == knowledge::ViewKind::Form)
        .unwrap()
        .id
        .clone();
    for _ in 0..3 {
        workspace_test_frame(&mut client, &ctx, size, vec![]);
    }
    assert!(
        !ctx.data(|d| d.get_temp::<(egui::Rect, bool)>(egui::Id::new("knowledge_form_submit")))
            .unwrap()
            .1
    );
    client.desktop_ui.knowledge.form_title = "实际表单提交".into();
    client.desktop_ui.knowledge.form_values.insert(
        "status".into(),
        knowledge::CellValue::Select("未开始".into()),
    );
    workspace_test_frame(&mut client, &ctx, size, vec![]);
    let (rect, enabled) = ctx
        .data(|d| d.get_temp::<(egui::Rect, bool)>(egui::Id::new("knowledge_form_submit")))
        .unwrap();
    assert!(enabled);
    let count = client.data.notes.len();
    workspace_click(&mut client, &ctx, size, rect.center());
    assert_eq!(client.data.notes.len(), count + 1, "{}", client.status);
    assert!(client.data.notes.iter().any(|p| p.title == "实际表单提交"
        && p.document
            .knowledge
            .as_ref()
            .is_some_and(|m| m.parent_id.as_deref() == Some(&client.selected_note_id))));
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn knowledge_workspace_metadata_rejected_save_keeps_original_and_retry_draft() {
    let dir = temp_test_dir("knowledge_workspace_save_failure");
    let mut client = knowledge_test_client(&dir);
    let original = client.state_json.clone();
    let mut meta = knowledge::KnowledgePage::default();
    meta.parent_id = Some("missing-parent".into());
    assert!(client.update_knowledge_page(meta));
    assert!(!client.save_note());
    assert_eq!(client.state_json, original);
    assert!(client.note_dirty);
    assert_eq!(
        client
            .desktop_ui
            .knowledge
            .page
            .as_ref()
            .unwrap()
            .parent_id
            .as_deref(),
        Some("missing-parent")
    );
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn knowledge_todo_filter_search_and_column_resize_keep_saved_meaning() {
    let dir = temp_test_dir("knowledge_projection");
    let mut client = knowledge_test_client(&dir);
    assert!(client.insert_knowledge_block(knowledge::BlockKind::Columns, None, 0, None));
    let parent = client.note_active_block_id.clone();
    let mut columns = client
        .note_blocks_draft
        .iter()
        .find(|b| b.id == parent)
        .unwrap()
        .clone();
    columns.knowledge.as_mut().unwrap().columns = 4;
    client.update_knowledge_block(columns.clone(), false);
    assert!(client.insert_knowledge_block(
        knowledge::BlockKind::Todo,
        Some(parent.clone()),
        3,
        None
    ));
    let todo = client.note_active_block_id.clone();
    let mut block = client
        .note_blocks_draft
        .iter()
        .find(|b| b.id == todo)
        .unwrap()
        .clone();
    block.text = "必须完成的任务".into();
    client.update_knowledge_block(block.clone(), true);
    columns.knowledge.as_mut().unwrap().columns = 2;
    client.update_knowledge_block(columns, false);
    assert_eq!(
        client
            .note_blocks_draft
            .iter()
            .find(|b| b.id == todo)
            .unwrap()
            .knowledge
            .as_ref()
            .unwrap()
            .column,
        1
    );
    assert!(client.save_note(), "{}", client.status);
    let note = client.selected_note().unwrap();
    assert!(knowledge_filter_matches(&note, KnowledgeQuickFilter::Todo));
    assert!(note_search_text(&note, &[]).contains("必须完成的任务"));
    let mut block = client
        .note_blocks_draft
        .iter()
        .find(|b| b.id == todo)
        .unwrap()
        .clone();
    block.knowledge.as_mut().unwrap().checked = true;
    client.update_knowledge_block(block, false);
    assert!(client.save_note(), "{}", client.status);
    assert!(!knowledge_filter_matches(
        &client.selected_note().unwrap(),
        KnowledgeQuickFilter::Todo
    ));
    assert_eq!(knowledge::block_plaintext(None, " / "), " / ");
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn knowledge_cell_refreshes_remote_value_without_erasing_focused_draft() {
    let ctx = workspace_test_context();
    let mut inputs = BTreeMap::new();
    let field = knowledge::DatabaseField {
        id: "cost".into(),
        name: "成本".into(),
        kind: knowledge::FieldKind::Number,
        ..Default::default()
    };
    let id = egui::Id::new(("property_input", "test_cost"));
    let mut draw = |value: f64, focus: bool| {
        if focus {
            ctx.memory_mut(|m| m.request_focus(id));
        } else {
            ctx.memory_mut(|m| m.surrender_focus(id));
        }
        ctx.begin_frame(egui::RawInput::default());
        egui::CentralPanel::default().show(&ctx, |ui| {
            knowledge_cell_editor(
                ui,
                "test_cost",
                &field,
                &knowledge::CellValue::Number(value),
                &[],
                &mut inputs,
                &mut DesktopNavigationState::default(),
            );
        });
        let _ = ctx.end_frame();
    };
    draw(10.0, false);
    draw(20.0, false);
    drop(draw);
    assert_eq!(inputs["test_cost"].text, "20");
    inputs.get_mut("test_cost").unwrap().text = "25.".into();
    ctx.memory_mut(|m| m.request_focus(id));
    ctx.begin_frame(egui::RawInput::default());
    egui::CentralPanel::default().show(&ctx, |ui| {
        knowledge_cell_editor(
            ui,
            "test_cost",
            &field,
            &knowledge::CellValue::Number(30.0),
            &[],
            &mut inputs,
            &mut DesktopNavigationState::default(),
        );
    });
    let _ = ctx.end_frame();
    assert_eq!(inputs["test_cost"].text, "25.");
}

#[test]
fn knowledge_encrypted_named_version_restores_structure_without_touching_live_timer() {
    let dir = temp_test_dir("knowledge_named_crypto");
    let mut client = knowledge_test_client(&dir);
    client.insert_knowledge_block(knowledge::BlockKind::Table, None, 0, None);
    let table = client.note_active_block_id.clone();
    let mut block = client
        .note_blocks_draft
        .iter()
        .find(|b| b.id == table)
        .unwrap()
        .clone();
    block.knowledge.as_mut().unwrap().table[1][0] = "私有表格原值".into();
    client.update_knowledge_block(block, false);
    let mut meta = client.desktop_ui.knowledge.page.clone().unwrap();
    meta.tags = vec!["私有标签".into()];
    client.update_knowledge_page(meta);
    assert!(client.save_note());
    let original = client.selected_note().unwrap().document;
    client.note_crypto_password_draft = "synthetic-knowledge-structured".into();
    client.enable_note_encryption();
    assert!(client.selected_note().unwrap().encryption.is_some());
    client.desktop_ui.knowledge.version_name = "修改前".into();
    assert!(client.create_named_knowledge_version(), "{}", client.status);
    let revision = client
        .selected_note()
        .unwrap()
        .revisions
        .iter()
        .find(|r| r.label == "修改前")
        .unwrap()
        .id
        .clone();
    let mut block = client
        .note_blocks_draft
        .iter()
        .find(|b| b.id == table)
        .unwrap()
        .clone();
    block.knowledge.as_mut().unwrap().table[1][0] = "修改后的值".into();
    client.update_knowledge_block(block, false);
    assert!(client.save_note());
    assert!(client.replace_state(
        app_data::start_slot_app_data_json(&client.state_json, 1, now_millis()),
        "start synthetic timer"
    ));
    let before: Value = serde_json::from_str(&client.state_json).unwrap();
    client.restore_note_revision("doc-a", &revision);
    assert_eq!(client.note_blocks_draft, original.blocks);
    let after: Value = serde_json::from_str(&client.state_json).unwrap();
    assert_eq!(before["slots"], after["slots"]);
    assert_eq!(before["financeProfile"], after["financeProfile"]);
    client.lock_note_encryption();
    assert!(client.selected_note_is_locked());
    assert!(!client.state_json.contains("私有表格原值"));
    assert!(!client.state_json.contains("私有标签"));
    client.note_search_draft = "私有标签".into();
    client.rebuild_view_cache();
    assert!(client.view_cache.notes.is_empty());
    client.note_crypto_password_draft = "synthetic-knowledge-structured".into();
    client.unlock_note_encryption();
    assert_eq!(client.note_blocks_draft, original.blocks);
    assert_eq!(client.desktop_ui.knowledge.page, original.knowledge);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}
