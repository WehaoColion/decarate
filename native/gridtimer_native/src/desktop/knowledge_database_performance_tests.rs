// v1.0.3.9 Windows - Keep large tables bounded without dropping a focused cell editor.
// v1.0.2.2 Windows - Keep indexed database search and deferred property queries correct.

#[test]
fn knowledge_gallery_preview_cache_respects_version_privacy_and_capacity() {
    let root = temp_test_dir("knowledge_gallery_preview_cache");
    let mut client = knowledge_test_client(&root);
    let mut page = knowledge::PageRecord {
        id: "gallery-record".into(),
        content: "**Hello** `world`".into(),
        ..Default::default()
    };
    let first = client.cached_knowledge_gallery_preview(&page);
    assert_eq!(
        first.split_whitespace().collect::<Vec<_>>(),
        ["Hello", "world"]
    );
    assert!(Arc::ptr_eq(
        &first,
        &client.cached_knowledge_gallery_preview(&page)
    ));

    page.meta.locked = true;
    let locked = client.cached_knowledge_gallery_preview(&page);
    assert_eq!(locked.as_str(), first.as_str());
    assert!(!Arc::ptr_eq(&first, &locked));
    page.encrypted = true;
    assert!(client.cached_knowledge_gallery_preview(&page).is_empty());
    assert!(!client
        .desktop_ui
        .knowledge
        .read_cache
        .borrow()
        .gallery_previews
        .contains_key(&page.id));

    page.encrypted = false;
    page.content = "New **content**".into();
    let unprotected = client.cached_knowledge_gallery_preview(&page);
    assert_eq!(
        unprotected.split_whitespace().collect::<Vec<_>>(),
        ["New", "content"]
    );
    client.data_version += 1;
    page.content = "Next [link](https://example.invalid)".into();
    let edited = client.cached_knowledge_gallery_preview(&page);
    assert_eq!(
        edited.split_whitespace().collect::<Vec<_>>(),
        ["Next", "link"]
    );
    assert!(!Arc::ptr_eq(&unprotected, &edited));

    // Account switches call this invalidation even if the new scope has the same version.
    client.invalidate_knowledge_read_cache();
    let other_scope = client.cached_knowledge_gallery_preview(&page);
    assert!(!Arc::ptr_eq(&edited, &other_scope));
    for index in 0..=KNOWLEDGE_GALLERY_PREVIEW_LIMIT {
        page.id = format!("record-{index}");
        let _ = client.cached_knowledge_gallery_preview(&page);
    }
    assert!(
        client
            .desktop_ui
            .knowledge
            .read_cache
            .borrow()
            .gallery_previews
            .len()
            <= KNOWLEDGE_GALLERY_PREVIEW_LIMIT
    );
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn knowledge_database_table_only_mounts_visible_rows_until_a_cell_has_focus() {
    let root = temp_test_dir("knowledge_table_visible_rows");
    let mut client = knowledge_test_client(&root);
    let db = knowledge::KnowledgeDatabase::task_database();
    let view = db
        .views
        .iter()
        .find(|view| view.kind == knowledge::ViewKind::Table)
        .unwrap()
        .clone();
    let records = (0..256)
        .map(|index| knowledge::PageRecord {
            id: format!("row-{index}"),
            title: format!("Synthetic record {index}"),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let rows = records
        .iter()
        .map(|record| record.id.clone())
        .collect::<Vec<_>>();
    let lookup = KnowledgeRecordLookup::new(&records);
    let ctx = workspace_test_context();
    let render = |client: &mut TimerWindowsClient, scroll_offset: Option<f32>| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 800.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    if let Some(offset) = scroll_offset {
                        let id =
                            ui.make_persistent_id(egui::Id::new("knowledge_database_table_scroll"));
                        let mut state =
                            egui::containers::scroll_area::State::load(ctx, id).unwrap_or_default();
                        state.offset.y = offset;
                        state.store(ctx, id);
                    }
                    client.ui_knowledge_table(ui, &db, &view, &rows, &lookup, false);
                });
            },
        )
    };
    render(&mut client, None);
    assert!(client.desktop_ui.knowledge.cell_inputs.len() < 80);
    assert!(client
        .desktop_ui
        .knowledge
        .cell_inputs
        .contains_key("table:row-0:date"));
    assert!(!client
        .desktop_ui
        .knowledge
        .cell_inputs
        .contains_key("table:row-255:date"));
    render(&mut client, Some(10_000.0));
    render(&mut client, None);
    assert!(client
        .desktop_ui
        .knowledge
        .cell_inputs
        .contains_key("table:row-255:date"));
    ctx.memory_mut(|memory| {
        memory.request_focus(egui::Id::new(("property_input", "table:row-255:date")))
    });
    render(&mut client, None);
    assert_eq!(client.desktop_ui.knowledge.cell_inputs.len(), rows.len());
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn knowledge_database_index_preserves_search_order_and_private_record_boundaries() {
    let records = vec![
        knowledge::PageRecord {
            id: "first".into(),
            title: "Alpha BETA".into(),
            ..Default::default()
        },
        knowledge::PageRecord {
            id: "second".into(),
            title: "beta Project".into(),
            ..Default::default()
        },
        knowledge::PageRecord {
            id: "encrypted".into(),
            title: "beta secret".into(),
            encrypted: true,
            ..Default::default()
        },
        knowledge::PageRecord {
            id: "deleted".into(),
            title: "beta trash".into(),
            deleted: true,
            ..Default::default()
        },
        knowledge::PageRecord {
            id: "other".into(),
            title: "unrelated".into(),
            ..Default::default()
        },
    ];
    let rows = [
        "second",
        "encrypted",
        "missing",
        "first",
        "deleted",
        "other",
    ]
    .map(str::to_owned);
    let lookup = KnowledgeRecordLookup::new(&records);
    assert_eq!(
        &*lookup.filter_rows(&rows, "  BeTa  "),
        &["second", "first"]
    );
    assert!(lookup.filter_rows(&rows, "not-present").is_empty());
    assert!(matches!(
        lookup.filter_rows(&rows, " \t "),
        std::borrow::Cow::Borrowed(_)
    ));
    assert_eq!(&*lookup.filter_rows(&rows, ""), &rows);
    assert_eq!(lookup.get("first").unwrap().title, "Alpha BETA");
    assert!(lookup.get("missing").is_none());
}

#[test]
fn knowledge_database_record_properties_defer_records_and_still_render_computed_values() {
    let dir = temp_test_dir("knowledge_database_deferred_properties");
    let mut client = knowledge_test_client(&dir);
    let mut database = knowledge::KnowledgeDatabase::task_database();
    client
        .data
        .notes
        .iter_mut()
        .find(|note| note.id == "doc-b")
        .unwrap()
        .document
        .knowledge = Some(knowledge::KnowledgePage {
        database: Some(database.clone()),
        ..Default::default()
    });
    let meta = knowledge::KnowledgePage {
        parent_id: Some("doc-b".into()),
        ..Default::default()
    };
    client
        .data
        .notes
        .iter_mut()
        .find(|note| note.id == "doc-a")
        .unwrap()
        .document
        .knowledge = Some(meta.clone());
    client.desktop_ui.knowledge.page = Some(meta);
    client.data_version += 1;
    client.invalidate_knowledge_read_cache();
    let ctx = workspace_test_context();
    ctx.style_mut(|style| style.animation_time = 0.0);
    let render = |client: &mut TimerWindowsClient| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(680.0, 500.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    client.ui_knowledge_record_properties(ui);
                });
            },
        )
    };
    let _ = render(&mut client);
    assert!(
        client
            .desktop_ui
            .knowledge
            .read_cache
            .borrow()
            .records
            .is_none(),
        "ordinary record inputs should not copy every page into query records"
    );
    database.fields.push(knowledge::DatabaseField {
        id: "calculated".into(),
        name: "Computed value".into(),
        kind: knowledge::FieldKind::Formula,
        formula: "40 + 2".into(),
        ..Default::default()
    });
    client
        .data
        .notes
        .iter_mut()
        .find(|note| note.id == "doc-b")
        .unwrap()
        .document
        .knowledge
        .as_mut()
        .unwrap()
        .database = Some(database);
    client.data_version += 1;
    client.invalidate_knowledge_read_cache();
    let output = render(&mut client);
    assert!(client
        .desktop_ui
        .knowledge
        .read_cache
        .borrow()
        .records
        .is_some());
    assert!(
        output.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Text(text) if text.galley.text() == "42")),
        "expanding a computed field must still display its actual value"
    );
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "Explicit isolated database lookup benchmark; requires a selected evidence file"]
fn knowledge_database_lookup_benchmark() {
    fn legacy(rows: &[String], records: &[knowledge::PageRecord], query: &str) -> usize {
        let all_rows = rows.to_vec();
        let count = knowledge_filter_rows(&all_rows, records, query).len();
        let filtered = knowledge_filter_rows(&all_rows, records, query);
        count
            + filtered
                .iter()
                .filter_map(|id| records.iter().find(|page| page.id == *id))
                .map(|page| page.title.len())
                .sum::<usize>()
    }
    fn indexed(rows: &[String], records: &[knowledge::PageRecord], query: &str) -> usize {
        let all_rows = rows.to_vec();
        let lookup = KnowledgeRecordLookup::new(records);
        let filtered = lookup.filter_rows(&all_rows, query);
        filtered.len()
            + filtered
                .iter()
                .filter_map(|id| lookup.get(id))
                .map(|page| page.title.len())
                .sum::<usize>()
    }
    fn summary(mut samples: Vec<u64>) -> Value {
        samples.sort_unstable();
        json!({"iterations": samples.len(), "medianMicros":samples[samples.len()/2] as f64/1000.0,
            "p95Micros":samples[samples.len()*95/100] as f64/1000.0})
    }
    let output = PathBuf::from(
        std::env::var_os("DESKTOP_DATABASE_PERFORMANCE_OUTPUT").expect("output path"),
    );
    let mut results = Vec::new();
    for count in [128, 512, 2048] {
        let records = (0..count)
            .map(|index| knowledge::PageRecord {
                id: format!("record-{index:04}"),
                title: format!("Project {} record {index}", index % 8),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let rows = records
            .iter()
            .rev()
            .map(|page| page.id.clone())
            .collect::<Vec<_>>();
        for query in ["", "Project 3"] {
            assert_eq!(
                legacy(&rows, &records, query),
                indexed(&rows, &records, query)
            );
            for _ in 0..3 {
                std::hint::black_box(legacy(&rows, &records, query));
                std::hint::black_box(indexed(&rows, &records, query));
            }
            let mut old_times = Vec::new();
            let mut new_times = Vec::new();
            for iteration in 0..24 {
                for old in [iteration % 2 == 0, iteration % 2 != 0] {
                    let start = Instant::now();
                    std::hint::black_box(if old {
                        legacy(&rows, &records, query)
                    } else {
                        indexed(&rows, &records, query)
                    });
                    let elapsed = start.elapsed().as_nanos() as u64;
                    if old {
                        old_times.push(elapsed);
                    } else {
                        new_times.push(elapsed);
                    }
                }
            }
            results.push(json!({"records":count,"query":query,"legacy":summary(old_times),"indexed":summary(new_times)}));
        }
    }
    let report = json!({"version":WINDOWS_CLIENT_VERSION,
        "scope":"Same-executable database frame row filtering and page lookup preparation, including index construction; excludes egui widgets, GPU, startup and retained process memory",
        "profile":"debug-test, warmed functions, alternating execution order", "results":results});
    fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!("{report}");
}
