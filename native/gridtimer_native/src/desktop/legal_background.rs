// Windows legal preparation and storage run outside the frame thread.
fn desktop_legal_text_page(text: &str, page: usize) -> (&str, usize) {
    const PAGE_BYTES: usize = 16 * 1024;
    let pages = text.len().div_ceil(PAGE_BYTES).max(1);
    let mut start = page.min(pages - 1) * PAGE_BYTES;
    let mut end = (start + PAGE_BYTES).min(text.len());
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[start..end], pages)
}

fn desktop_legal_long_text(ui: &mut egui::Ui, salt: impl std::hash::Hash, text: &str) {
    let id = ui.make_persistent_id(salt);
    let mut page = ui.data_mut(|data| data.get_temp::<usize>(id)).unwrap_or(0);
    let (_, pages) = desktop_legal_text_page(text, page);
    page = page.min(pages - 1);
    if pages > 1 {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(page > 0, egui::Button::new("上一段"))
                .clicked()
            {
                page -= 1;
            }
            ui.label(format!("第 {} / {} 段", page + 1, pages));
            if ui
                .add_enabled(page + 1 < pages, egui::Button::new("下一段"))
                .clicked()
            {
                page += 1;
            }
        });
        ui.data_mut(|data| data.insert_temp(id, page));
    }
    ui.label(desktop_legal_text_page(text, page).0);
}

fn desktop_legal_attachment_inputs(
    data: &DesktopAppData,
    unlocked: &std::collections::HashMap<String, DesktopNote>,
    identity: &AiWorkspaceIdentity,
    active: &impl Fn() -> bool,
) -> Vec<Value> {
    let mut result = Vec::new();
    let Ok(Some(store)) = probe_existing_scope_note_media_store(identity) else {
        return result;
    };
    let mut seen = HashSet::new();
    for stored in &data.notes {
        if !active() {
            break;
        }
        if stored.deleted_at_epoch_millis.is_some() {
            continue;
        }
        let note = if stored.encryption.is_some() {
            let Some(note) = unlocked.get(&stored.id) else {
                continue;
            };
            note
        } else {
            stored
        };
        let attachments = note
            .attachments
            .iter()
            .chain(
                note.revisions
                    .iter()
                    .flat_map(|revision| revision.attachments.iter()),
            )
            .chain(
                note.versions
                    .iter()
                    .filter(|version| version.deleted_at_epoch_millis.is_none())
                    .flat_map(|version| version.attachments.iter()),
            );
        for item in attachments {
            if !active() {
                return result;
            }
            if item.sha256.is_empty()
                || item.size_bytes <= 0
                || !seen.insert((note.id.clone(), item.id.clone(), item.sha256.clone()))
            {
                continue;
            }
            let Ok(path) = store.verified_blob_path(&item.id, &item.sha256, item.size_bytes) else {
                continue;
            };
            result.push(json!({"noteId": note.id, "attachmentId": item.id,
                "sha256": item.sha256, "path": path, "mimeType": item.mime_type,
                "sizeBytes": item.size_bytes}));
        }
    }
    result
}

impl TimerWindowsClient {
    fn cancel_desktop_legal_tasks(&mut self) {
        let state = &mut self.desktop_ui.legal_risk;
        state.cancel.store(true, AtomicOrdering::Release);
        state.sync_cancel.store(true, AtomicOrdering::Release);
        state.store_cancel.store(true, AtomicOrdering::Release);
        state.waiting_for_save = false;
        state.preview = None;
        state.selected_report = None;
        state.preview_version = None;
        state.unlocked_notes.clear();
        state.unlock_password.clear();
        state.store_queue.clear();
        state.sync_pending = false;
        self.task_supervisor.cancel_kinds(&[
            RuntimeTaskKind::LegalPreparation,
            RuntimeTaskKind::LegalScan,
            RuntimeTaskKind::LegalStore,
            RuntimeTaskKind::LegalSync,
        ]);
    }

    fn poll_desktop_legal_tasks(&mut self, ctx: &egui::Context) {
        self.ensure_legal_scope();
        self.cancel_changed_legal_scan();
        self.poll_legal_prepared();
        self.poll_legal_risk();
        self.poll_legal_store(ctx);
        if self.background_work_is_allowed() {
            self.advance_legal_preparation(ctx);
            self.poll_legal_report_sync(ctx);
        }
    }

    fn poll_legal_prepared(&mut self) {
        let Some(rx) = self.desktop_ui.legal_risk.prepared_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(outcome) => {
                self.desktop_ui.legal_risk.preparing_version = None;
                if outcome.scope != self.legal_scope()
                    || outcome.version != self.data_version
                    || outcome.cancellation.load(AtomicOrdering::Acquire)
                    || !Arc::ptr_eq(&outcome.cancellation, &self.desktop_ui.legal_risk.cancel)
                    || self.note_dirty
                    || self.slot_dirty
                    || self.finance_dirty
                    || self.theme_dirty
                    || self
                        .desktop_ui
                        .legal_risk
                        .cancel
                        .load(AtomicOrdering::Acquire)
                {
                    self.desktop_ui.legal_risk.message =
                        "资料已变化或准备已取消，请重新核对扫描范围".into();
                    return;
                }
                match outcome.result {
                    Ok(prepared) => {
                        self.desktop_ui.legal_risk.preview = Some(prepared);
                        self.desktop_ui.legal_risk.preview_version = Some(outcome.version);
                        self.desktop_ui.legal_risk.preview_digest = outcome.digest;
                        self.desktop_ui.legal_risk.selected_evidence_id.clear();
                        self.desktop_ui.legal_risk.message =
                            "扫描范围已固定，请核对发送清单".into();
                    }
                    Err(error) => self.desktop_ui.legal_risk.message = error,
                }
            }
            Err(mpsc::TryRecvError::Empty) => self.desktop_ui.legal_risk.prepared_rx = Some(rx),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.desktop_ui.legal_risk.preparing_version = None;
                self.desktop_ui.legal_risk.message = "扫描准备意外中断".into();
            }
        }
    }

    fn poll_legal_store(&mut self, ctx: &egui::Context) {
        if let Some(rx) = self.desktop_ui.legal_risk.store_rx.take() {
            match rx.try_recv() {
                Ok(outcome) if outcome.scope == self.legal_scope() => {
                    self.desktop_ui.legal_risk.store_durable = false;
                    let current = !outcome.cancellation.load(AtomicOrdering::Acquire)
                        && Arc::ptr_eq(
                            &outcome.cancellation,
                            &self.desktop_ui.legal_risk.store_cancel,
                        );
                    let report_current = current
                        && outcome.report_revision == self.desktop_ui.legal_risk.reports_revision;
                    match outcome.result {
                        Ok(DesktopLegalStoreValue::List(reports)) if report_current => {
                            self.desktop_ui.legal_risk.reports = reports;
                        }
                        Ok(DesktopLegalStoreValue::Read(id, report)) => {
                            if report_current
                                && self.desktop_ui.legal_risk.open
                                && id == self.desktop_ui.legal_risk.selected_report_id
                            {
                                self.desktop_ui.legal_risk.selected_report = Some(report);
                            }
                        }
                        Ok(DesktopLegalStoreValue::Deleted(id)) => {
                            // A successful durable commit remains valid even if cancellation
                            // raced its return; never replace newer metadata with an old list.
                            self.desktop_ui.legal_risk.reports_revision =
                                self.desktop_ui.legal_risk.reports_revision.wrapping_add(1);
                            self.desktop_ui
                                .legal_risk
                                .reports
                                .retain(|meta| meta.id != id);
                            if id == self.desktop_ui.legal_risk.selected_report_id {
                                self.desktop_ui.legal_risk.selected_report = None;
                            }
                            self.refresh_legal_reports();
                            self.queue_legal_report_sync();
                        }
                        Ok(DesktopLegalStoreValue::Unlocked(id, note)) => {
                            if current
                                && self.desktop_ui.legal_risk.open
                                && outcome.version == self.data_version
                            {
                                self.desktop_ui.legal_risk.unlocked_notes.insert(id, note);
                                self.desktop_ui.legal_risk.preview = None;
                                self.desktop_ui.legal_risk.preview_version = None;
                                self.desktop_ui.legal_risk.message =
                                    "已纳入本次扫描，关闭页面后会清除解锁内容".into();
                            }
                        }
                        Ok(DesktopLegalStoreValue::List(_)) if current => {
                            self.refresh_legal_reports()
                        }
                        Ok(DesktopLegalStoreValue::List(_)) => {}
                        Err(error) if current => self.desktop_ui.legal_risk.message = error,
                        Err(_) => {}
                    }
                }
                Ok(_) => {
                    self.desktop_ui.legal_risk.store_durable = false;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.desktop_ui.legal_risk.store_rx = Some(rx);
                    return;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.desktop_ui.legal_risk.store_durable = false;
                    self.desktop_ui.legal_risk.message = "报告读取任务意外中断".into()
                }
            }
        }
        if !self.background_work_is_allowed() || self.desktop_ui.legal_risk.sync_rx.is_some() {
            return;
        }
        let Some(request) = self.desktop_ui.legal_risk.store_queue.pop_front() else {
            return;
        };
        let scope = self.legal_scope();
        let version = self.data_version;
        let report_revision = self.desktop_ui.legal_risk.reports_revision;
        let path = self.state_path.clone();
        self.desktop_ui.legal_risk.store_durable =
            matches!(request, DesktopLegalStoreRequest::Delete(_));
        self.desktop_ui.legal_risk.store_cancel = Arc::new(AtomicBool::new(false));
        let cancellation = Arc::clone(&self.desktop_ui.legal_risk.store_cancel);
        let (tx, rx) = mpsc::channel();
        self.desktop_ui.legal_risk.store_rx = Some(rx);
        let wake = DesktopLegalTaskWake(ctx.clone());
        if let Err(error) = self.task_supervisor.spawn(
            RuntimeTaskKind::LegalStore,
            TaskDurability::CommitSensitive,
            move |token| {
                let _wake = wake;
                let result = (|| {
                    if token.is_cancelled() || cancellation.load(AtomicOrdering::Acquire) {
                        return Err("工作区操作已取消".into());
                    }
                    if let DesktopLegalStoreRequest::Unlock {
                        note,
                        password,
                        identity,
                    } = request
                    {
                        let id = note.id.clone();
                        let raw = serde_json::to_string(&note).map_err(|e| e.to_string())?;
                        let owner = desktop_state_owner(
                            &identity.server_instance_id,
                            &identity.account_namespace,
                            &identity.user_id,
                        )
                        .map_err(|e| e.to_string())?;
                        let crypto_scope =
                            gridtimer_native::desktop_state_store::desktop_note_session_scope(
                                &identity.namespace_root,
                                &owner,
                            )
                            .map_err(|e| e.to_string())?;
                        let (plain, session) = gridtimer_native::unlock_desktop_note_json_in_scope(
                            &raw,
                            &password,
                            &crypto_scope,
                        )
                        .ok_or("密码不正确或密封数据已损坏，笔记未覆盖")?;
                        let decoded =
                            serde_json::from_str::<DesktopNote>(&plain).map_err(|e| e.to_string());
                        let _ = gridtimer_native::close_desktop_note_session(&session);
                        let note = decoded?;
                        if note.id != id {
                            return Err("解锁内容不完整，笔记未覆盖".into());
                        }
                        return Ok(DesktopLegalStoreValue::Unlocked(id, note));
                    }
                    let store = DesktopLegalReportStore::open(&path, &scope)?;
                    match request {
                        DesktopLegalStoreRequest::List => {
                            store.list().map(DesktopLegalStoreValue::List)
                        }
                        DesktopLegalStoreRequest::Read(id) => store
                            .read(&id)
                            .map(|report| DesktopLegalStoreValue::Read(id, Arc::new(report))),
                        DesktopLegalStoreRequest::Delete(id) => {
                            if token.is_cancelled() || cancellation.load(AtomicOrdering::Acquire) {
                                return Err("工作区操作已取消".into());
                            }
                            store.delete(&id)?;
                            Ok(DesktopLegalStoreValue::Deleted(id))
                        }
                        DesktopLegalStoreRequest::Unlock { .. } => unreachable!(),
                    }
                })();
                let _ = tx.send(DesktopLegalStoreOutcome {
                    scope,
                    version,
                    report_revision,
                    cancellation,
                    result,
                });
            },
        ) {
            self.desktop_ui.legal_risk.store_rx = None;
            self.desktop_ui.legal_risk.store_durable = false;
            self.desktop_ui.legal_risk.message = format!("无法访问报告：{error}");
        }
    }
}
