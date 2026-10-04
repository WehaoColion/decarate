// v2.22.47 - Persist opt-in diagnostic sessions and export bounded, content-free evidence.

#[derive(Default)]
struct DesktopDiagnosticCapture {
    scope: PathBuf,
    loaded: bool,
    recording: bool,
    session: i64,
    sampled: Option<Instant>,
    last: Option<Value>,
}

fn desktop_diagnostic_metadata(client: &TimerWindowsClient) -> Value {
    json!({"version":WINDOWS_CLIENT_VERSION,"at":now_millis(),
        "tab":match client.tab {AppTab::Board=>0,AppTab::Notes=>1,AppTab::Knowledge=>2,AppTab::History=>3,AppTab::Finance=>4,AppTab::My=>5},
        "slots":client.data.slots.len(),"running":client.data.slots.iter().filter(|s|s.running_since_epoch_millis.is_some()).count(),
        "sessions":client.data.sessions.len(),"archives":client.data.archived_tasks.len(),"notes":client.data.notes.len(),
        "stateRevision":client.data_version,"pendingSaves":client.slot_dirty||client.note_dirty||client.finance_dirty,
        "backgroundTasks":client.task_supervisor.active_count(),"syncBusy":client.sync_task.is_some(),
        "stateStoreReady":client.state_path.is_file()})
}

impl TimerWindowsClient {
    fn diagnostic_directory(&self) -> PathBuf {
        self.state_path
            .parent()
            .unwrap_or(Path::new("."))
            .join("diagnostics")
    }

    fn collect_desktop_diagnostics(&mut self) {
        let directory = self.diagnostic_directory();
        let capture = &mut self.desktop_ui.parity.diagnostics;
        if !capture.loaded || capture.scope != directory {
            *capture = DesktopDiagnosticCapture {
                scope: directory.clone(),
                loaded: true,
                ..Default::default()
            };
            if let Ok(raw) = fs::read_to_string(directory.join("session.json")) {
                if let Ok(value) = serde_json::from_str::<Value>(&raw) {
                    capture.recording = value["recording"].as_bool().unwrap_or(false);
                    capture.session = value["session"].as_i64().unwrap_or_default();
                    if capture.session <= 0 {
                        capture.recording = false;
                    }
                }
            }
        }
        if !capture.recording
            || capture
                .sampled
                .is_some_and(|t| t.elapsed() < Duration::from_secs(2))
        {
            return;
        }
        capture.sampled = Some(Instant::now());
        let metadata = desktop_diagnostic_metadata(self);
        let capture = &mut self.desktop_ui.parity.diagnostics;
        let mut identity = metadata.clone();
        identity.as_object_mut().unwrap().remove("at");
        if capture.last.as_ref() == Some(&identity) {
            return;
        }
        let path = directory.join(format!("session_{}.jsonl", capture.session));
        // Keep the final recording state durable when a bounded session fills.
        if fs::metadata(&path).is_ok_and(|m| m.len() >= 1024 * 1024) {
            let _ = self.set_diagnostic_recording(false);
            self.status = "诊断收集已达 1 MB，已停止并保留记录".into();
            return;
        }
        let result = (|| -> io::Result<()> {
            fs::create_dir_all(&directory)?;
            let mut file = OpenOptions::new().create(true).append(true).open(path)?;
            writeln!(file, "{metadata}")?;
            file.flush()
        })();
        if result.is_ok() {
            capture.last = Some(identity);
        } else {
            self.status = "诊断记录写入失败".into();
        }
    }

    fn set_diagnostic_recording(&mut self, recording: bool) -> io::Result<()> {
        let directory = self.diagnostic_directory();
        fs::create_dir_all(&directory)?;
        let capture = &mut self.desktop_ui.parity.diagnostics;
        let session = if recording {
            now_millis().max(capture.session.saturating_add(1))
        } else {
            capture.session
        };
        atomic_replace_text_no_backup(
            &directory.join("session.json"),
            &json!({"recording":recording,"session":session}).to_string(),
        )?;
        capture.recording = recording;
        capture.session = session;
        capture.scope = directory;
        capture.loaded = true;
        capture.sampled = None;
        capture.last = None;
        Ok(())
    }

    fn ui_desktop_diagnostics(&mut self, ui: &mut egui::Ui) {
        self.ui_windows_update_history(ui);
        self.collect_desktop_diagnostics();
        card_frame().show(ui, |ui| {
            egui::CollapsingHeader::new("诊断").show(ui, |ui| {
                let recording = self.desktop_ui.parity.diagnostics.recording;
                ui.label(if recording {
                    "正在收集"
                } else {
                    "未收集"
                });
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .button(if recording {
                            "停止收集"
                        } else {
                            "开始收集"
                        })
                        .clicked()
                    {
                        match self.set_diagnostic_recording(!recording) {
                            Ok(()) => {
                                self.status = if recording {
                                    "已停止收集"
                                } else {
                                    "已开始收集"
                                }
                                .into()
                            }
                            Err(e) => self.status = format!("收集状态未保存：{e}"),
                        }
                    }
                    if ui.button("导出诊断").clicked() {
                        self.export_desktop_diagnostics();
                    }
                });
            });
        });
    }

    fn export_desktop_diagnostics(&mut self) {
        let capture = &self.desktop_ui.parity.diagnostics;
        let path = self
            .diagnostic_directory()
            .join(format!("session_{}.jsonl", capture.session));
        let mut events = Vec::<Value>::new();
        if let Ok(mut file) = File::open(path) {
            let mut raw = String::new();
            if std::io::Read::by_ref(&mut file)
                .take(1_048_576 + 4096)
                .read_to_string(&mut raw)
                .is_ok()
            {
                events = raw
                    .lines()
                    .filter_map(|line| serde_json::from_str(line).ok())
                    .collect();
            }
        }
        let report = json!({"format":"tenrate-desktop-diagnostics-v1","session":capture.session,
            "recording":capture.recording,"current":desktop_diagnostic_metadata(self),"events":events});
        match desktop_transfer_file_dialog(
            true,
            "导出诊断",
            &format!("diagnostics_{}.json", now_millis()),
            "json",
            "诊断记录",
        ) {
            Ok(Some(path)) => match write_transfer_text_verified(
                &path,
                &serde_json::to_string_pretty(&report).unwrap(),
            ) {
                Ok(()) => self.status = format!("诊断已导出：{}", path.display()),
                Err(e) => self.status = format!("导出失败：{e}"),
            },
            Ok(None) => {}
            Err(e) => self.status = e.to_string(),
        }
    }
}
include!("windows_update_history.rs");
