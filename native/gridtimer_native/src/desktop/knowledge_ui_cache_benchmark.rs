// Windows - Compare actual cold/warm read preparation against the prior implementation.
#[test]
#[ignore = "Explicit synthetic UI cache benchmark; selected output path required"]
fn knowledge_ui_cache_performance_benchmark() {
    let output = PathBuf::from(
        std::env::var_os("DESKTOP_UI_CACHE_PERFORMANCE_OUTPUT").expect("output path"),
    );
    let mut results = Vec::new();
    for count in [256, 2048] {
        let root = temp_test_dir("knowledge_ui_cache_benchmark");
        let mut client = fixture(&root, count, 1, 64);
        for (index, note) in client.data.notes.iter_mut().enumerate() {
            note.title = format!("Project {} record {index}", index % 8);
            if index > 0 {
                note.document.knowledge.as_mut().unwrap().parent_id =
                    Some(format!("perf-{}", (index - 1) / 2));
            }
        }
        client.data_version += 1;
        client.desktop_ui.experience.palette_query = "Project 3".into();
        client.desktop_ui.experience.trail = vec!["perf-7".into(), "perf-2".into()];
        assert_eq!(
            client.legacy_knowledge_quick_choices(),
            *client.knowledge_quick_choices()
        );
        let legacy = measure("quick_search_legacy_rebuild", 24, || {
            std::hint::black_box(client.legacy_knowledge_quick_choices());
        });
        let indexed_cold = measure("quick_search_changed_snapshot", 24, || {
            client.invalidate_knowledge_read_cache();
            std::hint::black_box(client.knowledge_quick_choices());
        });
        let indexed_warm = measure("quick_search_unchanged_frame", 24, || {
            std::hint::black_box(client.knowledge_quick_choices());
        });
        let rows = Arc::new(
            client
                .data
                .notes
                .iter()
                .rev()
                .map(|note| note.id.clone())
                .collect::<Vec<_>>(),
        );
        let records = client.knowledge_records();
        let lookup = client.knowledge_record_lookup(&records);
        let expected = lookup.filter_rows(&rows, "Project 3").into_owned();
        assert_eq!(
            expected,
            *client.cached_knowledge_database_rows(&rows, &records, &lookup, "Project 3")
        );
        let database_legacy = measure("database_legacy_prepare_search", 24, || {
            // Previous production preparation copied query IDs, then built a borrowed
            // index and searched every title on every frame, including warm frames.
            let all_rows = rows.as_ref().clone();
            let mut by_id = std::collections::HashMap::with_capacity(records.len());
            for page in records.iter() {
                by_id.entry(page.id.as_str()).or_insert(page);
            }
            let query = "Project 3".trim().to_lowercase();
            let filtered = all_rows
                .iter()
                .filter(|id| {
                    by_id.get(id.as_str()).is_some_and(|page| {
                        !page.encrypted
                            && !page.deleted
                            && page.title.to_lowercase().contains(&query)
                    })
                })
                .cloned()
                .collect::<Vec<_>>();
            std::hint::black_box((all_rows, by_id, filtered));
        });
        let database_cold = measure("database_changed_query", 24, || {
            let mut cache = client.desktop_ui.knowledge.read_cache.borrow_mut();
            cache.record_indices = None;
            cache.record_search = None;
            drop(cache);
            let records = client.knowledge_records();
            let lookup = client.knowledge_record_lookup(&records);
            std::hint::black_box(client.cached_knowledge_database_rows(
                &rows,
                &records,
                &lookup,
                "Project 3",
            ));
        });
        let database_warm = measure("database_unchanged_frame", 24, || {
            let records = client.knowledge_records();
            let lookup = client.knowledge_record_lookup(&records);
            std::hint::black_box(client.cached_knowledge_database_rows(
                &rows,
                &records,
                &lookup,
                "Project 3",
            ));
        });
        results.push(
            json!({"records":count + 1, "quickSearch":[legacy, indexed_cold, indexed_warm],
            "database":[database_legacy, database_cold, database_warm]}),
        );
        drop(client);
        fs::remove_dir_all(root).unwrap();
    }
    let report = json!({"version":WINDOWS_CLIENT_VERSION,
        "scope":"same-executable synthetic quick-navigation and database read preparation; excludes rendering, GPU, disk, networking and startup",
        "profile": if cfg!(debug_assertions) { "debug-test" } else { "release-test" },
        "sampling":"warmup before each sample; 24 iterations",
        "allocationMetric":"cumulative requested bytes on measured thread, not retained process memory",
        "results":results});
    fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!("KNOWLEDGE_UI_CACHE_PERFORMANCE {report}");
}
