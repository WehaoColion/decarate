// v2.22.51 - Keep journal tamper detection intact when reusing parsed snapshots.

#[test]
fn journal_audit_changed_bytes_cannot_reuse_previous_semantics() {
    let (directory, store) = temp_store("analysis_changed_bytes");
    let backup_json = state_with_sessions(3, 100);
    store
        .record("guest", &backup_json, 1_000, "local_save")
        .unwrap();
    let newest_json = state_with_sessions(4, 200);
    let mut newest = store
        .record("guest", &newest_json, 2_000, "local_save")
        .unwrap();
    verify_snapshot(&newest, Some("guest"), 0).unwrap();
    let mut changed: Value = serde_json::from_str(&newest.app_data_json).unwrap();
    changed["slots"][0]["title"] = json!("Changed outside the journal");
    newest.app_data_json = changed.to_string();
    newest.raw_sha256 = sha256_hex(newest.app_data_json.as_bytes());
    // Even a rewritten raw digest and envelope cannot lend the old canonical
    // and semantic metadata to different document bytes.
    newest.envelope_sha256 = snapshot_envelope_sha256(
        &newest.owner,
        newest.schema_version,
        newest.revision,
        newest.item_count,
        &newest.semantic_summary,
        &newest.raw_sha256,
        &newest.canonical_json_sha256,
        &newest.sync_state_sha256,
        &newest.parent_envelope_sha256,
        newest.created_at_epoch_millis,
        &newest.source,
    );
    let connection = store.open_connection(false).unwrap();
    connection.execute(
        "UPDATE desktop_state_snapshots SET app_data_json=?1, raw_sha256=?2, envelope_sha256=?3 WHERE id=?4",
        params![newest.app_data_json, newest.raw_sha256, newest.envelope_sha256, newest.id],
    ).unwrap();
    drop(connection);
    let recovered = store.latest_valid("guest", 3_000).unwrap().unwrap();
    assert_eq!(backup_json, recovered.app_data_json);
    assert_eq!(1, quarantine_count(&store));
    drop(store);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn journal_audit_cached_json_still_checks_sync_and_owner() {
    let (directory, store) = temp_store("analysis_sync_owner");
    let raw = state_with_sessions(3, 500);
    let snapshot = store
        .record_with_sync_state("guest", &raw, b"protected-a", 1_000, "local_save")
        .unwrap();
    verify_snapshot(&snapshot, Some("guest"), 0).unwrap();
    let mut changed = snapshot.clone();
    changed.protected_sync_state = b"protected-b".to_vec();
    assert!(verify_snapshot(&changed, Some("guest"), 0).is_err());
    assert!(verify_snapshot(&snapshot, Some("another-account"), 0).is_err());
    changed = snapshot.clone();
    changed.item_count += 1;
    changed.envelope_sha256 = snapshot_envelope_sha256(
        &changed.owner,
        changed.schema_version,
        changed.revision,
        changed.item_count,
        &changed.semantic_summary,
        &changed.raw_sha256,
        &changed.canonical_json_sha256,
        &changed.sync_state_sha256,
        &changed.parent_envelope_sha256,
        changed.created_at_epoch_millis,
        &changed.source,
    );
    assert!(verify_snapshot(&changed, Some("guest"), 0).is_err());
    assert_eq!(
        raw,
        store
            .latest_valid("guest", 2_000)
            .unwrap()
            .unwrap()
            .app_data_json
    );
    drop(store);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
#[ignore = "Opt-in synthetic repeated history parsing benchmark"]
fn journal_audit_repeated_history_analysis_benchmark() {
    let output = std::env::var_os("TIMER_JOURNAL_BENCHMARK_RESULT").expect("isolated result path");
    let base: Value = serde_json::from_str(&state_with_sessions(2_000, 100)).unwrap();
    let corpus: Vec<String> = (0..64)
        .map(|index| {
            let mut value = base.clone();
            value["slots"][0]["title"] = json!(format!("Synthetic history {index}"));
            value.to_string()
        })
        .collect();
    let mut durations = Vec::new();
    for _ in 0..3 {
        let started = std::time::Instant::now();
        for raw in &corpus {
            let analysis = analyze_app_data_json(raw, 0).unwrap();
            assert!(analysis.item_count >= 2_000);
        }
        durations.push(started.elapsed().as_micros() as u64);
    }
    let report = json!({"ok":true,"snapshots":corpus.len(),"sessionsPerSnapshot":2000,"totalJsonBytes":corpus.iter().map(String::len).sum::<usize>(),"passes":3,"passMicros":durations});
    fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!("{report}");
}
