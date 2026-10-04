// v2.22.48 - Inspect timer records, archive navigation and Android risk labels.
// v2.22.47 - Inspect Android parity pages and render PNG exports in a real native window.
// Opt-in visual inspection only; this is not a pixel snapshot assertion.

fn parity_visual_finance_profile() -> Value {
    let mut ledgers = serde_json::Map::new();
    for (month, days, income) in [("2026-08", 31, 9000), ("2026-09", 15, 10000)] {
        for day in 1..=days {
            let incomes = if day == 1 {
                json!([{"id":format!("income-{month}"),"kind":"ACTIVE","name":"工资","amount":income}])
            } else {
                json!([])
            };
            ledgers.insert(format!("{month}-{day:02}"), json!({"incomes":incomes,"expenses":[{"id":format!("expense-{month}-{day}"),"bucket":"FOOD","name":"日常餐饮","amount":if day==12 {300} else {60}}],"confirmedAtEpochMillis":1}));
        }
    }
    json!({"dailyLedgers":ledgers,"monthlySnapshots":{
        "2026-08":{"assets":[{"id":"cash-aug","kind":"CASH_RESERVE","name":"存款","amount":16000}],"liabilities":[],"confirmedAtEpochMillis":1},
        "2026-09":{"assets":[{"id":"cash-sep","kind":"CASH_RESERVE","name":"存款","amount":20000}],"liabilities":[{"id":"debt-sep","kind":"LIABILITY_BALANCE","name":"分期余额","amount":3000}],"confirmedAtEpochMillis":1}
    }})
}

#[cfg(target_os = "windows")]
#[test]
#[ignore = "opens a native window for visual review"]
fn native_window_visual_review() {
    use winit::platform::windows::EventLoopBuilderExtWindows;
    let output = PathBuf::from(
        std::env::var_os("TIMER_VISUAL_REVIEW_DIR").expect("review output directory"),
    );
    fs::create_dir_all(&output).unwrap();
    let profile = output.join(format!("profile-{}", std::process::id()));
    fs::create_dir_all(&profile).unwrap();
    let now = now_millis();
    let mut state: Value = serde_json::from_str(&knowledge_fixture_state()).unwrap();
    let timer_history: Value = serde_json::from_str(&history_test_state(now)).unwrap();
    state["sessions"] = timer_history["sessions"].clone();
    state["archivedTasks"] = timer_history["archivedTasks"].clone();
    for (index, (title, note, elapsed)) in [
        (
            "完成产品方案",
            "梳理用户流程，整理本周的改进清单",
            5_042_000,
        ),
        (
            "阅读与摘录",
            "读完第三章，记下值得继续思考的问题",
            2_640_000,
        ),
        ("学习 Rust", "练习所有权与借用", 1_815_000),
        ("整理项目资料", "", 915_000),
        ("英语听力", "", 0),
        ("晚间复盘", "", 0),
    ]
    .into_iter()
    .enumerate()
    {
        let slot = &mut state["slots"][index];
        slot["title"] = json!(title);
        slot["note"] = json!(note);
        slot["accumulatedMillis"] = json!(elapsed);
    }
    if std::env::var_os("TIMER_VISUAL_FINANCE_DATA").is_some() {
        state["financeProfile"] = parity_visual_finance_profile();
    }
    let raw = app_data::sanitize_app_data_json(&state.to_string(), now).unwrap();
    let mut client = test_client_for_account_scope(&profile, "visual-review", raw);
    client.sync.token.clear();
    client.sync.server_url.clear();
    client.settings.timer_bell_enabled = false;
    client.status = "本地数据已就绪".into();
    struct ReviewApp {
        client: TimerWindowsClient,
        output: PathBuf,
        scene: usize,
        frames: usize,
        started: Instant,
    }
    const SCENES: &[(&str, AppTab, u8, f32, f32)] = &[
        ("board_light", AppTab::Board, 1, 1240.0, 800.0),
        ("board_running", AppTab::Board, 1, 1240.0, 800.0),
        ("board_dark", AppTab::Board, 2, 1240.0, 800.0),
        ("board_compact", AppTab::Board, 1, 760.0, 560.0),
        ("board_minimum", AppTab::Board, 1, 760.0, 480.0),
        ("timer_editor", AppTab::Board, 1, 1000.0, 720.0),
        ("board_empty", AppTab::Board, 1, 1240.0, 800.0),
        ("notes_light", AppTab::Notes, 1, 1240.0, 800.0),
        ("notes_dark", AppTab::Notes, 2, 1240.0, 800.0),
        ("notes_compact", AppTab::Notes, 1, 760.0, 560.0),
        ("notes_minimum", AppTab::Notes, 1, 760.0, 480.0),
        ("knowledge_light", AppTab::Knowledge, 1, 1240.0, 800.0),
        ("knowledge_dark", AppTab::Knowledge, 2, 1240.0, 800.0),
        ("history_light", AppTab::History, 1, 1240.0, 800.0),
        ("history_minimum", AppTab::History, 1, 760.0, 480.0),
        ("history_empty", AppTab::History, 1, 760.0, 480.0),
        ("history_detail_return", AppTab::History, 1, 1000.0, 720.0),
        ("archive_light", AppTab::History, 1, 1240.0, 800.0),
        ("finance_light", AppTab::Finance, 1, 1240.0, 800.0),
        ("finance_dark", AppTab::Finance, 2, 1240.0, 800.0),
        ("account_light", AppTab::My, 1, 1240.0, 800.0),
        ("knowledge_compact", AppTab::Knowledge, 1, 760.0, 560.0),
        ("knowledge_minimum", AppTab::Knowledge, 1, 760.0, 480.0),
        ("finance_compact", AppTab::Finance, 1, 760.0, 560.0),
        ("finance_minimum", AppTab::Finance, 1, 760.0, 480.0),
        ("account_oled", AppTab::My, 3, 1000.0, 720.0),
        ("finance_cockpit", AppTab::Finance, 1, 1240.0, 800.0),
        ("finance_budgets", AppTab::Finance, 1, 1240.0, 800.0),
        ("finance_forecast", AppTab::Finance, 1, 1240.0, 800.0),
        ("finance_health", AppTab::Finance, 1, 1240.0, 800.0),
        ("finance_snapshot", AppTab::Finance, 1, 1240.0, 800.0),
        ("finance_trend", AppTab::Finance, 1, 760.0, 560.0),
    ];
    impl eframe::App for ReviewApp {
        fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
            if self.scene >= SCENES.len() {
                self.client.poll_note_image_export(ctx, frame);
                if self.client.desktop_ui.parity.image_export.busy() {
                    ctx.request_repaint_after(Duration::from_millis(100));
                    return;
                }
                fs::write(self.output.join("export_status.json"),json!({"status":self.client.status,"path":self.client.transfers.last_note_export}).to_string()).unwrap();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            assert!(
                self.started.elapsed() < Duration::from_secs(150),
                "native capture timeout"
            );
            if self.frames > 0 {
                let images = ctx.input(|i| {
                    i.events
                        .iter()
                        .filter_map(|e| match e {
                            egui::Event::Screenshot { image, .. } => Some(image.clone()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                });
                for captured in images {
                    let pixels: Vec<u8> =
                        captured.pixels.iter().flat_map(|p| p.to_array()).collect();
                    image::save_buffer(
                        self.output.join(format!("{}.png", SCENES[self.scene].0)),
                        &pixels,
                        captured.width() as u32,
                        captured.height() as u32,
                        image::ColorType::Rgba8,
                    )
                    .unwrap();
                    self.scene += 1;
                    self.frames = 0;
                    if self.scene == SCENES.len() {
                        ctx.request_repaint();
                        return;
                    }
                }
            }
            let (_, tab, theme, width, height) = SCENES[self.scene];
            if self.frames == 0 {
                ACTIVE_DESKTOP_THEME.store(theme, AtomicOrdering::Relaxed);
                self.client.data.theme_mode = if theme == 1 { "LIGHT" } else { "DARK" }.into();
                self.client.data.oled_theme_enabled = theme == 3;
                install_ui_style(ctx);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(width, height)));
                self.client.switch_tab(tab);
                self.client.data.note_preferences.selected_folder_id = None;
                self.client.view_cache.data_version = u64::MAX;
                self.client.desktop_ui.slot_editor_open = false;
                self.client.desktop_ui.board_query.clear();
                if tab == AppTab::History {
                    self.client.switch_tab(AppTab::Board);
                    let detail = SCENES[self.scene].0 == "history_detail_return";
                    if detail {
                        self.client.open_slot_editor(1);
                    }
                    assert!(self.client.open_timer_history(
                        detail.then_some(1),
                        SCENES[self.scene].0 == "archive_light",
                    ));
                    if SCENES[self.scene].0 == "history_empty" {
                        self.client.desktop_ui.history_query = "不存在的记录".into();
                    }
                }
                if SCENES[self.scene].0 == "board_running" {
                    self.client
                        .toggle_slot(&self.client.selected_slot().unwrap(), ctx);
                }
                if SCENES[self.scene].0 == "timer_editor" {
                    self.client.open_slot_editor(1);
                }
                if SCENES[self.scene].0 == "board_empty" {
                    self.client.desktop_ui.board_query = "找不到的任务".into();
                }
                self.client.rebuild_view_cache();
                if tab == AppTab::Notes {
                    self.client.select_note_by_id("sticky-a");
                    if SCENES[self.scene].0 == "notes_light" {
                        let note = self.client.note_sharing_snapshot().unwrap();
                        let html = self.client.note_sharing_html(&note).unwrap();
                        self.client.desktop_ui.parity.image_export = NoteImageExport {
                            page: Some(note_image_page(&html)),
                            path: Some(self.output.join("note_export.png")),
                            workspace: self.client.background_job_workspace_fingerprint(),
                            started: Some(Instant::now()),
                            ..Default::default()
                        };
                    }
                }
                if tab == AppTab::Knowledge {
                    self.client.select_note_by_id("doc-a");
                }
                if let Some(page) = [
                    "finance_cockpit",
                    "finance_budgets",
                    "finance_forecast",
                    "finance_health",
                    "finance_snapshot",
                    "finance_trend",
                ]
                .iter()
                .position(|name| *name == SCENES[self.scene].0)
                {
                    self.client.finance_workbench.tab = 5;
                    self.client.finance_workbench.overview.page = page;
                    self.client.finance_workbench.overview.date = "2026-09-15".into();
                    self.client.finance_workbench.overview.month = "2026-09".into();
                    self.client.finance_workbench.overview.year = "2026".into();
                }
            }
            self.client.poll_note_image_export(ctx, frame);
            self.client.refresh_timer_projection(now_millis());
            ctx.input_mut(|i| i.pointer = Default::default());
            self.client.ui_workspace(ctx);
            self.frames += 1;
            if self.frames == 8 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
            }
            ctx.request_repaint_after(Duration::from_millis(80));
        }
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1240.0, 800.0])
            .with_position([40.0, 40.0]),
        event_loop_builder: Some(Box::new(|builder| {
            builder.with_any_thread(true);
        })),
        persist_window: false,
        ..Default::default()
    };
    let expected_output = output.clone();
    eframe::run_native(
        "十倍率 · 界面检查",
        options,
        Box::new(move |cc| {
            install_ui_fonts(&cc.egui_ctx);
            Box::new(ReviewApp {
                client,
                output,
                scene: 0,
                frames: 0,
                started: Instant::now(),
            })
        }),
    )
    .unwrap();
    assert!(SCENES
        .iter()
        .all(|s| expected_output.join(format!("{}.png", s.0)).is_file()));
    let exported = image::open(expected_output.join("note_export.png"))
        .expect("native WebView PNG export must decode");
    assert_eq!(1080, exported.width());
    assert!(exported.height() > 128);
}
