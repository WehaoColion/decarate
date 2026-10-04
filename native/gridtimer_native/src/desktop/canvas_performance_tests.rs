// Isolated redraw benchmark for a large canvas. Run explicitly with --ignored --nocapture.

#[test]
fn knowledge_canvas_note_editor_keeps_unicode_limit_and_stored_input() {
    let root = temp_test_dir("knowledge_canvas_note_editor_limit");
    let mut client = knowledge_test_client(&root);
    assert!(client
        .desktop_ui
        .knowledge_canvas
        .store
        .new_canvas("输入边界"));
    client
        .desktop_ui
        .knowledge_canvas
        .store
        .active_mut()
        .unwrap()
        .nodes
        .push(KnowledgeCanvasNode {
            id: "unicode-note".into(),
            kind: KnowledgeCanvasNodeKind::Note,
            ..Default::default()
        });
    let context = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1_200.0, 800.0));
    let render = |client: &mut TimerWindowsClient, events: Vec<egui::Event>| {
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    client.draw_knowledge_canvas_nodes(ui, screen, 0);
                });
            },
        );
    };
    render(&mut client, vec![]);
    context.memory_mut(|memory| {
        memory.request_focus(egui::Id::new(("knowledge_canvas_note", "unicode-note")))
    });
    render(&mut client, vec![egui::Event::Text("汉".repeat(8_100))]);
    let node = &client
        .desktop_ui
        .knowledge_canvas
        .store
        .active()
        .unwrap()
        .nodes[0];
    assert_eq!(node.text.chars().count(), 8_000);
    assert!(node.text.chars().all(|character| character == '汉'));
    assert!(client.desktop_ui.knowledge_canvas.dirty);
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "Explicit synthetic canvas frame benchmark"]
fn knowledge_canvas_large_frame_benchmark() {
    let root = temp_test_dir("knowledge_canvas_large_frame_benchmark");
    let mut client = knowledge_test_client(&root);
    let store = &mut client.desktop_ui.knowledge_canvas.store;
    assert!(store.new_canvas("性能基准"));
    let canvas = store.active_mut().unwrap();
    let body = "画布便签内容。".repeat(1_000);
    canvas.nodes = (0..1_000)
        .map(|index| KnowledgeCanvasNode {
            id: format!("benchmark-note-{index}"),
            kind: KnowledgeCanvasNodeKind::Note,
            text: body.clone(),
            x: (index % 40) as f32 * 300.0,
            y: (index / 40) as f32 * 220.0,
            ..Default::default()
        })
        .collect();
    let context = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1_200.0, 800.0));
    let result = measure("canvas_1000_notes_redraw", 30, || {
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    client.draw_knowledge_canvas_nodes(ui, screen, 0);
                });
            },
        );
    });
    println!("{result}");
    drop(client);
    fs::remove_dir_all(root).unwrap();
}
