// v1.0.3 Windows - Guard history comparison invalidation and privacy boundaries.

fn history_performance_fixture(root: &Path, blocks: usize, revisions: usize) -> TimerWindowsClient {
    let mut client = knowledge_test_client(root);
    let note = client
        .data
        .notes
        .iter_mut()
        .find(|note| note.id == "doc-a")
        .unwrap();
    note.document.blocks = (0..blocks)
        .map(|index| {
            let mut block =
                new_desktop_text_block(&format!("Paragraph {index} {}", "sample ".repeat(70)));
            block.id = format!("block-{index}");
            block
        })
        .collect();
    note.revisions = (0..revisions)
        .map(|index| DesktopNoteRevisionSnapshot {
            id: format!("history-{index}"),
            label: format!("Checkpoint {index}"),
            title: note.title.clone(),
            document: note.document.clone(),
            kind: note.kind.clone(),
            captured_at_epoch_millis: 1_700_000_000_000 + index as i64,
            ..Default::default()
        })
        .collect();
    note.document.blocks[0].text = "Current first paragraph".into();
    client.note_blocks_draft = note.document.blocks.clone();
    client.data_version += 1;
    client.desktop_ui.knowledge.compare_revision = "history-0".into();
    client
}

#[test]
fn knowledge_history_comparison_tracks_text_undo_structure_revision_and_source_changes() {
    let root = temp_test_dir("history_comparison_changes");
    let mut client = history_performance_fixture(&root, 3, 2);
    let rows = client.cached_knowledge_history_rows().unwrap();
    assert_eq!(rows.len(), 2);
    let original = client.cached_knowledge_history_comparison().unwrap();
    assert_eq!(
        (original.added, original.removed, original.changed),
        (0, 0, 1)
    );
    assert!(Arc::ptr_eq(
        &original,
        &client.cached_knowledge_history_comparison().unwrap()
    ));
    let block = client.note_blocks_draft[0].clone();
    client.apply_note_block_text_edit(&block, "Unsaved replacement".into());
    let edited = client.cached_knowledge_history_comparison().unwrap();
    assert!(!Arc::ptr_eq(&original, &edited));
    assert!(edited.current_markdown.contains("Unsaved replacement"));
    assert!(!edited.old_markdown.contains("Unsaved replacement"));
    client.undo_note_draft();
    assert_eq!(
        client
            .cached_knowledge_history_comparison()
            .unwrap()
            .current_markdown,
        original.current_markdown
    );
    client.redo_note_draft();
    assert!(client
        .cached_knowledge_history_comparison()
        .unwrap()
        .current_markdown
        .contains("Unsaved replacement"));
    client.insert_knowledge_block(knowledge::BlockKind::Paragraph, None, 0, None);
    assert_eq!(
        client.cached_knowledge_history_comparison().unwrap().added,
        1
    );
    client.desktop_ui.knowledge.compare_revision = "history-1".into();
    let selected = client.cached_knowledge_history_comparison().unwrap();
    assert_eq!(selected.revision_id, "history-1");
    client
        .data
        .notes
        .iter_mut()
        .find(|note| note.id == "doc-a")
        .unwrap()
        .revisions[1]
        .document
        .blocks[1]
        .text = "Updated stored recovery content".into();
    client.data_version += 1;
    let refreshed = client.cached_knowledge_history_comparison().unwrap();
    assert!(refreshed
        .old_markdown
        .contains("Updated stored recovery content"));
    assert!(!Arc::ptr_eq(&selected, &refreshed));
    assert!(!Arc::ptr_eq(
        &rows,
        &client.cached_knowledge_history_rows().unwrap()
    ));
    client.note_dirty = false;
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn knowledge_history_cache_releases_plaintext_on_lock_and_page_change() {
    let root = temp_test_dir("history_comparison_privacy");
    let mut client = history_performance_fixture(&root, 2, 2);
    let index = client
        .data
        .notes
        .iter()
        .position(|note| note.id == "doc-a")
        .unwrap();
    client.data.notes[index].encryption = Some(Default::default());
    client.note_unlocked_record = Some(client.data.notes[index].clone());
    let comparison = client.cached_knowledge_history_comparison().unwrap();
    let rows = client.cached_knowledge_history_rows().unwrap();
    let comparison_weak = Arc::downgrade(&comparison);
    let rows_weak = Arc::downgrade(&rows);
    drop(comparison);
    drop(rows);
    client.close_note_crypto_session();
    assert!(comparison_weak.upgrade().is_none());
    assert!(rows_weak.upgrade().is_none());
    assert!(client.cached_knowledge_history_comparison().is_none());
    assert!(client.cached_knowledge_history_rows().is_none());
    client.select_note_by_id("doc-b");
    assert!(client.cached_knowledge_history_comparison().is_none());
    assert!(client.cached_knowledge_history_rows().unwrap().is_empty());
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "Explicit isolated history read-model benchmark; requires a selected evidence file"]
fn knowledge_history_read_models_benchmark() {
    fn legacy(client: &TimerWindowsClient) -> (usize, usize, usize, String, String) {
        let mut note = client.selected_note().unwrap();
        if client.note_dirty {
            note.title = client.note_title_draft.clone();
            note.document.blocks = client.note_blocks_draft.clone();
            note.document.knowledge = client.desktop_ui.knowledge.page.clone();
        }
        let rows = note
            .revisions
            .iter()
            .map(|revision| {
                (
                    revision.id.clone(),
                    revision.label.clone(),
                    desktop_local_timestamp(revision.captured_at_epoch_millis),
                )
            })
            .collect::<Vec<_>>();
        std::hint::black_box(rows);
        let revision = note
            .revisions
            .iter()
            .find(|revision| revision.id == client.desktop_ui.knowledge.compare_revision)
            .unwrap();
        let old_ids = revision
            .document
            .blocks
            .iter()
            .map(|block| block.id.as_str())
            .collect::<HashSet<_>>();
        let new_ids = note
            .document
            .blocks
            .iter()
            .map(|block| block.id.as_str())
            .collect::<HashSet<_>>();
        let added = new_ids.difference(&old_ids).count();
        let removed = old_ids.difference(&new_ids).count();
        let changed = note
            .document
            .blocks
            .iter()
            .filter(|block| {
                revision
                    .document
                    .blocks
                    .iter()
                    .find(|old| old.id == block.id)
                    .is_some_and(|old| old != *block)
            })
            .count();
        let markdown = |blocks: &[DesktopNoteBlock]| {
            knowledge::blocks_markdown(
                &blocks
                    .iter()
                    .filter_map(|block| serde_json::to_value(block).ok())
                    .collect::<Vec<_>>(),
            )
        };
        (
            added,
            removed,
            changed,
            markdown(&revision.document.blocks),
            markdown(&note.document.blocks),
        )
    }
    fn summary(mut times: Vec<u64>) -> Value {
        times.sort_unstable();
        json!({"iterations":times.len(), "medianMicros":times[times.len()/2] as f64/1000.0,
            "p95Micros":times[times.len()*95/100] as f64/1000.0})
    }
    let output =
        PathBuf::from(std::env::var_os("DESKTOP_HISTORY_PERFORMANCE_OUTPUT").expect("output path"));
    let mut results = Vec::new();
    for (blocks, revisions) in [(64, 6), (256, 20)] {
        let root = temp_test_dir("history_read_model_benchmark");
        let client = history_performance_fixture(&root, blocks, revisions);
        let cold = Instant::now();
        let new = client.cached_knowledge_history_comparison().unwrap();
        client.cached_knowledge_history_rows().unwrap();
        let cold_micros = cold.elapsed().as_nanos() as f64 / 1000.0;
        let old = legacy(&client);
        assert_eq!((new.added, new.removed, new.changed), (old.0, old.1, old.2));
        assert_eq!((&new.old_markdown, &new.current_markdown), (&old.3, &old.4));
        let mut old_times = Vec::new();
        let mut new_times = Vec::new();
        for iteration in 0..24 {
            for before in [iteration % 2 == 0, iteration % 2 != 0] {
                let started = Instant::now();
                if before {
                    std::hint::black_box(legacy(&client));
                } else {
                    std::hint::black_box(client.cached_knowledge_history_rows().unwrap());
                    std::hint::black_box(client.cached_knowledge_history_comparison().unwrap());
                }
                let elapsed = started.elapsed().as_nanos() as u64;
                if before {
                    old_times.push(elapsed);
                } else {
                    new_times.push(elapsed);
                }
            }
        }
        results.push(
            json!({"blocks":blocks,"recoveryVersions":revisions,"coldReadModelMicros":cold_micros,
            "legacyPerFrame":summary(old_times),"cachedPerFrame":summary(new_times)}),
        );
        drop(client);
        fs::remove_dir_all(root).unwrap();
    }
    let report = json!({"version":WINDOWS_CLIENT_VERSION,
        "scope":"same-binary synthetic history row and selected comparison preparation; includes legacy full-note/history clone, diff and Markdown; excludes egui layout, GPU and persistence",
        "results":results});
    fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!("{report}");
}
