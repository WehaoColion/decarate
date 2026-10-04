// v1.0.3.10 Windows - Exercise pending input, validation and concurrent refresh boundaries.

fn audit_form_frame(
    client: &mut TimerWindowsClient,
    ctx: &egui::Context,
    database: &knowledge::KnowledgeDatabase,
    events: Vec<egui::Event>,
) -> (egui::Rect, bool) {
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 700.0),
            )),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                client.ui_knowledge_form(ui, database, &[], false);
            });
        },
    );
    ctx.data(|data| {
        data.get_temp::<(egui::Rect, bool)>(egui::Id::new("knowledge_form_submit"))
            .unwrap()
    })
}

#[test]
fn input_boundary_form_rejects_invalid_optional_draft_and_submits_corrected_value() {
    let root = temp_test_dir("audit_form_pending_input");
    let mut client = knowledge_test_client(&root);
    assert!(client.create_knowledge_page(None, true));
    let database = client
        .desktop_ui
        .knowledge
        .page
        .as_ref()
        .unwrap()
        .database
        .clone()
        .unwrap();
    let parent = client.selected_note_id.clone();
    client.desktop_ui.knowledge.form_title = "Pending property".into();
    let ctx = workspace_test_context();
    audit_form_frame(&mut client, &ctx, &database, vec![]);
    let key = format!("form:{parent}:date");
    let focus = egui::Id::new(("property_input", &key));
    ctx.memory_mut(|memory| memory.request_focus(focus));
    let (_, enabled) = audit_form_frame(
        &mut client,
        &ctx,
        &database,
        vec![egui::Event::Text("2026-02-30".into())],
    );
    assert_eq!(
        client.desktop_ui.knowledge.cell_inputs[&key].text,
        "2026-02-30"
    );
    assert!(
        !enabled,
        "an invalid optional input must block record creation"
    );
    assert!(!client.desktop_ui.knowledge.form_title.is_empty());

    client
        .desktop_ui
        .knowledge
        .cell_inputs
        .get_mut(&key)
        .unwrap()
        .text = "2026-09-26".into();
    let (rect, enabled) = audit_form_frame(&mut client, &ctx, &database, vec![]);
    assert!(
        enabled,
        "valid pending input should enable the submit button without requiring a blur"
    );
    let count = client.data.notes.len();
    for pressed in [true, false] {
        audit_form_frame(
            &mut client,
            &ctx,
            &database,
            vec![
                egui::Event::PointerMoved(rect.center()),
                egui::Event::PointerButton {
                    pos: rect.center(),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    assert_eq!(client.data.notes.len(), count + 1, "{}", client.status);
    let created = client
        .data
        .notes
        .iter()
        .find(|note| note.title == "Pending property")
        .unwrap();
    assert_eq!(
        created.document.knowledge.as_ref().unwrap().properties["date"],
        knowledge::CellValue::Date(knowledge::DateRange {
            start: "2026-09-26".into(),
            end: String::new()
        })
    );
    let reopened = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    assert!(
        reopened
            .notes
            .iter()
            .any(|note| note.id == created.id
                && note.document.knowledge == created.document.knowledge)
    );
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn input_boundary_remote_refresh_does_not_erase_cell_draft_after_focus_leaves() {
    let ctx = workspace_test_context();
    let field = knowledge::DatabaseField {
        id: "amount".into(),
        name: "金额".into(),
        kind: knowledge::FieldKind::Number,
        ..Default::default()
    };
    let mut inputs = BTreeMap::new();
    let id = egui::Id::new(("property_input", "audit_amount"));
    let render = |inputs: &mut BTreeMap<String, KnowledgeCellInput>, value: f64| {
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                knowledge_cell_editor(
                    ui,
                    "audit_amount",
                    &field,
                    &knowledge::CellValue::Number(value),
                    &[],
                    inputs,
                    &mut DesktopNavigationState::default(),
                );
            });
        });
    };
    render(&mut inputs, 10.0);
    inputs.get_mut("audit_amount").unwrap().text = "25.".into();
    ctx.memory_mut(|memory| memory.request_focus(id));
    render(&mut inputs, 30.0);
    ctx.memory_mut(|memory| memory.surrender_focus(id));
    render(&mut inputs, 30.0);
    assert_eq!(
        inputs["audit_amount"].text, "25.",
        "remote refresh erased an uncommitted local edit on blur"
    );
    render(&mut inputs, 25.0);
    assert_eq!(
        inputs["audit_amount"].text, "25",
        "a successfully committed value should adopt canonical formatting"
    );
    render(&mut inputs, 40.0);
    assert_eq!(
        inputs["audit_amount"].text, "40",
        "clean cells should keep receiving remote updates"
    );
}

#[test]
fn input_boundary_form_rejects_a_relation_to_a_removed_or_private_page() {
    let root = temp_test_dir("audit_form_stale_relation");
    let mut client = knowledge_test_client(&root);
    assert!(client.create_knowledge_page(None, true));
    let mut database = knowledge::KnowledgeDatabase::task_database();
    database.fields.push(knowledge::DatabaseField {
        id: "related".into(),
        name: "关联页面".into(),
        kind: knowledge::FieldKind::Relation,
        ..Default::default()
    });
    client.desktop_ui.knowledge.form_title = "Related record".into();
    client.desktop_ui.knowledge.form_values.insert(
        "related".into(),
        knowledge::CellValue::Relation(vec!["doc-a".into()]),
    );
    let ctx = workspace_test_context();
    assert!(audit_form_frame(&mut client, &ctx, &database, vec![]).1);
    client
        .data
        .notes
        .iter_mut()
        .find(|n| n.id == "doc-a")
        .unwrap()
        .deleted_at_epoch_millis = Some(1);
    assert!(
        !audit_form_frame(&mut client, &ctx, &database, vec![]).1,
        "a previously selected relation must be rechecked before creation"
    );
    client
        .desktop_ui
        .knowledge
        .form_values
        .insert("related".into(), knowledge::CellValue::Empty);
    assert!(audit_form_frame(&mut client, &ctx, &database, vec![]).1);
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn input_boundary_disabled_canvas_cannot_delete_selected_nodes_from_keyboard() {
    let root = temp_test_dir("audit_disabled_canvas");
    let mut client = knowledge_test_client(&root);
    assert!(client.ensure_knowledge_canvas_loaded());
    client.knowledge_canvas_add_note();
    assert_eq!(
        client
            .desktop_ui
            .knowledge_canvas
            .store
            .active()
            .unwrap()
            .nodes
            .len(),
        1
    );
    let ctx = workspace_test_context();
    for enabled in [false, true] {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 800.0),
                )),
                events: vec![egui::Event::Key {
                    key: egui::Key::Delete,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add_enabled_ui(enabled, |ui| client.ui_knowledge_canvas(ui));
                });
            },
        );
        assert_eq!(
            client
                .desktop_ui
                .knowledge_canvas
                .store
                .active()
                .unwrap()
                .nodes
                .len(),
            usize::from(!enabled),
            "disabled canvas must honor the modal editing barrier"
        );
    }
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn input_boundary_navigation_commits_valid_properties_and_keeps_invalid_drafts_open() {
    let root = temp_test_dir("audit_property_navigation");
    let mut client = knowledge_test_client(&root);
    assert!(client.create_knowledge_page(None, true));
    let parent = client.selected_note_id.clone();
    let row = json!({"id":"pending-row","kind":"DOCUMENT","title":"Pending row",
    "document":{"blocks":[],"knowledge":knowledge::KnowledgePage {
        parent_id:Some(parent.clone()), ..Default::default()
    }}});
    assert!(client.replace_state(
        app_data::upsert_note_app_data_json(&client.state_json, &row.to_string(), now_millis()),
        "fixture"
    ));
    let key = "table:pending-row:date".to_string();
    client.desktop_ui.knowledge.cell_inputs.insert(
        key.clone(),
        KnowledgeCellInput {
            source: String::new(),
            text: "2026-02-30".into(),
        },
    );
    let before = client.state_json.clone();
    client.select_note_by_id("doc-a");
    assert_eq!(
        client.selected_note_id, parent,
        "navigation must not discard invalid property text"
    );
    assert_eq!(client.state_json, before);
    assert_eq!(
        client.desktop_ui.knowledge.cell_inputs[&key].text,
        "2026-02-30"
    );

    client
        .desktop_ui
        .knowledge
        .cell_inputs
        .get_mut(&key)
        .unwrap()
        .text = "2026-09-26".into();
    client.workspace_persistence_ready = false;
    client.select_note_by_id("doc-a");
    assert_eq!(client.selected_note_id, parent);
    assert_eq!(
        client.desktop_ui.knowledge.cell_inputs[&key].text,
        "2026-09-26"
    );
    client.workspace_persistence_ready = true;
    client.set_note_canvas_text("Database body edited before navigation");
    client.select_note_by_id("doc-a");
    assert_eq!(client.selected_note_id, "doc-a", "{}", client.status);
    let saved = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    assert!(saved.notes.iter().any(|note| note.id == parent
        && note_body_text(note).contains("Database body edited before navigation")));
    let row = saved
        .notes
        .iter()
        .find(|note| note.id == "pending-row")
        .unwrap();
    assert_eq!(
        row.document.knowledge.as_ref().unwrap().properties["date"].text(),
        "2026-09-26"
    );
    client.select_note_by_id("pending-row");
    client.desktop_ui.knowledge.cell_inputs.insert(
        "page:pending-row:date".into(),
        KnowledgeCellInput {
            source: "2026-09-26".into(),
            text: "2026-09-27".into(),
        },
    );
    assert!(client.return_to_document_list());
    client.select_note_by_id("pending-row");
    client.set_note_canvas_text("Body changed after returning to the same record");
    client.flush_note_draft().unwrap();
    let saved = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    let row = saved
        .notes
        .iter()
        .find(|note| note.id == "pending-row")
        .unwrap();
    assert_eq!(
        row.document.knowledge.as_ref().unwrap().properties["date"].text(),
        "2026-09-27",
        "reopening the same record must not restore stale property metadata"
    );
    drop(client);
    fs::remove_dir_all(root).unwrap();
}
