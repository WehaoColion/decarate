// v1.0.3.9 Windows - Measure large knowledge database table redraw work.
// v1.0.3.8 Windows - Measure sticky canvas redraw allocations with synthetic multi-block notes.
// v2.22.54 - Isolated allocation and frame-time measurements; no production data is read.
mod desktop_performance_probe {
    use super::*;
    include!("sticky_editor_performance_tests.rs");
    include!("canvas_performance_tests.rs");
    include!("legal_ui_performance_tests.rs");
    include!("knowledge_ui_cache_benchmark.rs");
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    struct ProbeAllocator;
    thread_local! {
        static ALLOCATIONS: Cell<Option<(u64, u64)>> = const { Cell::new(None) };
    }
    fn record(bytes: usize) {
        let _ = ALLOCATIONS.try_with(|count| {
            if let Some((calls, total)) = count.get() {
                count.set(Some((calls + 1, total.saturating_add(bytes as u64))));
            }
        });
    }
    unsafe impl GlobalAlloc for ProbeAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let ptr = System.alloc(layout);
            record(layout.size());
            ptr
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            let ptr = System.alloc_zeroed(layout);
            record(layout.size());
            ptr
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            System.dealloc(ptr, layout);
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            let ptr = System.realloc(ptr, layout, new_size);
            record(new_size);
            ptr
        }
    }
    #[global_allocator]
    static ALLOCATOR: ProbeAllocator = ProbeAllocator;

    fn measure(name: &str, iterations: usize, mut action: impl FnMut()) -> Value {
        for _ in 0..3 {
            action();
        }
        let mut times = Vec::with_capacity(iterations);
        ALLOCATIONS.with(|v| v.set(Some((0, 0))));
        for _ in 0..iterations {
            let start = Instant::now();
            action();
            times.push(start.elapsed().as_nanos() as u64);
        }
        let (calls, bytes) = ALLOCATIONS.with(|v| v.replace(None).unwrap());
        times.sort_unstable();
        json!({"name":name,"iterations":iterations,"allocationCalls":calls,
            "allocatedBytes":bytes,"allocatedBytesPerOperation":bytes / iterations as u64,
            "medianMicros":times[iterations/2] as f64/1000.0,
            "p95Micros":times[(iterations*95/100).min(iterations-1)] as f64/1000.0})
    }

    fn fixture(root: &Path, pages: usize, blocks: usize, text_len: usize) -> TimerWindowsClient {
        let mut client = knowledge_test_client(root);
        let seed = client
            .data
            .notes
            .iter()
            .find(|n| n.id == "doc-a")
            .unwrap()
            .clone();
        let mut notes = Vec::new();
        for page in 0..pages {
            let mut note = seed.clone();
            note.id = format!("perf-{page}");
            note.title = format!("资料 {page:04}");
            note.folder_id = None;
            note.document.knowledge = Some(knowledge::KnowledgePage::default());
            note.document.blocks = (0..blocks)
                .map(|block| {
                    let mut b = new_desktop_text_block(&"sample ".repeat(text_len / 7));
                    b.id = format!("block-{page}-{block}");
                    b.knowledge = Some(Default::default());
                    b
                })
                .collect();
            note.content = note_canvas_storage_text(&note.document.blocks);
            note.revisions = (0..6)
                .map(|revision| DesktopNoteRevisionSnapshot {
                    id: format!("revision-{page}-{revision}"),
                    content: note.content.clone(),
                    document: note.document.clone(),
                    ..Default::default()
                })
                .collect();
            note.versions.clear();
            notes.push(note);
        }
        let mut database = seed;
        database.id = "perf-db".into();
        database.title = "任务库".into();
        database.content.clear();
        database.document.blocks.clear();
        database.revisions.clear();
        database.versions.clear();
        database.document.knowledge = Some(knowledge::KnowledgePage {
            database: Some(knowledge::KnowledgeDatabase::task_database()),
            ..Default::default()
        });
        for note in &mut notes {
            note.document.knowledge.as_mut().unwrap().parent_id = Some("perf-db".into());
        }
        notes.push(database);
        client.data.notes = notes;
        client.state_json = serde_json::to_string(&client.data).unwrap();
        client.data_version += 1;
        client.selected_note_id.clear();
        client.sync.token.clear();
        client.sync.server_url.clear();
        client.sync.ai_api_key.clear();
        client.select_note_by_id("perf-0");
        client.rebuild_view_cache();
        client
    }

    fn frame(client: &mut TimerWindowsClient, ctx: &egui::Context, time: &mut f64) {
        *time += 0.02;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1240.0, 820.0),
                )),
                time: Some(*time),
                ..Default::default()
            },
            |ctx| {
                client.ui_workspace(ctx);
            },
        );
    }

    #[test]
    #[ignore = "Explicit synthetic database table frame benchmark; output path required"]
    fn database_table_frame_benchmark() {
        let output = PathBuf::from(
            std::env::var_os("DESKTOP_TABLE_PERFORMANCE_OUTPUT").expect("output path"),
        );
        let root = temp_test_dir("database_table_frame_perf");
        let mut client = knowledge_test_client(&root);
        let db = knowledge::KnowledgeDatabase::task_database();
        let view = db
            .views
            .iter()
            .find(|view| view.kind == knowledge::ViewKind::Table)
            .unwrap()
            .clone();
        let records = (0..512)
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
        let mut time = 0.0;
        let measurement = measure("database_table_frame", 20, || {
            time += 0.05;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 800.0),
                    )),
                    time: Some(time),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        client.ui_knowledge_table(ui, &db, &view, &rows, &lookup, false);
                    });
                },
            );
        });
        let report = json!({"version":WINDOWS_CLIENT_VERSION,
            "scope":"headless egui database table, 512 synthetic records; excludes GPU, disk and sync",
            "allocationMetric":"cumulative requested bytes on test thread, not retained memory",
            "measurement":measurement});
        fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        println!("DATABASE_TABLE_FRAME_PERFORMANCE {report}");
        drop(client);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "Explicit synthetic multi-block canvas benchmark; output path required"]
    fn sticky_canvas_blocks_benchmark() {
        let output = PathBuf::from(
            std::env::var_os("DESKTOP_CANVAS_PERFORMANCE_OUTPUT").expect("output path"),
        );
        let mut results = Vec::new();
        for (blocks, text_len) in [(32, 2048), (128, 4096)] {
            let root = temp_test_dir("sticky_canvas_perf");
            let mut client = fixture(&root, 1, blocks, text_len);
            let note = client
                .data
                .notes
                .iter_mut()
                .find(|note| note.id == "perf-0")
                .unwrap();
            note.kind = DesktopNoteKind::Sticky.code().to_string();
            note.document.knowledge = None;
            note.revisions.clear();
            client.tab = AppTab::Notes;
            client.select_note_by_id("perf-0");
            let ctx = workspace_test_context();
            let mut time = 0.0;
            let measurement = measure("sticky_canvas_blocks_frame", 20, || {
                time += 0.05;
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1100.0, 820.0),
                        )),
                        time: Some(time),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            client.ui_note_canvas_blocks(ui);
                        });
                    },
                );
            });
            assert!(!client.note_dirty);
            assert_eq!(client.note_blocks_draft.len(), blocks);
            results.push(json!({"blocks": blocks, "textBytesPerBlock": text_len,
                "measurement": measurement}));
            drop(client);
            fs::remove_dir_all(root).unwrap();
        }
        fs::write(
            output,
            serde_json::to_vec_pretty(&json!({"results": results})).unwrap(),
        )
        .unwrap();
    }

    #[test]
    #[ignore = "Explicit synthetic knowledge block redraw benchmark; output path required"]
    fn knowledge_blocks_frame_benchmark() {
        let output = PathBuf::from(
            std::env::var_os("DESKTOP_KNOWLEDGE_BLOCKS_PERFORMANCE_OUTPUT").expect("output path"),
        );
        let root = temp_test_dir("knowledge_blocks_frame_perf");
        let mut client = fixture(&root, 1, 128, 4096);
        let mut toc = new_desktop_text_block("");
        toc.id = "perf-toc".into();
        toc.knowledge = Some(knowledge::KnowledgeBlock {
            kind: knowledge::BlockKind::TableOfContents,
            ..Default::default()
        });
        client.note_blocks_draft.insert(0, toc);
        client.tab = AppTab::Knowledge;
        let ctx = workspace_test_context();
        let mut time = 0.0;
        let measurement = measure("knowledge_blocks_frame", 20, || {
            time += 0.05;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100.0, 820.0),
                    )),
                    time: Some(time),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        client.ui_knowledge_blocks(ui);
                    });
                },
            );
        });
        assert_eq!(client.note_blocks_draft.len(), 129);
        let report = json!({"version":WINDOWS_CLIENT_VERSION,
            "scope":"headless egui knowledge editor; 128 synthetic 4 KiB blocks and one outline block",
            "allocationMetric":"cumulative requested bytes on test thread, not retained memory",
            "measurement":measurement});
        fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        println!("KNOWLEDGE_BLOCKS_FRAME_PERFORMANCE {report}");
        drop(client);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "Explicit isolated persistence benchmark; requires a selected evidence file"]
    fn desktop_privacy_save_benchmark() {
        let output =
            PathBuf::from(std::env::var_os("DESKTOP_PRIVACY_SAVE_OUTPUT").expect("output path"));
        let mut results = Vec::new();
        for (scale, pages, blocks, text_len) in
            [("representative", 43, 2, 512), ("stress", 128, 8, 1024)]
        {
            let root = temp_test_dir(&format!("privacy_save_{scale}"));
            let mut client = fixture(&root, pages, blocks, text_len);
            // The frame fixture uses display-only revision defaults. Make the
            // persisted fixture obey the shared Android document schema.
            for note in &mut client.data.notes {
                note.latest_version_id.clear();
                for revision in &mut note.revisions {
                    revision.kind = note.kind.clone();
                    revision.accent_seed = note.accent_seed.clone();
                }
            }
            client.state_json = app_data::sanitize_app_data_json(
                &serde_json::to_string(&client.data).unwrap(),
                now_millis(),
            )
            .expect("persistence benchmark fixture must be valid AppData");
            client.state_json = audit_sealed_state(&client.state_json, "perf-0");
            client.data = decode_data(&client.state_json);
            client.save_state().unwrap();
            let policy = gridtimer_native::desktop_state_store::DesktopPrivacyPolicy::default()
                .including_snapshot(&client.state_json)
                .unwrap();
            let state_bytes = client.state_json.len();
            let analysis = measure("privacy_inspect_unchanged", 12, || {
                std::hint::black_box(policy.including_snapshot(&client.state_json).unwrap());
                assert!(policy.redact_json(&client.state_json).unwrap().is_none());
            });
            let mut states = Vec::new();
            let mut value: Value = serde_json::from_str(&client.state_json).unwrap();
            for index in 0..9 {
                let note = value["notes"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|n| n["id"] == "perf-1")
                    .unwrap();
                note["title"] = json!(format!("Draft {index}"));
                note["updatedAtEpochMillis"] = json!(now_millis() + index);
                states.push(serde_json::to_string(&value).unwrap());
            }
            let mut states = states.into_iter();
            let saves = measure("save_changed_draft_after_encryption", 6, || {
                client.state_json = states.next().unwrap();
                client.save_state().unwrap();
            });
            assert_eq!(
                fs::read_to_string(&client.state_path).unwrap(),
                client.state_json
            );
            let store = open_desktop_state_store(client.sync_path.parent().unwrap()).unwrap();
            let owner = audit_owner(&client);
            assert!(!store.privacy_policy(&owner).unwrap().1);
            assert_eq!(
                store
                    .latest_valid(&owner, now_millis())
                    .unwrap()
                    .unwrap()
                    .app_data_json,
                client.state_json
            );
            drop(store);
            results.push(json!({"scale":scale, "stateBytes":state_bytes,
                "measurements":[analysis, saves]}));
            drop(client);
            fs::remove_dir_all(root).unwrap();
        }
        fs::write(
            output,
            serde_json::to_vec_pretty(&json!({"results":results})).unwrap(),
        )
        .unwrap();
    }

    #[test]
    #[ignore = "Same-build old full-note clone versus display-only list construction; synthetic data"]
    fn note_list_cache_performance_benchmark() {
        let mut results = Vec::new();
        for (pages, blocks, text_len) in [(43, 2, 512), (128, 8, 1024)] {
            let root = temp_test_dir("note_list_benchmark");
            let client = fixture(&root, pages, blocks, text_len);
            let source = &client.data.notes;
            let old = measure("full_note_list", 40, || {
                let mut rows = source.clone();
                rows.sort_by(|a, b| compare_desktop_notes(a, b, 0));
                std::hint::black_box(rows);
            });
            let new = measure("display_only_list", 40, || {
                let mut rows = source
                    .iter()
                    .map(|n| (n, DesktopNoteListItem::from_note(n, false, None, "")))
                    .collect::<Vec<_>>();
                rows.sort_by(|a, b| compare_desktop_notes(a.0, b.0, 0));
                std::hint::black_box(rows.into_iter().map(|(_, row)| row).collect::<Vec<_>>());
            });
            results.push(
                json!({"pages":pages+1,"stateBytes":client.state_json.len(),"revisionsPerPage":6,
                "fullNoteList":old,"displayOnlyList":new}),
            );
            drop(client);
            fs::remove_dir_all(root).unwrap();
        }
        let report = json!({"version":WINDOWS_CLIENT_VERSION,"scope":"same-build list row construction and disposal; no disk I/O or painting",
            "allocationMetric":"cumulative requested bytes per operation, not retained process RAM", "results":results});
        if let Some(path) = std::env::var_os("DESKTOP_NOTE_LIST_PERFORMANCE_OUTPUT") {
            fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        }
        println!("NOTE_LIST_PERFORMANCE {report}");
    }

    #[test]
    #[ignore = "Explicit isolated allocation benchmark; writes only an explicitly selected evidence file"]
    fn desktop_performance_benchmark() {
        let output =
            PathBuf::from(std::env::var_os("DESKTOP_PERFORMANCE_OUTPUT").expect("output path"));
        let mut results = Vec::new();
        for (scale, pages, blocks, text_len) in
            [("representative", 43, 2, 512), ("stress", 128, 8, 1024)]
        {
            let root = temp_test_dir(&format!("performance_{scale}"));
            let mut client = fixture(&root, pages, blocks, text_len);
            let ctx = egui::Context::default();
            install_ui_fonts(&ctx);
            install_ui_style(&ctx);
            let mut time = 1.0;
            results.push(json!({"scale":scale,"pages":pages+1,"blocksPerPage":blocks,"revisionsPerPage":6,
                "stateBytes":client.state_json.len(),"measurements":[
                measure("knowledge_records",50,|| {std::hint::black_box(client.knowledge_records());}),
                measure("page_frame",40,|| frame(&mut client,&ctx,&mut time)),
                measure("timer_projection_parse",15,|| {std::hint::black_box(app_data::TimerProjector::parse(&client.state_json).unwrap());})
            ]}));
            client.select_note_by_id("perf-db");
            let db = measure("database_frame", 40, || frame(&mut client, &ctx, &mut time));
            results.last_mut().unwrap()["measurements"]
                .as_array_mut()
                .unwrap()
                .push(db);
            // Warm both existing pages before measuring the actual selection and frame.
            // Keep this before edits so the selection save barrier has no pending draft.
            assert!(!client.note_dirty && !client.persistence.pending());
            for id in ["perf-0", "perf-1"] {
                client.select_note_by_id(id);
                frame(&mut client, &ctx, &mut time);
                assert_eq!(client.selected_note_id, id);
            }
            let mut switch_index = 0;
            let switches = measure("switch_page_and_frame", 40, || {
                let id = ["perf-0", "perf-1"][switch_index % 2];
                switch_index += 1;
                client.select_note_by_id(id);
                frame(&mut client, &ctx, &mut time);
            });
            assert!(!client.note_dirty && !client.persistence.pending());
            results.last_mut().unwrap()["measurements"]
                .as_array_mut()
                .unwrap()
                .push(switches);
            client.select_note_by_id("perf-0");
            let block = client.note_blocks_draft[0].clone();
            let mut edit = 0;
            let edits = measure("coalesced_text_edit", 40, || {
                edit += 1;
                client.apply_note_block_text_edit(&block, format!("{} {edit}", block.text));
            });
            results.last_mut().unwrap()["measurements"]
                .as_array_mut()
                .unwrap()
                .push(edits);
            client.note_dirty = false;
            drop(client);
            fs::remove_dir_all(root).unwrap();
        }
        let report = json!({"version":WINDOWS_CLIENT_VERSION,"profile":if cfg!(debug_assertions) { "debug test; compare only identical harness builds" } else { "release test; optimized; compare only identical harness builds" },
            "allocationMetric":"Cumulative requested heap bytes on the rendering test thread, not retained process memory",
            "at":now_millis(),"results":results});
        fs::write(&output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        println!("{}", report);
    }
}

#[test]
fn knowledge_read_cache_refreshes_after_save_and_discards_previous_generation() {
    let root = temp_test_dir("knowledge_read_cache_refresh");
    let mut client = knowledge_test_client(&root);
    let before = client.knowledge_records();
    assert!(Arc::ptr_eq(&before, &client.knowledge_records()));
    let weak = Arc::downgrade(&before);
    let navigation = client.knowledge_navigation();
    assert!(Arc::ptr_eq(&navigation, &client.knowledge_navigation()));
    client.note_title_draft = "修改后的标题".into();
    client.mark_note_dirty();
    assert!(client.save_note());
    let after = client.knowledge_records();
    assert_eq!(
        after.iter().find(|p| p.id == "doc-a").unwrap().title,
        "修改后的标题"
    );
    assert!(!Arc::ptr_eq(&before, &after));
    drop(before);
    assert!(
        weak.upgrade().is_none(),
        "the cache must release its previous data generation"
    );
    let updated_navigation = client.knowledge_navigation();
    assert!(!Arc::ptr_eq(&navigation, &updated_navigation));
    assert_eq!(
        updated_navigation.pages[*updated_navigation.by_id.get("doc-a").unwrap()].title,
        "修改后的标题"
    );
    assert!(client.move_knowledge_page("doc-b", Some("doc-a"), 1));
    let moved = client.knowledge_navigation();
    assert_eq!(
        moved
            .ancestors("doc-b")
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        vec!["doc-a"]
    );
    client.close_note_crypto_session();
    assert!(client
        .desktop_ui
        .knowledge
        .read_cache
        .borrow()
        .records
        .is_none());
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn coalesced_text_edits_preserve_undo_redo_and_batch_boundaries() {
    let root = temp_test_dir("coalesced_edit_memory");
    let mut client = knowledge_test_client(&root);
    let block = client.note_blocks_draft[0].clone();
    let original = client.note_content_draft.clone();
    client.apply_note_block_text_edit(&block, "第一笔".into());
    let count = client.note_undo_stack.len();
    client.note_last_edit_epoch_millis = now_millis();
    client.apply_note_block_text_edit(&block, "第二笔".into());
    assert_eq!(client.note_undo_stack.len(), count);
    client.undo_note_draft();
    assert_eq!(client.note_content_draft, original);
    client.redo_note_draft();
    assert_eq!(client.note_content_draft, "第二笔");
    client.note_last_edit_key = format!("block:{}", block.id);
    client.note_last_edit_epoch_millis = now_millis() - NOTE_EDIT_BATCH_MILLIS - 1;
    client.apply_note_block_text_edit(&block, "第三笔".into());
    client.undo_note_draft();
    assert_eq!(client.note_content_draft, "第二笔");
    client.apply_note_block_text_edit(&block, "分支编辑".into());
    assert!(client.note_redo_stack.is_empty());
    client.note_dirty = false;
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn knowledge_navigation_handles_cycles_and_omits_trashed_pages() {
    let root = temp_test_dir("navigation_cycle_cache");
    let mut client = knowledge_test_client(&root);
    for (id, parent) in [("doc-a", "doc-b"), ("doc-b", "doc-a")] {
        client
            .data
            .notes
            .iter_mut()
            .find(|n| n.id == id)
            .unwrap()
            .document
            .knowledge = Some(knowledge::KnowledgePage {
            parent_id: Some(parent.into()),
            ..Default::default()
        });
    }
    let navigation = KnowledgeNavigation::from_notes(&client.data.notes);
    assert_eq!(navigation.ancestors("doc-a").len(), 1);
    client
        .data
        .notes
        .iter_mut()
        .find(|n| n.id == "doc-b")
        .unwrap()
        .deleted_at_epoch_millis = Some(now_millis());
    let navigation = KnowledgeNavigation::from_notes(&client.data.notes);
    assert!(!navigation.by_id.contains_key("doc-b"));
    assert!(navigation.ancestors("doc-a").is_empty());
    drop(client);
    fs::remove_dir_all(root).unwrap();
}
