// v2.22.47 - Guard cross-platform state boundaries, periods and export identity.

#[test]
fn parity_timer_today_uses_end_time_and_keeps_only_twenty_recent_records() {
    let sessions = (0..24)
        .map(|i| DesktopSession {
            id: format!("s{i}"),
            slot_id: 1,
            started_at_epoch_millis: 80 + i,
            ended_at_epoch_millis: 90 + i,
            duration_millis: 10,
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let summary = android_timer_stats(&sessions, (100, 110));
    let s = &summary[&1];
    assert_eq!(10, s.today_count);
    assert_eq!(100, s.today_millis);
    assert_eq!(24, s.count);
    assert_eq!(20, s.recent.len());
    assert_eq!("s23", s.recent[0].id);
    assert_eq!(10, s.longest);
}

#[test]
fn parity_drag_reordering_preserves_unmoved_slots_and_blocks_other_workspaces() {
    assert_eq!(
        Some(vec![1, 3, 4, 2]),
        reordered_timer_ids(&[1, 2, 3, 4], 2, 4)
    );
    assert_eq!(
        Some(vec![4, 1, 2, 3]),
        reordered_timer_ids(&[1, 2, 3, 4], 4, 1)
    );
    assert!(reordered_timer_ids(&[1, 2, 3], 7, 1).is_none());
    let dir = temp_test_dir("parity_drag");
    let mut client =
        test_client_for_account_scope(&dir, "drag", app_data::default_app_data_json(now_millis()));
    let before = client.state_json.clone();
    assert!(!client.reorder_timer_from_drop(
        &TimerDragPayload {
            slot_id: 1,
            workspace: "other-account".into()
        },
        2
    ));
    assert_eq!(before, client.state_json);
    let payload = TimerDragPayload {
        slot_id: 1,
        workspace: client.background_job_workspace_fingerprint(),
    };
    client.sync_task = Some(SyncTaskState {
        kind: SyncTaskKind::Download,
        phase: SyncTaskPhase::AppData,
        started_at_epoch_millis: now_millis(),
    });
    assert!(!client.reorder_timer_from_drop(&payload, 2));
    assert_eq!(before, client.state_json);
    client.sync_task = None;
    assert!(client.reorder_timer_from_drop(&payload, 2));
    let after: Value = serde_json::from_str(&client.state_json).unwrap();
    assert_eq!(
        json!([2, 1]),
        json!([after["slotOrder"][0], after["slotOrder"][1]])
    );
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn parity_finance_reference_handles_current_period_leap_day_and_invalid_dates() {
    let mut state = FinanceOverviewState {
        period: 1,
        month: "2024-02".into(),
        ..Default::default()
    };
    assert_eq!(
        Some("2024-02-29".into()),
        finance_overview_reference(&state, "2026-09-15")
    );
    state.month = "2026-09".into();
    assert_eq!(
        Some("2026-09-15".into()),
        finance_overview_reference(&state, "2026-09-15")
    );
    state.period = 2;
    state.year = "2026".into();
    state.quarter = 3;
    assert_eq!(
        Some("2026-09-15".into()),
        finance_overview_reference(&state, "2026-09-15")
    );
    state.quarter = 0;
    assert!(finance_overview_reference(&state, "2026-09-15").is_none());
    state.period = 0;
    state.date = "2026-02-29".into();
    assert!(finance_overview_reference(&state, "2026-09-15").is_none());
}

#[test]
fn parity_finance_overview_shares_android_cutoffs_and_unknown_states() {
    let raw=json!({"dailyLedgers":{
        "2026-09-14":{"incomes":[{"id":"salary","name":"工资","kind":"ACTIVE","amount":1000}],"expenses":[{"id":"cost","name":"食品","bucket":"FOOD","amount":50}]},
        "2026-09-16":{"incomes":[{"id":"future","name":"未来","kind":"ACTIVE","amount":9000}],"expenses":[]}
    }}).to_string();
    let s = build_finance_overview(&raw, 1, "2026-09-15").unwrap();
    // JNI projects persisted whole-yuan fields and optional cents before calculation.
    let android_view = gridtimer_native::finance_money::project_profile_json(&raw).unwrap();
    assert_eq!(100_000, s.report.total_income);
    assert_eq!(5_000, s.report.total_outflow);
    assert_eq!(95_000, s.report.net_cashflow);
    assert!(!s.risk.forecast_available);
    assert!(!s.risk.has_fresh_snapshot);
    assert_eq!(
        gridtimer_native::build_finance_health_score_values(&android_view, 1, 2026, 9, 15),
        s.health
    );
    assert_eq!(
        gridtimer_native::build_finance_trend_values(&android_view, 1, 2026, 9, 15).unwrap(),
        s.trend
    );
    let empty = build_finance_overview("{}", 1, "2026-09-15").unwrap();
    assert!(empty.health.is_none());
    assert_eq!(DesktopFinanceRiskState::Unknown, empty.risk.risk_state);
    assert!(!empty.trend.cashflow_comparison_available);
    assert!(build_finance_overview(&raw, 1, "invalid").is_none());
}

#[test]
fn parity_risk_actions_open_android_destination_and_anomaly_day() {
    let dir = temp_test_dir("parity_risk_navigation");
    let mut client = knowledge_test_client(&dir);
    for kind in [2, 3, 6] {
        client.open_finance_risk_action(kind, "2026-08-31", "");
        assert_eq!(1, client.finance_workbench.tab);
        assert_eq!("2026-08", client.finance_workbench.month_key);
    }
    client.open_finance_risk_action(7, "2026-09-15", "2026-09-04");
    assert_eq!(0, client.finance_workbench.tab);
    assert_eq!("2026-09-04", client.finance_workbench.day_key);
    client.open_finance_risk_action(7, "2026-09-15", "bad");
    assert_eq!("2026-09-15", client.finance_workbench.day_key);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn parity_sharing_exports_selected_version_and_rejects_locked_or_missing_versions() {
    let dir = temp_test_dir("parity_share_identity");
    let mut client = knowledge_test_client(&dir);
    client.switch_tab(AppTab::Notes);
    client.select_note_by_id("sticky-a");
    client.set_note_canvas_text("历史正文");
    assert!(client.save_note());
    let version = client.selected_note().unwrap().latest_version_id.clone();
    client.create_note_product_version("");
    client.set_note_canvas_text("当前正文");
    assert!(client.save_note());
    assert!(client.open_note_version(&version));
    assert!(note_share_text(&client.note_sharing_snapshot().unwrap()).contains("历史正文"));
    client.selected_note_version_id = "missing-version".into();
    assert!(client.note_sharing_snapshot().is_err());
    client.selected_note_version_id.clear();
    client.note_crypto_password_draft = "synthetic-parity-password".into();
    client.enable_note_encryption();
    client.lock_note_encryption();
    assert!(client.selected_note_is_locked());
    assert!(client.note_sharing_snapshot().is_err());
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn parity_diagnostic_sessions_persist_and_do_not_include_user_content_or_secrets() {
    let dir = temp_test_dir("parity_diagnostics");
    let mut client = knowledge_test_client(&dir);
    client.sync.ai_api_key = "private-key-marker".into();
    client.sync.token = "private-token-marker".into();
    client.note_content_draft = "private-content-marker".into();
    client.set_diagnostic_recording(true).unwrap();
    client.collect_desktop_diagnostics();
    let session = client.desktop_ui.parity.diagnostics.session;
    let events = fs::read_to_string(
        client
            .diagnostic_directory()
            .join(format!("session_{session}.jsonl")),
    )
    .unwrap();
    for marker in [
        "private-key-marker",
        "private-token-marker",
        "private-content-marker",
    ] {
        assert!(!events.contains(marker));
    }
    client.desktop_ui.parity.diagnostics = DesktopDiagnosticCapture::default();
    client.collect_desktop_diagnostics();
    assert!(client.desktop_ui.parity.diagnostics.recording);
    assert_eq!(session, client.desktop_ui.parity.diagnostics.session);
    client.set_diagnostic_recording(false).unwrap();
    client.desktop_ui.parity.diagnostics = DesktopDiagnosticCapture::default();
    client.collect_desktop_diagnostics();
    assert!(!client.desktop_ui.parity.diagnostics.recording);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn parity_rich_to_text_keeps_recovery_content_and_encryption() {
    let dir = temp_test_dir("parity_plain_conversion");
    let mut client = knowledge_test_client(&dir);
    client.switch_tab(AppTab::Notes);
    client.select_note_by_id("sticky-a");
    let mut note = client.selected_note().unwrap();
    note.document.rich_text_enabled = true;
    note.document.rich_text_plain_text = "保留的正文".into();
    note.document.blocks = vec![new_desktop_text_block("<p><strong>保留的正文</strong></p>")];
    note.content = "保留的正文".into();
    assert!(client.replace_state(
        app_data::upsert_note_app_data_json(
            &client.state_json,
            &serde_json::to_string(&note).unwrap(),
            now_millis()
        ),
        "prepared"
    ));
    client.select_note_by_id("sticky-a");
    client.note_crypto_password_draft = "synthetic-conversion-password".into();
    client.enable_note_encryption();
    assert!(client.convert_selected_rich_note_to_text());
    let plain = client.selected_note().unwrap();
    assert!(!plain.document.rich_text_enabled);
    assert!(plain.content.contains("保留的正文"));
    assert!(plain.encryption.is_some());
    assert!(plain.revisions.iter().any(|r| r.document.rich_text_enabled));
    assert!(!client.state_json.contains("保留的正文"));
    client.lock_note_encryption();
    assert!(!client.convert_selected_rich_note_to_text());
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn parity_added_finance_pages_render_small_windows_without_changing_data() {
    let dir = temp_test_dir("parity_finance_views");
    let mut client = knowledge_test_client(&dir);
    client.switch_tab(AppTab::Finance);
    client.finance_workbench.tab = 5;
    let before = client.state_json.clone();
    let ctx = workspace_test_context();
    for page in 0..6 {
        client.finance_workbench.overview.page = page;
        workspace_test_frame(&mut client, &ctx, egui::vec2(760.0, 480.0), vec![]);
        assert_eq!(before, client.state_json);
        assert!(!client.finance_dirty);
    }
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(target_os = "windows")]
#[test]
#[ignore = "opens an isolated native window to validate tray hiding and timer continuity"]
fn native_parity_runtime_review() {
    use winit::platform::windows::EventLoopBuilderExtWindows;
    let output = PathBuf::from(
        std::env::var_os("TIMER_PARITY_RUNTIME_DIR").expect("runtime review output directory"),
    );
    fs::create_dir_all(&output).unwrap();
    let root = output.join(format!("profile-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let mut client = knowledge_test_client(&root);
    client.sync.token.clear();
    client.sync.server_url.clear();
    client.sync.ai_api_key.clear();
    client.settings.timer_bell_enabled = false;
    client.settings.close_to_tray = true;
    client.desktop_ui.slot_editor_focus = false;
    client.switch_tab(AppTab::Board);
    client.open_slot_editor(1);
    client.slot_title_draft = "专注任务".into();
    client.mark_slot_dirty();
    client.flush_slot_draft().unwrap();
    client.desktop_ui.slot_editor_open = false;
    let timer_ctx = workspace_test_context();
    client.toggle_slot(&client.selected_slot().unwrap(), &timer_ctx);
    wait_for_timer_action(&mut client);
    let profile = json!({"dailyLedgers":{
        "2026-09-14":{"incomes":[{"id":"salary","kind":"ACTIVE","name":"工资","amount":1200}],"expenses":[{"id":"food","bucket":"FOOD","name":"食品","amount":50}]},
        "2026-09-15":{"incomes":[],"expenses":[{"id":"travel","bucket":"LIVING","name":"交通","amount":20}]}
    },"monthlySnapshots":{"2026-09":{"assets":[{"id":"cash","name":"存款","kind":"CASH_RESERVE","amount":8000}],"liabilities":[],"confirmedAtEpochMillis":1}}});
    let next = app_data::update_finance_profile_app_data_json(
        &client.state_json,
        &profile.to_string(),
        now_millis(),
    );
    assert!(client.replace_state(next, "测试数据已就绪"));
    struct RuntimeReview {
        client: TimerWindowsClient,
        output: PathBuf,
        started: Instant,
        close_requested: bool,
        hidden_since: Option<Instant>,
        hidden_start_elapsed: Option<i64>,
        hidden_max_elapsed: i64,
        hidden_frames: usize,
        restored: bool,
    }
    impl eframe::App for RuntimeReview {
        fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
            self.client.update(ctx, frame);
            if !self.close_requested && self.started.elapsed() > Duration::from_secs(2) {
                // Exercise the same close handler as the native close button.
                ctx.input_mut(|input| {
                    input
                        .raw
                        .viewports
                        .get_mut(&egui::ViewportId::ROOT)
                        .unwrap()
                        .events
                        .push(egui::ViewportEvent::Close)
                });
                self.client.handle_close_request(ctx);
                self.close_requested = true;
                assert!(
                    self.client.desktop_ui.parity.tray.hidden,
                    "closing must keep the process in the notification area"
                );
            }
            if self.client.desktop_ui.parity.tray.hidden {
                self.hidden_frames += 1;
                let elapsed = self
                    .client
                    .desktop_ui
                    .projection
                    .as_ref()
                    .and_then(|p| p.slots.first())
                    .map(|s| s.accumulated_millis)
                    .unwrap_or_default();
                self.hidden_start_elapsed.get_or_insert(elapsed);
                self.hidden_max_elapsed = self.hidden_max_elapsed.max(elapsed);
                let since = self.hidden_since.get_or_insert_with(Instant::now);
                if since.elapsed() > Duration::from_secs(5) {
                    self.client.desktop_ui.parity.tray.hidden = false;
                    self.client
                        .activation_requested
                        .store(true, AtomicOrdering::Release);
                    self.restored = true;
                }
            }
            if self.restored && self.started.elapsed() > Duration::from_secs(10) {
                let report = json!({"nativeTray":self.client.desktop_ui.parity.tray.native.is_some(),"hiddenFrames":self.hidden_frames,
                    "hiddenTimerDeltaMillis":self.hidden_start_elapsed.map(|s|self.hidden_max_elapsed-s),"restored":self.restored,
                    "noteExport":self.client.transfers.last_note_export,"status":self.client.status,
                    "running":self.client.data.slots.iter().filter(|s|s.running_since_epoch_millis.is_some()).count(),"notes":self.client.data.notes.len()});
                fs::write(
                    self.output.join("runtime_status.json"),
                    serde_json::to_string_pretty(&report).unwrap(),
                )
                .unwrap();
                assert!(self.hidden_frames >= 5);
                assert!(
                    self.hidden_max_elapsed - self.hidden_start_elapsed.unwrap() >= 4_000,
                    "the timer must keep advancing while its native window is hidden"
                );
                self.client.desktop_ui.parity.tray.force_exit = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            assert!(
                self.started.elapsed() < Duration::from_secs(45),
                "tray continuity review timeout"
            );
        }
        fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
            self.client.on_exit(gl);
        }
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1240.0, 800.0]),
        event_loop_builder: Some(Box::new(|b| {
            b.with_any_thread(true);
        })),
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        "十倍率 · 功能验收",
        options,
        Box::new(move |cc| {
            install_ui_fonts(&cc.egui_ctx);
            install_ui_style(&cc.egui_ctx);
            client.start_desktop_tray(cc);
            Box::new(RuntimeReview {
                client,
                output,
                started: Instant::now(),
                close_requested: false,
                hidden_since: None,
                hidden_start_elapsed: None,
                hidden_max_elapsed: 0,
                hidden_frames: 0,
                restored: false,
            })
        }),
    )
    .unwrap();
}
