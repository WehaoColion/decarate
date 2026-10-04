// Windows canvas management: preserve board identity and content across edits.

#[test]
fn windows_canvas_new_boards_have_distinct_names_and_identity() {
    let mut store = KnowledgeCanvasStore::default();
    assert!(store.new_canvas("知识画布"));
    assert!(store.new_canvas("新画布"));
    assert!(store.new_canvas("新画布"));

    assert_eq!(store.canvases.len(), 3);
    assert_eq!(store.active_canvas_id, store.canvases[2].id);
    let names = store
        .canvases
        .iter()
        .map(|canvas| canvas.title.as_str())
        .collect::<HashSet<_>>();
    let ids = store
        .canvases
        .iter()
        .map(|canvas| canvas.id.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(
        names.len(),
        3,
        "the picker must distinguish newly created boards"
    );
    assert_eq!(ids.len(), 3);
    assert!(store.validate().is_ok());

    while store.canvases.len() < 128 {
        assert!(store.new_canvas("新画布"));
    }
    let before = store.clone();
    assert!(!store.new_canvas("新画布"));
    assert_eq!(
        store, before,
        "a rejected creation must leave the store intact"
    );
}

#[test]
fn windows_canvas_duplicate_rekeys_nodes_and_edges_without_losing_content() {
    let mut store = KnowledgeCanvasStore::default();
    assert!(store.new_canvas("研究"));
    let original_id = store.active_canvas_id.clone();
    let original = store.active_mut().unwrap();
    original.nodes = vec![
        KnowledgeCanvasNode {
            id: "source-a".into(),
            kind: KnowledgeCanvasNodeKind::Note,
            text: "第一条想法".into(),
            x: 32.0,
            y: 48.0,
            color: "amber".into(),
            ..Default::default()
        },
        KnowledgeCanvasNode {
            id: "source-b".into(),
            kind: KnowledgeCanvasNodeKind::Page,
            page_id: "document-1".into(),
            x: 240.0,
            y: 120.0,
            ..Default::default()
        },
    ];
    original.edges = vec![KnowledgeCanvasEdge {
        id: "edge-a".into(),
        from: "source-a".into(),
        to: "source-b".into(),
    }];

    assert!(store.duplicate_active());
    assert_eq!(store.canvases.len(), 2);
    assert_ne!(store.active_canvas_id, original_id);
    let source = &store.canvases[0];
    let copy = store.active().unwrap();
    assert_ne!(source.title, copy.title);
    assert_eq!(source.nodes.len(), copy.nodes.len());
    assert_eq!(source.edges.len(), copy.edges.len());
    assert_eq!(copy.nodes[0].text, source.nodes[0].text);
    assert_eq!(copy.nodes[0].color, source.nodes[0].color);
    assert_eq!(copy.nodes[1].page_id, source.nodes[1].page_id);
    assert_eq!((copy.nodes[0].x, copy.nodes[1].y), (32.0, 120.0));
    assert_ne!(copy.nodes[0].id, source.nodes[0].id);
    assert_ne!(copy.nodes[1].id, source.nodes[1].id);
    assert_ne!(copy.edges[0].id, source.edges[0].id);
    assert_eq!(copy.edges[0].from, copy.nodes[0].id);
    assert_eq!(copy.edges[0].to, copy.nodes[1].id);
    assert!(store.validate().is_ok());
}

#[test]
fn windows_canvas_delete_active_chooses_survivor_and_keeps_last_board() {
    let mut store = KnowledgeCanvasStore::default();
    for title in ["一", "二", "三"] {
        assert!(store.new_canvas(title));
    }
    let middle_id = store.canvases[1].id.clone();
    let final_id = store.canvases[2].id.clone();
    store.active_canvas_id = middle_id.clone();
    assert!(store.delete_canvas(&middle_id));
    assert_eq!(store.canvases.len(), 2);
    assert!(
        store.active().is_some(),
        "deleting the active board must select a survivor"
    );
    assert_ne!(store.active_canvas_id, middle_id);
    assert!(store.canvases.iter().any(|canvas| canvas.id == final_id));
    assert!(store.validate().is_ok());

    let active_id = store.active_canvas_id.clone();
    assert!(store.delete_canvas(&active_id));
    assert_eq!(store.canvases.len(), 1);
    let last_id = store.active_canvas_id.clone();
    assert!(!store.delete_canvas(&last_id));
    assert_eq!(store.canvases.len(), 1);
    assert_eq!(store.active_canvas_id, last_id);
    assert!(store.validate().is_ok());
}

#[test]
fn windows_canvas_store_json_roundtrip_keeps_active_board_and_graph() {
    let mut store = KnowledgeCanvasStore::default();
    assert!(store.new_canvas("项目"));
    store.active_mut().unwrap().nodes.push(KnowledgeCanvasNode {
        id: "note-1".into(),
        kind: KnowledgeCanvasNodeKind::Note,
        text: "可编辑内容".into(),
        ..Default::default()
    });
    assert!(store.duplicate_active());
    let serialized = serde_json::to_string(&store).unwrap();
    let reloaded: KnowledgeCanvasStore = serde_json::from_str(&serialized).unwrap();
    assert_eq!(reloaded, store);
    assert!(reloaded.validate().is_ok());
    assert_eq!(reloaded.active().unwrap().nodes[0].text, "可编辑内容");
    assert!(knowledge_canvas_json_is_valid(&serialized).is_some());

    let mut invalid = reloaded;
    invalid.active_mut().unwrap().title = " \t ".into();
    assert!(
        invalid.validate().is_err(),
        "blank board names cannot be saved"
    );
}

#[test]
fn windows_canvas_legacy_blank_title_is_repaired_without_losing_cards() {
    let mut store = KnowledgeCanvasStore::default();
    assert!(store.new_canvas("旧画布"));
    store.active_mut().unwrap().nodes.push(KnowledgeCanvasNode {
        id: "legacy-card".into(),
        kind: KnowledgeCanvasNodeKind::Note,
        text: "历史内容".into(),
        ..Default::default()
    });
    store.active_mut().unwrap().title = " \t ".into();
    let legacy = serde_json::to_string(&store).unwrap();

    let repaired_json = knowledge_canvas_json_is_valid(&legacy)
        .expect("a legacy blank title should not discard its canvas");
    let repaired: KnowledgeCanvasStore = serde_json::from_str(&repaired_json).unwrap();
    assert!(repaired.validate().is_ok());
    assert_eq!(repaired.canvases.len(), 1);
    assert_eq!(repaired.active_canvas_id, store.active_canvas_id);
    assert!(!repaired.active().unwrap().title.trim().is_empty());
    assert_eq!(repaired.active().unwrap().nodes[0].id, "legacy-card");
    assert_eq!(repaired.active().unwrap().nodes[0].text, "历史内容");
}
