#[test]
#[ignore = "Explicit synthetic sticky editor frame benchmark; output path required"]
fn sticky_editor_frame_benchmark() {
    let output = std::env::var_os("DESKTOP_STICKY_PERFORMANCE_OUTPUT").expect("output path");
    let mut results = Vec::new();
    for (count, history_bytes) in [(12, 4096), (64, 16384)] {
        let root = temp_test_dir("sticky_editor_perf");
        let mut client = test_client_for_account_scope(
            &root,
            "synthetic-sticky",
            app_state_with_note_versions(),
        );
        client.tab = AppTab::Notes;
        let note = client
            .data
            .notes
            .iter_mut()
            .find(|n| n.id == "note-versioned")
            .unwrap();
        let seed = note
            .versions
            .iter()
            .find(|v| v.id == "version-old")
            .unwrap()
            .clone();
        note.versions = (0..count)
            .map(|n| {
                let mut version = seed.clone();
                version.id = format!("version-{n}");
                version.sequence = n as i64 + 1;
                version.content = "history ".repeat(history_bytes / 8);
                version.document.blocks = vec![new_desktop_text_block(&version.content)];
                version
            })
            .collect();
        note.latest_version_id.clear();
        note.revisions = (0..count)
            .map(|n| DesktopNoteRevisionSnapshot {
                id: format!("recovery-{n}"),
                content: note.versions[n].content.clone(),
                document: note.versions[n].document.clone(),
                ..Default::default()
            })
            .collect();
        let history_json_bytes = serde_json::to_vec(&note).unwrap().len();
        client.data_version += 1;
        client.select_note_by_id("note-versioned");
        let ctx = workspace_test_context();
        let mut time = 0.0;
        let measurement = measure("sticky_editor_frame", 40, || {
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
                    egui::CentralPanel::default().show(ctx, |ui| client.ui_sticky_note_editor(ui));
                },
            );
        });
        assert!(!client.note_dirty);
        assert_eq!(client.selected_note_ref().unwrap().versions.len(), count);
        results.push(json!({"versions":count,"recoveryPoints":count,"historyTextBytesPerSnapshot":history_bytes,
            "noteJsonBytes":history_json_bytes,"measurement":measurement}));
        drop(client);
        fs::remove_dir_all(root).unwrap();
    }
    let report = json!({"version":WINDOWS_CLIENT_VERSION,
        "scope":"complete headless egui sticky-editor frame with folded recovery panel; excludes native GPU, disk, autosave and compositor",
        "allocationMetric":"cumulative requested bytes on test thread, not retained process memory", "results":results});
    fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!("STICKY_FRAME_PERFORMANCE {report}");
}
