fn legal_performance_prepared(
    bytes: usize,
) -> Arc<gridtimer_native::legal_scan::LegalScanPrepared> {
    use gridtimer_native::legal_scan::*;
    Arc::new(LegalScanPrepared {
        manifest: LegalScanManifest {
            workspace_id: "synthetic".into(),
            captured_at_epoch_millis: 1,
            coverage: Default::default(),
            omissions: vec![],
            evidence_count: 1,
            upload_bytes: bytes,
            estimated_calls: 2,
        },
        evidence: vec![LegalEvidence {
            id: "E1".into(),
            category: "任务".into(),
            source_path: "slot/1".into(),
            title: "性能测试资料".into(),
            event_at_epoch_millis: None,
            text: "x".repeat(bytes),
            image_data_url: None,
        }],
        batches: vec![],
    })
}

#[test]
fn legal_snapshot_version_scope_dirty_and_cancel_block_sending() {
    let root = temp_test_dir("legal_snapshot_boundary");
    let mut client = knowledge_test_client(&root);
    client.desktop_ui.legal_risk.scope = client.legal_scope();
    client.desktop_ui.legal_risk.preview_version = Some(client.data_version);
    assert!(client.legal_snapshot_current());
    client.data_version += 1;
    assert!(!client.legal_snapshot_current());
    client.desktop_ui.legal_risk.preview_version = Some(client.data_version);
    client.note_dirty = true;
    assert!(!client.legal_snapshot_current());
    client.note_dirty = false;
    client.desktop_ui.legal_risk.scope.push_str("different");
    assert!(!client.legal_snapshot_current());
    client.desktop_ui.legal_risk.scope = client.legal_scope();
    client
        .desktop_ui
        .legal_risk
        .cancel
        .store(true, AtomicOrdering::Release);
    assert!(!client.legal_snapshot_current());
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legal_preparation_drops_changed_cancelled_and_other_workspace_results() {
    let root = temp_test_dir("legal_preparation_stale");
    let mut client = knowledge_test_client(&root);
    client.desktop_ui.legal_risk.scope = client.legal_scope();
    let prepared = legal_performance_prepared(1024);
    for case in 0..4 {
        let (tx, rx) = mpsc::channel();
        client.desktop_ui.legal_risk.prepared_rx = Some(rx);
        client
            .desktop_ui
            .legal_risk
            .cancel
            .store(case == 2, AtomicOrdering::Release);
        tx.send(DesktopLegalPreparedOutcome {
            scope: if case == 1 {
                "other".into()
            } else {
                client.legal_scope()
            },
            version: client.data_version + u64::from(case == 0),
            cancellation: Arc::clone(&client.desktop_ui.legal_risk.cancel),
            digest: "test".into(),
            result: Ok(Arc::clone(&prepared)),
        })
        .unwrap();
        client.poll_legal_prepared();
        if case < 3 {
            assert!(client.desktop_ui.legal_risk.preview.is_none());
        } else {
            assert!(Arc::ptr_eq(
                &prepared,
                client.desktop_ui.legal_risk.preview.as_ref().unwrap()
            ));
        }
    }
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legal_failed_save_never_starts_preparation_and_completion_wakes_ui() {
    let root = temp_test_dir("legal_prepare_save_barrier");
    let mut client = knowledge_test_client(&root);
    let ctx = egui::Context::default();
    client.desktop_ui.legal_risk.waiting_for_save = true;
    client.workspace_persistence_ready = false;
    client.advance_legal_preparation(&ctx);
    assert!(!client.desktop_ui.legal_risk.waiting_for_save);
    assert!(client.desktop_ui.legal_risk.prepared_rx.is_none());
    assert_eq!(client.task_supervisor.active_count(), 0);
    let wakes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    for _ in 0..5 {
        let _ = ctx.run(egui::RawInput::default(), |_| {});
    }
    let count = Arc::clone(&wakes);
    ctx.set_request_repaint_callback(move |_| {
        count.fetch_add(1, AtomicOrdering::Relaxed);
    });
    let wake = DesktopLegalTaskWake(ctx.clone());
    client
        .task_supervisor
        .spawn(
            RuntimeTaskKind::LegalPreparation,
            TaskDurability::Ephemeral,
            move |_| {
                let _wake = wake;
            },
        )
        .unwrap();
    client.task_supervisor.drain_for(Duration::from_secs(1));
    assert!(wakes.load(AtomicOrdering::Relaxed) > 0);
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legal_text_pages_cover_utf8_without_truncation_and_retry_is_bounded() {
    let text = "资料😀".repeat(20_000);
    let (_, pages) = desktop_legal_text_page(&text, 0);
    let joined = (0..pages)
        .map(|page| desktop_legal_text_page(&text, page).0)
        .collect::<String>();
    assert_eq!(text, joined);
    assert_eq!(
        [60_000, 120_000, 240_000, 480_000, 900_000, 900_000],
        [1, 2, 3, 4, 5, 99].map(desktop_legal_retry_millis)
    );
}

#[test]
fn legal_closed_page_releases_payload_and_hidden_frame_drains_completion() {
    use gridtimer_native::legal_scan::*;
    let root = temp_test_dir("legal_close_and_hidden");
    let mut client = knowledge_test_client(&root);
    let scope = client.legal_scope();
    client.desktop_ui.legal_risk.scope = scope.clone();
    client.desktop_ui.legal_risk.loaded_scope = scope.clone();
    let prepared = legal_performance_prepared(1024 * 1024);
    let report = Arc::new(LegalReport {
        workspace_id: "synthetic".into(),
        captured_at_epoch_millis: 1,
        completed: false,
        findings: vec![],
        manifest: prepared.manifest.clone(),
        errors: vec!["x".repeat(1024 * 1024)],
    });
    client.desktop_ui.legal_risk.preview = Some(Arc::clone(&prepared));
    client.desktop_ui.legal_risk.selected_report = Some(Arc::clone(&report));
    client.close_legal_risk();
    assert_eq!(Arc::strong_count(&prepared), 1);
    assert_eq!(Arc::strong_count(&report), 1);
    let (tx, rx) = mpsc::channel();
    client.desktop_ui.legal_risk.running = true;
    client.desktop_ui.legal_risk.outcome_rx = Some(rx);
    client.desktop_ui.parity.tray.hidden = true;
    client.task_supervisor.begin_shutdown(); // Isolate global result draining from new disk/network jobs.
    tx.send(DesktopLegalScanOutcome {
        scope,
        version: client.data_version,
        cancellation: Arc::clone(&client.desktop_ui.legal_risk.cancel),
        saved_report_id: Some("completed-local-report".into()),
        error: None,
    })
    .unwrap();
    client.poll_desktop_legal_tasks(&egui::Context::default());
    assert!(!client.desktop_ui.legal_risk.running);
    assert!(client.desktop_ui.legal_risk.outcome_rx.is_none());
    assert!(client.desktop_ui.legal_risk.selected_report.is_none());
    assert!(!client
        .desktop_ui
        .legal_risk
        .store_queue
        .iter()
        .any(|job| matches!(job, DesktopLegalStoreRequest::Read(_))));
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legal_reopened_page_rejects_cancelled_store_and_preparation_payloads() {
    use gridtimer_native::legal_scan::LegalReport;
    let root = temp_test_dir("legal_reopen_stale_store");
    let mut client = knowledge_test_client(&root);
    let scope = client.legal_scope();
    client.desktop_ui.legal_risk.scope = scope.clone();
    client.desktop_ui.legal_risk.loaded_scope = scope.clone();
    client.desktop_ui.legal_risk.open = true;
    client.desktop_ui.legal_risk.selected_report_id = "same-report".into();
    let store_cancel = Arc::clone(&client.desktop_ui.legal_risk.store_cancel);
    let preparation_cancel = Arc::clone(&client.desktop_ui.legal_risk.cancel);
    client.close_legal_risk();
    client.desktop_ui.legal_risk.open = true;
    client.desktop_ui.legal_risk.store_cancel = Arc::new(AtomicBool::new(false));
    client.desktop_ui.legal_risk.cancel = Arc::new(AtomicBool::new(false));
    client.task_supervisor.begin_shutdown(); // Only consume the controlled late receipts.
    let prepared = legal_performance_prepared(1024);
    let report = Arc::new(LegalReport {
        workspace_id: "synthetic".into(),
        captured_at_epoch_millis: 1,
        completed: false,
        manifest: prepared.manifest.clone(),
        findings: vec![],
        errors: vec![],
    });
    let report_weak = Arc::downgrade(&report);
    let (tx, rx) = mpsc::channel();
    client.desktop_ui.legal_risk.store_rx = Some(rx);
    tx.send(DesktopLegalStoreOutcome {
        scope: scope.clone(),
        version: client.data_version,
        report_revision: client.desktop_ui.legal_risk.reports_revision,
        cancellation: Arc::clone(&store_cancel),
        result: Ok(DesktopLegalStoreValue::Read("same-report".into(), report)),
    })
    .unwrap();
    client.poll_legal_store(&egui::Context::default());
    assert!(client.desktop_ui.legal_risk.selected_report.is_none());
    assert!(report_weak.upgrade().is_none());
    let (tx, rx) = mpsc::channel();
    client.desktop_ui.legal_risk.store_rx = Some(rx);
    tx.send(DesktopLegalStoreOutcome {
        scope: scope.clone(),
        version: client.data_version,
        report_revision: client.desktop_ui.legal_risk.reports_revision,
        cancellation: store_cancel,
        result: Ok(DesktopLegalStoreValue::Unlocked(
            "old-note".into(),
            DesktopNote::default(),
        )),
    })
    .unwrap();
    client.poll_legal_store(&egui::Context::default());
    assert!(client.desktop_ui.legal_risk.unlocked_notes.is_empty());
    let prepared_weak = Arc::downgrade(&prepared);
    let (tx, rx) = mpsc::channel();
    client.desktop_ui.legal_risk.prepared_rx = Some(rx);
    tx.send(DesktopLegalPreparedOutcome {
        scope,
        version: client.data_version,
        cancellation: preparation_cancel,
        digest: "same-version".into(),
        result: Ok(prepared),
    })
    .unwrap();
    client.poll_legal_prepared();
    assert!(client.desktop_ui.legal_risk.preview.is_none());
    assert!(prepared_weak.upgrade().is_none());
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legal_sync_result_prevents_late_store_list_and_read_from_replacing_current_reports() {
    use gridtimer_native::legal_scan::LegalReport;
    let root = temp_test_dir("legal_store_revision");
    let mut client = knowledge_test_client(&root);
    let scope = client.legal_scope();
    client.desktop_ui.legal_risk.scope = scope.clone();
    client.desktop_ui.legal_risk.loaded_scope = scope.clone();
    client.desktop_ui.legal_risk.open = true;
    client.desktop_ui.legal_risk.selected_report_id = "old-report".into();
    client.task_supervisor.begin_shutdown();
    let revision = client.desktop_ui.legal_risk.reports_revision;
    let (tx, rx) = mpsc::channel();
    client.desktop_ui.legal_risk.sync_rx = Some(rx);
    tx.send(DesktopLegalSyncOutcome {
        scope: scope.clone(),
        revision: client.desktop_ui.legal_risk.sync_revision,
        result: Ok(vec![serde_json::from_value(json!({
            "id":"new-report", "createdAtEpochMillis":2, "sha256":"new",
            "completed":true, "findingCount":0 }))
        .unwrap()]),
    })
    .unwrap();
    client.poll_legal_report_sync(&egui::Context::default());
    assert_ne!(client.desktop_ui.legal_risk.reports_revision, revision);
    let (tx, rx) = mpsc::channel();
    client.desktop_ui.legal_risk.store_rx = Some(rx);
    tx.send(DesktopLegalStoreOutcome {
        scope: scope.clone(),
        version: client.data_version,
        report_revision: revision,
        cancellation: Arc::clone(&client.desktop_ui.legal_risk.store_cancel),
        result: Ok(DesktopLegalStoreValue::List(vec![])),
    })
    .unwrap();
    client.poll_legal_store(&egui::Context::default());
    assert_eq!(client.desktop_ui.legal_risk.reports.len(), 1);
    assert_eq!(client.desktop_ui.legal_risk.reports[0].id, "new-report");
    let (tx, rx) = mpsc::channel();
    client.desktop_ui.legal_risk.store_rx = Some(rx);
    tx.send(DesktopLegalStoreOutcome {
        scope,
        version: client.data_version,
        report_revision: revision,
        cancellation: Arc::clone(&client.desktop_ui.legal_risk.store_cancel),
        result: Ok(DesktopLegalStoreValue::Read(
            "old-report".into(),
            Arc::new(LegalReport {
                workspace_id: "synthetic".into(),
                captured_at_epoch_millis: 1,
                completed: false,
                manifest: legal_performance_prepared(0).manifest.clone(),
                findings: vec![],
                errors: vec![],
            }),
        )),
    })
    .unwrap();
    client.poll_legal_store(&egui::Context::default());
    assert!(client.desktop_ui.legal_risk.selected_report.is_none());
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legal_delete_cancels_sync_waits_for_it_and_survives_page_close() {
    let root = temp_test_dir("legal_delete_close");
    let mut client = knowledge_test_client(&root);
    let scope = client.legal_scope();
    client.desktop_ui.legal_risk.scope = scope.clone();
    client.desktop_ui.legal_risk.loaded_scope = scope.clone();
    client.desktop_ui.legal_risk.open = true;
    client.sync.token.clear();
    let ctx = egui::Context::default();
    let (tx, rx) = mpsc::channel();
    client.desktop_ui.legal_risk.sync_rx = Some(rx);
    let sync_cancel = Arc::clone(&client.desktop_ui.legal_risk.sync_cancel);
    let old_revision = client.desktop_ui.legal_risk.sync_revision;
    client.queue_legal_report_delete("deleted-report".into());
    assert!(sync_cancel.load(AtomicOrdering::Acquire));
    client.poll_legal_store(&ctx);
    assert!(client.desktop_ui.legal_risk.store_rx.is_none());
    assert_eq!(client.desktop_ui.legal_risk.store_queue.len(), 1);
    assert_eq!(client.task_supervisor.active_count(), 0);
    tx.send(DesktopLegalSyncOutcome {
        scope: scope.clone(),
        revision: old_revision,
        result: Ok(vec![]),
    })
    .unwrap();
    client.poll_legal_report_sync(&ctx);
    client.poll_legal_store(&ctx);
    assert!(client.desktop_ui.legal_risk.store_durable);
    let store_cancel = Arc::clone(&client.desktop_ui.legal_risk.store_cancel);
    client.close_legal_risk();
    assert!(!store_cancel.load(AtomicOrdering::Acquire));
    client.task_supervisor.drain_for(Duration::from_secs(10));
    assert_eq!(client.task_supervisor.active_count(), 0);
    client.task_supervisor.begin_shutdown(); // Consume the deletion without dispatching its refresh.
    client.poll_legal_store(&ctx);
    let store = DesktopLegalReportStore::open(&client.state_path, &scope).unwrap();
    assert!(store
        .tombstones()
        .unwrap()
        .contains(&"deleted-report".to_string()));
    assert!(client.desktop_ui.legal_risk.sync_pending);
    assert!(client
        .desktop_ui
        .legal_risk
        .store_queue
        .iter()
        .any(|job| matches!(job, DesktopLegalStoreRequest::List)));
    drop(store);
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legal_old_scan_receipt_keeps_history_without_selecting_or_loading_current_report() {
    let root = temp_test_dir("legal_old_scan_receipt");
    let mut client = knowledge_test_client(&root);
    let scope = client.legal_scope();
    client.desktop_ui.legal_risk.scope = scope.clone();
    client.desktop_ui.legal_risk.loaded_scope = scope.clone();
    client.desktop_ui.legal_risk.open = true;
    client.desktop_ui.legal_risk.preview_version = Some(client.data_version);
    client.desktop_ui.legal_risk.selected_report_id = "current-selection".into();
    client.desktop_ui.legal_risk.message = "current-session".into();
    client.task_supervisor.begin_shutdown();
    for changed_version in [false, true] {
        let cancellation = if changed_version {
            Arc::clone(&client.desktop_ui.legal_risk.cancel)
        } else {
            Arc::new(AtomicBool::new(true))
        };
        let (tx, rx) = mpsc::channel();
        client.desktop_ui.legal_risk.outcome_rx = Some(rx);
        client.desktop_ui.legal_risk.running = true;
        tx.send(DesktopLegalScanOutcome {
            scope: scope.clone(),
            version: client.data_version + u64::from(changed_version),
            cancellation,
            saved_report_id: Some("old-saved-history".into()),
            error: None,
        })
        .unwrap();
        client.poll_legal_risk();
        assert!(!client.desktop_ui.legal_risk.running);
        assert_eq!(client.desktop_ui.legal_risk.message, "current-session");
        assert_eq!(
            client.desktop_ui.legal_risk.selected_report_id,
            "current-selection"
        );
        assert!(client.desktop_ui.legal_risk.selected_report.is_none());
        assert!(client.desktop_ui.legal_risk.sync_pending);
        assert!(client
            .desktop_ui
            .legal_risk
            .store_queue
            .iter()
            .any(|job| matches!(job, DesktopLegalStoreRequest::List)));
        assert!(!client
            .desktop_ui
            .legal_risk
            .store_queue
            .iter()
            .any(|job| matches!(job, DesktopLegalStoreRequest::Read(_))));
    }
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legal_send_dispatch_requires_configuration_and_matching_digest() {
    let root = temp_test_dir("legal_send_dispatch_gate");
    let mut client = knowledge_test_client(&root);
    client.desktop_ui.legal_risk.scope = client.legal_scope();
    client.desktop_ui.legal_risk.preview_version = Some(client.data_version);
    client.desktop_ui.legal_risk.preview = Some(legal_performance_prepared(16));
    client.desktop_ui.legal_risk.preview_digest =
        format!("{:x}", Sha256::digest(client.state_json.as_bytes()));
    client.sync.ai_api_key.clear();
    client.sync.ai_base_url = "http://127.0.0.1:1/v1".into();
    client.sync.ai_model = "synthetic".into();
    client.start_legal_risk(&egui::Context::default());
    let unauthorized = client.task_supervisor.active_count();
    client.task_supervisor.begin_shutdown();
    client.task_supervisor.drain_for(Duration::from_secs(1));
    assert_eq!(unauthorized, 0);
    client.task_supervisor = TaskSupervisor::default();
    client.sync.ai_api_key = "synthetic".into();
    client.desktop_ui.legal_risk.preview_digest = "stale-content".into();
    client.start_legal_risk(&egui::Context::default());
    let stale = client.task_supervisor.active_count();
    client.task_supervisor.begin_shutdown();
    client.task_supervisor.drain_for(Duration::from_secs(1));
    assert_eq!(stale, 0);
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legal_cancel_and_account_change_stop_after_synthetic_capability_probe() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    for account_change in [false, true] {
        let root = temp_test_dir(if account_change {
            "legal_change_after_probe"
        } else {
            "legal_cancel_after_probe"
        });
        let mut client = knowledge_test_client(&root);
        let scope = client.legal_scope();
        client.desktop_ui.legal_risk.scope = scope.clone();
        client.desktop_ui.legal_risk.loaded_scope = scope;
        client.desktop_ui.legal_risk.open = true;
        client.desktop_ui.legal_risk.preview_version = Some(client.data_version);
        client.desktop_ui.legal_risk.preview_digest =
            format!("{:x}", Sha256::digest(client.state_json.as_bytes()));
        client.desktop_ui.legal_risk.preview = Some(Arc::new(
            gridtimer_native::legal_scan::prepare_scan(
                &client.state_json,
                "synthetic",
                1,
                "[]",
                "[]",
            )
            .unwrap(),
        ));
        assert!(!client
            .desktop_ui
            .legal_risk
            .preview
            .as_ref()
            .unwrap()
            .batches
            .is_empty());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        client.sync.ai_api_key = "synthetic".into();
        client.sync.ai_model = "synthetic".into();
        client.sync.ai_base_url = format!("http://{address}/v1");
        let (probe_tx, probe_rx) = mpsc::channel();
        let (reply_tx, reply_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            let started = Instant::now();
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(stream) => break stream,
                    Err(error)
                        if error.kind() == io::ErrorKind::WouldBlock
                            && started.elapsed() < Duration::from_secs(10) =>
                    {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(error) => panic!("synthetic capability probe not received: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            let header = String::from_utf8(request).unwrap();
            let length: usize = header
                .lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .map(|(_, value)| value.trim().parse().unwrap())
                })
                .unwrap();
            let mut body = vec![0; length];
            stream.read_exact(&mut body).unwrap();
            let body = serde_json::from_slice::<Value>(&body).unwrap();
            assert_eq!(body["store"], false);
            probe_tx.send(()).unwrap();
            reply_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            let body = json!({"status":"completed","output":[{"content":[{"type":"output_text","text":"{\"findings\":[]}"}]}]}).to_string();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            listener
        });
        client.start_legal_risk(&egui::Context::default());
        let outcome = client
            .desktop_ui
            .legal_risk
            .outcome_rx
            .take()
            .expect("authorized worker started");
        probe_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        if account_change {
            client.sync.account_namespace.push_str("-changed");
            client.ensure_legal_scope();
        } else {
            client.close_legal_risk();
        }
        reply_tx.send(()).unwrap();
        outcome.recv_timeout(Duration::from_secs(10)).unwrap();
        client.task_supervisor.drain_for(Duration::from_secs(1));
        let listener = server.join().unwrap();
        listener.set_nonblocking(true).unwrap();
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        drop(client);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
#[ignore = "Explicit synthetic legal frame allocation benchmark; no user data or network"]
fn legal_preview_frame_allocation_benchmark() {
    let output = std::env::var_os("DESKTOP_LEGAL_PERFORMANCE_OUTPUT").map(PathBuf::from);
    let mut results = Vec::new();
    for mib in [1, 8, 32] {
        let root = temp_test_dir(&format!("legal_frame_{mib}"));
        let mut client = knowledge_test_client(&root);
        client.desktop_ui.legal_risk.scope = client.legal_scope();
        client.desktop_ui.legal_risk.loaded_scope = client.legal_scope();
        client.desktop_ui.legal_risk.preview_version = Some(client.data_version);
        client.desktop_ui.legal_risk.open = true;
        client.desktop_ui.legal_risk.preview = Some(legal_performance_prepared(mib * 1024 * 1024));
        let ctx = workspace_test_context();
        let mut time = 0.0;
        let mut render = |legacy_clone: bool| {
            let shared = Arc::clone(client.desktop_ui.legal_risk.preview.as_ref().unwrap());
            let legacy = legacy_clone.then(|| (*shared).clone());
            let prepared = legacy.as_ref().unwrap_or(&shared);
            time += 0.02;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1240.0, 820.0),
                    )),
                    time: Some(time),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        client.ui_legal_preview(ui, prepared);
                    });
                },
            );
        };
        let before = measure("legacy_deep_clone_and_preview_frame", 30, || render(true));
        let after = measure("shared_preview_frame", 30, || render(false));
        assert!(
            before["allocatedBytesPerOperation"].as_u64().unwrap()
                > after["allocatedBytesPerOperation"].as_u64().unwrap()
                    + (mib * 1024 * 1024 / 2) as u64
        );
        results.push(json!({"payloadMiB":mib,"before":before,"after":after}));
        drop(client);
        fs::remove_dir_all(root).unwrap();
    }
    let report = json!({"version":WINDOWS_CLIENT_VERSION,"scope":"synthetic collapsed legal preview; same render path and only snapshot ownership differs", "results":results});
    if let Some(path) = output {
        fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    println!("LEGAL_PREVIEW_PERFORMANCE {report}");
}

#[test]
#[ignore = "Explicit synthetic legal report frame benchmark; no user data or network"]
fn legal_report_frame_allocation_benchmark() {
    use gridtimer_native::legal_scan::*;
    let mut results = Vec::new();
    for mib in [1, 16, 32] {
        let root = temp_test_dir(&format!("legal_report_frame_{mib}"));
        let mut client = knowledge_test_client(&root);
        client.desktop_ui.legal_risk.reports = vec![serde_json::from_value(json!({
            "id":"synthetic", "createdAtEpochMillis":1, "sha256":"", "completed":true,
            "findingCount":1 }))
        .unwrap()];
        client.desktop_ui.legal_risk.selected_report_id = "synthetic".into();
        let report = Arc::new(LegalReport {
            workspace_id: "synthetic".into(),
            captured_at_epoch_millis: 1,
            completed: true,
            manifest: legal_performance_prepared(0).manifest.clone(),
            errors: vec![],
            findings: vec![LegalFinding {
                title: "待核查线索".into(),
                area: "合同".into(),
                fact: "x".repeat(mib * 1024 * 1024),
                evidence: vec![],
                event_at_epoch_millis: None,
                missing_facts: String::new(),
                recommendation: String::new(),
                laws: vec![],
            }],
        });
        let ctx = workspace_test_context();
        let mut time = 0.0;
        let mut render = |legacy_clone: bool| {
            client.desktop_ui.legal_risk.selected_report = Some(if legacy_clone {
                Arc::new((*report).clone())
            } else {
                Arc::clone(&report)
            });
            time += 0.02;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1240.0, 820.0),
                    )),
                    time: Some(time),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        client.ui_legal_reports(ui);
                    });
                },
            );
        };
        let before = measure("legacy_deep_clone_and_report_frame", 30, || render(true));
        let after = measure("shared_report_frame", 30, || render(false));
        assert!(
            before["allocatedBytesPerOperation"].as_u64().unwrap()
                > after["allocatedBytesPerOperation"].as_u64().unwrap()
                    + (mib * 1024 * 1024 / 2) as u64
        );
        results.push(json!({"payloadMiB":mib,"before":before,"after":after}));
        drop(client);
        fs::remove_dir_all(root).unwrap();
    }
    let report = json!({"version":WINDOWS_CLIENT_VERSION,
        "scope":"synthetic collapsed legal findings; identical render path with legacy deep clone or shared ownership",
        "results":results});
    if let Some(path) = std::env::var_os("DESKTOP_LEGAL_REPORT_PERFORMANCE_OUTPUT") {
        fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    println!("LEGAL_REPORT_PERFORMANCE {report}");
}

#[test]
#[ignore = "Explicit 32 MiB synthetic preparation ownership/memory probe; no AI network"]
fn legal_preparation_memory_reclamation_probe() {
    use gridtimer_native::legal_scan::{LegalFinding, LegalReport};
    use std::io::Write as _;

    fn marker(stage: &str, details: Value, hold_millis: u64) {
        println!(
            "LEGAL_PREPARATION_MEMORY_STAGE {}",
            json!({
                "stage": stage, "pid": std::process::id(), "details": details,
                "holdMillis": hold_millis,
            })
        );
        std::io::stdout().flush().unwrap();
        if hold_millis > 0 {
            thread::sleep(Duration::from_millis(hold_millis));
        }
    }

    const PAYLOAD_BYTES: usize = 32 * 1024 * 1024;
    const NOTE_BYTES: usize = 1024 * 1024;
    let hold_millis = std::env::var("DESKTOP_LEGAL_PREPARATION_MEMORY_HOLD_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1500)
        .min(5000);
    marker(
        "process_ready",
        json!({"payloadBytes": PAYLOAD_BYTES}),
        hold_millis,
    );
    let root = temp_test_dir("legal_preparation_memory_reclamation");
    let mut client = knowledge_test_client(&root);
    client.data.slots.clear();
    client.data.sessions.clear();
    client.data.archived_tasks.clear();
    client.data.notes = (0..32)
        .map(|index| {
            let phrase = "Synthetic readable legal preparation record. ";
            let mut content = phrase.repeat(NOTE_BYTES.div_ceil(phrase.len()));
            content.truncate(NOTE_BYTES);
            DesktopNote {
                id: format!("memory-note-{index}"),
                title: format!("Synthetic record {index}"),
                kind: "STICKY".into(),
                content,
                ..Default::default()
            }
        })
        .collect();
    assert_eq!(
        client
            .data
            .notes
            .iter()
            .map(|note| note.content.len())
            .sum::<usize>(),
        PAYLOAD_BYTES
    );
    client.state_json = serde_json::to_string(&client.data).unwrap();
    client.data_version += 1;
    // A clean, already loaded fixture measures preparation rather than a 32 MiB save.
    fs::write(&client.state_path, &client.state_json).unwrap();
    client.sync.token.clear();
    client.sync.ai_api_key.clear();
    client.sync.server_url.clear();
    client.sync.ai_base_url.clear();
    client.startup_sync_due_epoch_millis = 0;
    client.note_dirty = false;
    client.slot_dirty = false;
    client.finance_dirty = false;
    client.theme_dirty = false;
    client.desktop_ui.legal_risk.scope = client.legal_scope();
    client.desktop_ui.legal_risk.loaded_scope = client.legal_scope();
    client.desktop_ui.legal_risk.open = true;
    let state_bytes = client.state_json.len();
    let ctx = egui::Context::default();
    marker(
        "fixture_ready",
        json!({"payloadBytes": PAYLOAD_BYTES, "stateBytes": state_bytes,
        "noteCount": client.data.notes.len(), "cleanFixture": true}),
        hold_millis,
    );

    let started = Instant::now();
    client.prepare_legal_risk(&ctx);
    assert!(client.desktop_ui.legal_risk.waiting_for_save);
    client.advance_legal_preparation(&ctx);
    assert!(client.desktop_ui.legal_risk.prepared_rx.is_some());
    assert!(client.task_supervisor.active_count() > 0);
    marker("worker_started", json!({"stateBytes": state_bytes}), 0);
    while client.desktop_ui.legal_risk.prepared_rx.is_some() {
        client.poll_legal_prepared();
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "preparation worker exceeded bounded probe deadline"
        );
        if client.desktop_ui.legal_risk.prepared_rx.is_some() {
            thread::sleep(Duration::from_millis(5));
        }
    }
    let prepare_micros = started.elapsed().as_micros() as u64;
    client.task_supervisor.drain_for(Duration::from_secs(1));
    assert_eq!(client.task_supervisor.active_count(), 0);
    let preview = client
        .desktop_ui
        .legal_risk
        .preview
        .as_ref()
        .unwrap_or_else(|| {
            panic!(
                "preparation failed: {}",
                client.desktop_ui.legal_risk.message
            )
        });
    let evidence_text_bytes = preview
        .evidence
        .iter()
        .map(|item| item.text.len())
        .sum::<usize>();
    let evidence_image_bytes = preview
        .evidence
        .iter()
        .map(|item| item.image_data_url.as_ref().map_or(0, String::len))
        .sum::<usize>();
    let batch_text_bytes = preview
        .batches
        .iter()
        .flat_map(|batch| &batch.parts)
        .map(|part| part.text.len())
        .sum::<usize>();
    let evidence_count = preview.evidence.len();
    let batch_count = preview.batches.len();
    assert!(
        evidence_text_bytes >= PAYLOAD_BYTES,
        "readable fixture was truncated"
    );
    assert_eq!(
        batch_text_bytes, evidence_text_bytes,
        "all evidence text must reach batches"
    );
    assert_eq!(evidence_image_bytes, 0);
    let preview_weak = Arc::downgrade(preview);
    let manifest = preview.manifest.clone();
    marker(
        "prepared",
        json!({"stateBytes": state_bytes, "evidenceTextBytes": evidence_text_bytes,
        "evidenceImageBytes": evidence_image_bytes, "batchTextBytes": batch_text_bytes,
        "evidenceCount": evidence_count, "batchCount": batch_count, "prepareElapsedMicros": prepare_micros,
        "previewStrongOwners": preview_weak.strong_count()}),
        hold_millis,
    );

    let phrase = "Synthetic final report fact. ";
    let mut fact = phrase.repeat(PAYLOAD_BYTES.div_ceil(phrase.len()));
    fact.truncate(PAYLOAD_BYTES);
    let report = Arc::new(LegalReport {
        workspace_id: "synthetic-memory-report".into(),
        captured_at_epoch_millis: 1,
        completed: false,
        manifest,
        errors: vec![],
        findings: vec![LegalFinding {
            title: "Synthetic finding".into(),
            area: "合同".into(),
            fact,
            evidence: vec![],
            event_at_epoch_millis: None,
            missing_facts: String::new(),
            recommendation: String::new(),
            laws: vec![],
        }],
    });
    let report_weak = Arc::downgrade(&report);
    client.desktop_ui.legal_risk.selected_report = Some(report);
    marker(
        "report_loaded",
        json!({"reportFactBytes": PAYLOAD_BYTES,
        "previewStrongOwners": preview_weak.strong_count(), "reportStrongOwners": report_weak.strong_count()}),
        hold_millis,
    );

    client.close_legal_risk();
    assert!(
        preview_weak.upgrade().is_none(),
        "closed preview still has a strong owner"
    );
    assert!(
        report_weak.upgrade().is_none(),
        "closed report still has a strong owner"
    );
    assert!(client.desktop_ui.legal_risk.preview.is_none());
    assert!(client.desktop_ui.legal_risk.selected_report.is_none());
    marker(
        "closed",
        json!({"previewReleased": true, "reportReleased": true,
        "previewStrongOwners": preview_weak.strong_count(), "reportStrongOwners": report_weak.strong_count()}),
        hold_millis,
    );
    drop(client);
    drop(ctx);
    fs::remove_dir_all(root).unwrap();
    marker(
        "client_dropped",
        json!({"previewReleased": preview_weak.upgrade().is_none(),
        "reportReleased": report_weak.upgrade().is_none()}),
        hold_millis,
    );

    let result = json!({"version": WINDOWS_CLIENT_VERSION, "pid": std::process::id(),
        "scope": "32 MiB synthetic clean workspace through prepare_legal_risk, advance_legal_preparation and the supervised worker; no AI requests",
        "payloadBytes": PAYLOAD_BYTES, "stateBytes": state_bytes, "evidenceTextBytes": evidence_text_bytes,
        "evidenceImageBytes": evidence_image_bytes, "batchTextBytes": batch_text_bytes,
        "evidenceCount": evidence_count, "batchCount": batch_count, "prepareElapsedMicros": prepare_micros,
        "observationPollMillis": 5, "reportFactBytes": PAYLOAD_BYTES,
        "previewReleasedOnClose": true, "reportReleasedOnClose": true,
        "memoryMeasurement": "sample process working/private/peak memory externally at the emitted stages; ownership release does not imply the allocator immediately returns every page to Windows"});
    if let Some(path) = std::env::var_os("DESKTOP_LEGAL_PREPARATION_MEMORY_OUTPUT") {
        fs::write(path, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    }
    println!("LEGAL_PREPARATION_MEMORY_RESULT {result}");
    std::io::stdout().flush().unwrap();
}
