// v1.1.0.7 Windows - Keep legal actions visible and require one-shot send confirmation.
#[path = "../desktop/legal_workflow.rs"]
mod desktop_legal_workflow;

include!("legal_report_store.rs");
include!("legal_report_sync.rs");
include!("legal_background.rs");

struct DesktopLegalScanOutcome {
    scope: String,
    version: u64,
    cancellation: Arc<AtomicBool>,
    saved_report_id: Option<String>,
    error: Option<String>,
}

struct DesktopLegalSyncOutcome {
    scope: String,
    revision: u64,
    result: Result<Vec<DesktopLegalReportMeta>, String>,
}

struct DesktopLegalPreparedOutcome {
    scope: String,
    version: u64,
    cancellation: Arc<AtomicBool>,
    digest: String,
    result: Result<Arc<gridtimer_native::legal_scan::LegalScanPrepared>, String>,
}

enum DesktopLegalStoreRequest {
    List,
    Read(String),
    Delete(String),
    Unlock {
        note: DesktopNote,
        password: zeroize::Zeroizing<String>,
        identity: AiWorkspaceIdentity,
    },
}

enum DesktopLegalStoreValue {
    List(Vec<DesktopLegalReportMeta>),
    Read(String, Arc<gridtimer_native::legal_scan::LegalReport>),
    Deleted(String),
    Unlocked(String, DesktopNote),
}

struct DesktopLegalStoreOutcome {
    scope: String,
    version: u64,
    report_revision: u64,
    cancellation: Arc<AtomicBool>,
    result: Result<DesktopLegalStoreValue, String>,
}

// Completion, including an unwinding worker, wakes the event loop exactly once.
struct DesktopLegalTaskWake(egui::Context);
impl Drop for DesktopLegalTaskWake {
    fn drop(&mut self) {
        self.0.request_repaint();
    }
}

fn desktop_legal_retry_millis(failures: u32) -> i64 {
    [60_000, 120_000, 240_000, 480_000, 900_000][failures.saturating_sub(1).min(4) as usize]
}

struct DesktopLegalRiskState {
    open: bool,
    scope: String,
    loaded_scope: String,
    preview: Option<Arc<gridtimer_native::legal_scan::LegalScanPrepared>>,
    preview_digest: String,
    preview_version: Option<u64>,
    preparing_version: Option<u64>,
    waiting_for_save: bool,
    prepared_rx: Option<mpsc::Receiver<DesktopLegalPreparedOutcome>>,
    store_queue: std::collections::VecDeque<DesktopLegalStoreRequest>,
    store_rx: Option<mpsc::Receiver<DesktopLegalStoreOutcome>>,
    store_cancel: Arc<AtomicBool>,
    store_durable: bool,
    unlocked_notes: std::collections::HashMap<String, DesktopNote>,
    unlock_note_id: String,
    unlock_password: String,
    show_contents: bool,
    selected_evidence_id: String,
    evidence_text_page: usize,
    running: bool,
    outcome_rx: Option<mpsc::Receiver<DesktopLegalScanOutcome>>,
    cancel: Arc<AtomicBool>,
    running_cancel: Option<Arc<AtomicBool>>,
    running_binding: Option<desktop_legal_workflow::SendBinding>,
    send_consent: Option<desktop_legal_workflow::SendConsent>,
    reports: Vec<DesktopLegalReportMeta>,
    reports_revision: u64,
    selected_report_id: String,
    selected_report: Option<Arc<gridtimer_native::legal_scan::LegalReport>>,
    message: String,
    sync_rx: Option<mpsc::Receiver<DesktopLegalSyncOutcome>>,
    sync_cancel: Arc<AtomicBool>,
    sync_last_attempt_at: i64,
    sync_normal_seen_at: i64,
    sync_revision: u64,
    sync_pending: bool,
    sync_message: String,
    sync_failures: u32,
    sync_retry_at: i64,
}

impl Default for DesktopLegalRiskState {
    fn default() -> Self {
        Self {
            open: false,
            scope: String::new(),
            loaded_scope: String::new(),
            preview: None,
            preview_digest: String::new(),
            preview_version: None,
            preparing_version: None,
            waiting_for_save: false,
            prepared_rx: None,
            store_queue: Default::default(),
            store_rx: None,
            store_cancel: Arc::new(AtomicBool::new(false)),
            store_durable: false,
            unlocked_notes: std::collections::HashMap::new(),
            unlock_note_id: String::new(),
            unlock_password: String::new(),
            show_contents: false,
            selected_evidence_id: String::new(),
            evidence_text_page: 0,
            running: false,
            outcome_rx: None,
            cancel: Arc::new(AtomicBool::new(false)),
            running_cancel: None,
            running_binding: None,
            send_consent: None,
            reports: Vec::new(),
            reports_revision: 0,
            selected_report_id: String::new(),
            selected_report: None,
            message: String::new(),
            sync_rx: None,
            sync_cancel: Arc::new(AtomicBool::new(false)),
            sync_last_attempt_at: 0,
            sync_normal_seen_at: 0,
            sync_revision: 0,
            sync_pending: true,
            sync_message: String::new(),
            sync_failures: 0,
            sync_retry_at: 0,
        }
    }
}

fn legal_source_tombstoned(
    data: &DesktopAppData,
    entity_type: &str,
    entity_id: &str,
    updated_at: i64,
) -> bool {
    data.extra
        .get("tombstones")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|item| {
                item.get("entityType").and_then(Value::as_str) == Some(entity_type)
                    && item.get("entityId").and_then(Value::as_str) == Some(entity_id)
                    && item
                        .get("deletedAtEpochMillis")
                        .and_then(Value::as_i64)
                        .is_some_and(|deleted_at| deleted_at > 0 && deleted_at >= updated_at)
            })
        })
}

fn legal_finance_entry_available(record: &Value, collection: &str, entry_id: &str) -> bool {
    record
        .get(collection)
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().enumerate().any(|(index, entry)| {
                let matches = entry
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .map_or_else(|| entry_id == index.to_string(), |id| entry_id == id);
                matches
                    && !entry
                        .get("deletedAtEpochMillis")
                        .and_then(Value::as_i64)
                        .is_some_and(|deleted_at| deleted_at > 0)
            })
        })
}

fn legal_source_available_in_data(data: &DesktopAppData, source_path: &str) -> bool {
    let segments = source_path.split('/').collect::<Vec<_>>();
    match segments.as_slice() {
        ["finance", "day", day] | ["finance", "day", day, _, _] => {
            let Some(record) = data
                .finance_profile
                .extra
                .get("dailyLedgers")
                .and_then(Value::as_object)
                .and_then(|items| items.get(*day))
            else {
                return false;
            };
            let revision = data
                .extra
                .get("financeDayLedgerRevisions")
                .and_then(|items| items.get(*day))
                .and_then(Value::as_i64)
                .unwrap_or(0);
            if legal_source_tombstoned(data, "financeDayLedger", day, revision) {
                return false;
            }
            if let [_, _, _, collection, entry_id] = segments.as_slice() {
                matches!(*collection, "incomes" | "expenses")
                    && legal_finance_entry_available(record, collection, entry_id)
            } else {
                true
            }
        }
        ["finance", "month", month] | ["finance", "month", month, _, _] => {
            let Some(record) = data
                .finance_profile
                .extra
                .get("monthlySnapshots")
                .and_then(Value::as_object)
                .and_then(|items| items.get(*month))
            else {
                return false;
            };
            let revision = data
                .extra
                .get("financeMonthSnapshotRevisions")
                .and_then(|items| items.get(*month))
                .and_then(Value::as_i64)
                .unwrap_or(0);
            if legal_source_tombstoned(data, "financeMonthSnapshot", month, revision) {
                return false;
            }
            if let [_, _, _, collection, entry_id] = segments.as_slice() {
                matches!(*collection, "assets" | "liabilities")
                    && legal_finance_entry_available(record, collection, entry_id)
            } else {
                true
            }
        }
        ["session", id] => data.sessions.iter().enumerate().any(|(index, item)| {
            let actual = if item.id.is_empty() {
                index.to_string()
            } else {
                item.id.clone()
            };
            actual == *id && !legal_source_tombstoned(data, "session", id, 0)
        }),
        ["archivedTask", id] => data.archived_tasks.iter().enumerate().any(|(index, item)| {
            let actual = if item.id.is_empty() {
                index.to_string()
            } else {
                item.id.clone()
            };
            actual == *id && !legal_source_tombstoned(data, "archivedTask", id, 0)
        }),
        _ => false,
    }
}

impl TimerWindowsClient {
    fn legal_snapshot_current(&self) -> bool {
        !self.slot_dirty
            && !self.note_dirty
            && !self.finance_dirty
            && !self.theme_dirty
            && !self
                .desktop_ui
                .legal_risk
                .cancel
                .load(AtomicOrdering::Acquire)
            && self.desktop_ui.legal_risk.preview_version == Some(self.data_version)
            && self.desktop_ui.legal_risk.scope == self.legal_scope()
    }

    fn cancel_changed_legal_scan(&mut self) {
        let changed_preparation =
            self.desktop_ui
                .legal_risk
                .preparing_version
                .is_some_and(|version| {
                    version != self.data_version
                        || self.note_dirty
                        || self.slot_dirty
                        || self.finance_dirty
                        || self.theme_dirty
                });
        let changed_send = self.desktop_ui.legal_risk.running
            && (self.workspace_edit_locked()
                || !self.legal_snapshot_current()
                || self
                    .desktop_ui
                    .legal_risk
                    .running_binding
                    .as_ref()
                    .is_some_and(|binding| self.legal_send_binding().as_ref() != Some(binding)));
        if changed_preparation || changed_send {
            self.desktop_ui
                .legal_risk
                .cancel
                .store(true, AtomicOrdering::Release);
            if let Some(cancellation) = &self.desktop_ui.legal_risk.running_cancel {
                cancellation.store(true, AtomicOrdering::Release);
            }
        }
    }

    fn poll_legal_report_sync(&mut self, ctx: &egui::Context) {
        let current_scope = self.legal_scope();
        let prior = self.desktop_ui.legal_risk.sync_rx.take();
        if let Some(rx) = prior {
            match rx.try_recv() {
                Ok(outcome) => {
                    if outcome.scope == current_scope
                        && outcome.revision == self.desktop_ui.legal_risk.sync_revision
                        && !self
                            .desktop_ui
                            .legal_risk
                            .sync_cancel
                            .load(AtomicOrdering::Acquire)
                    {
                        match outcome.result {
                            Ok(reports) => {
                                if outcome.revision == self.desktop_ui.legal_risk.sync_revision {
                                    self.desktop_ui.legal_risk.sync_pending = false;
                                }
                                self.desktop_ui.legal_risk.sync_message = "报告已同步".into();
                                self.desktop_ui.legal_risk.sync_failures = 0;
                                self.desktop_ui.legal_risk.sync_retry_at = 0;
                                self.desktop_ui.legal_risk.reports_revision =
                                    self.desktop_ui.legal_risk.reports_revision.wrapping_add(1);
                                self.desktop_ui.legal_risk.reports = reports;
                                if !self.desktop_ui.legal_risk.reports.iter().any(|meta| {
                                    meta.id == self.desktop_ui.legal_risk.selected_report_id
                                }) {
                                    self.desktop_ui.legal_risk.selected_report = None;
                                }
                            }
                            Err(error) => {
                                self.legal_sync_failed(error);
                            }
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.desktop_ui.legal_risk.sync_rx = Some(rx);
                    return;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.legal_sync_failed("连接意外中断".into());
                }
            }
            // Store polling precedes sync polling; give deferred local work its next frame.
            if !self.desktop_ui.legal_risk.store_queue.is_empty() {
                ctx.request_repaint();
            }
        }
        if !sync_identity_is_bound(&self.sync)
            || self.sync.token.is_empty()
            || self.sync.last_sync_at_epoch_millis <= 0
        {
            return;
        }
        if self.sync.last_sync_at_epoch_millis > self.desktop_ui.legal_risk.sync_normal_seen_at {
            self.desktop_ui.legal_risk.sync_normal_seen_at = self.sync.last_sync_at_epoch_millis;
            self.desktop_ui.legal_risk.sync_pending = true;
        }
        let now = now_millis();
        if self.persistence.waiting_to_close
            || !self.background_work_is_allowed()
            || !self.desktop_ui.legal_risk.sync_pending
            || self.sync_task.is_some()
            || self.note_media_sync_result_rx.is_some()
            || self.startup_sync_due_epoch_millis > 0
            || self.persistence.pending()
            || self.desktop_ui.legal_risk.store_rx.is_some()
            || !self.desktop_ui.legal_risk.store_queue.is_empty()
        {
            return;
        }
        if now < self.desktop_ui.legal_risk.sync_retry_at {
            ctx.request_repaint_after(Duration::from_millis(
                (self.desktop_ui.legal_risk.sync_retry_at - now) as u64,
            ));
            return;
        }
        let Some(checkpoint) = self
            .sync
            .account_generation_checkpoints
            .get(&self.sync.account_namespace)
            .cloned()
        else {
            return;
        };
        if checkpoint.workspace_id.is_empty() || checkpoint.workspace_capability.is_empty() {
            return;
        }
        let state_path = self.state_path.clone();
        let scope = current_scope.clone();
        let server_url = self.sync.server_url.clone();
        let token = self.sync.token.clone();
        let workspace_id = checkpoint.workspace_id;
        let proof = checkpoint.workspace_capability;
        let generation = self.sync.acknowledged_generation;
        self.desktop_ui.legal_risk.sync_cancel = Arc::new(AtomicBool::new(false));
        let cancellation = Arc::clone(&self.desktop_ui.legal_risk.sync_cancel);
        let revision = self.desktop_ui.legal_risk.sync_revision;
        let (tx, rx) = mpsc::channel();
        self.desktop_ui.legal_risk.sync_rx = Some(rx);
        self.desktop_ui.legal_risk.sync_last_attempt_at = now;
        self.desktop_ui.legal_risk.sync_message = "法律报告正在同步".into();
        let wake = DesktopLegalTaskWake(ctx.clone());
        if let Err(error) = self.task_supervisor.spawn(
            RuntimeTaskKind::LegalSync,
            TaskDurability::Recoverable,
            move |token_cancel| {
                let _wake = wake;
                let _peer_verification = desktop_sync_peer_guard();
                let result = DesktopLegalReportStore::open(&state_path, &scope).and_then(|store| {
                    sync_desktop_legal_reports_while(
                        &store,
                        &server_url,
                        &token,
                        &workspace_id,
                        &proof,
                        generation,
                        || {
                            !cancellation.load(AtomicOrdering::Acquire)
                                && !token_cancel.is_cancelled()
                        },
                    )?;
                    store.list()
                });
                let _ = tx.send(DesktopLegalSyncOutcome {
                    scope,
                    revision,
                    result,
                });
            },
        ) {
            self.desktop_ui.legal_risk.sync_rx = None;
            self.legal_sync_failed(error.to_string());
        }
    }

    fn legal_sync_failed(&mut self, error: String) {
        let state = &mut self.desktop_ui.legal_risk;
        state.sync_pending = true;
        state.sync_failures = state.sync_failures.saturating_add(1);
        state.sync_retry_at =
            now_millis().saturating_add(desktop_legal_retry_millis(state.sync_failures));
        state.sync_message = format!("报告待同步：{error}");
    }

    fn queue_legal_report_sync(&mut self) {
        self.desktop_ui
            .legal_risk
            .sync_cancel
            .store(true, AtomicOrdering::Release);
        self.desktop_ui.legal_risk.sync_revision =
            self.desktop_ui.legal_risk.sync_revision.wrapping_add(1);
        self.desktop_ui.legal_risk.sync_pending = true;
        self.desktop_ui.legal_risk.sync_message = if sync_identity_is_bound(&self.sync) {
            "报告待同步".into()
        } else {
            "访客报告仅保存在本机".into()
        };
    }

    fn legal_scope(&self) -> String {
        format!(
            "{}\0{}\0{}\0{}",
            self.state_path.display(),
            self.sync.server_instance_id,
            self.sync.account_namespace,
            self.sync.user_id
        )
    }

    fn refresh_legal_reports(&mut self) {
        if !self
            .desktop_ui
            .legal_risk
            .store_queue
            .iter()
            .any(|job| matches!(job, DesktopLegalStoreRequest::List))
        {
            self.desktop_ui
                .legal_risk
                .store_queue
                .push_back(DesktopLegalStoreRequest::List);
        }
    }

    fn ensure_legal_scope(&mut self) {
        let scope = self.legal_scope();
        if self.desktop_ui.legal_risk.scope != scope {
            self.cancel_desktop_legal_tasks();
            let open = self.desktop_ui.legal_risk.open;
            self.desktop_ui.legal_risk = DesktopLegalRiskState::default();
            self.desktop_ui.legal_risk.open = open;
            self.desktop_ui.legal_risk.scope = scope.clone();
        }
        if self.desktop_ui.legal_risk.loaded_scope != scope {
            self.refresh_legal_reports();
            self.desktop_ui.legal_risk.loaded_scope = scope;
        }
    }

    fn unlock_legal_note(&mut self) {
        if !self.legal_send_readiness().can_prepare() {
            self.desktop_ui.legal_risk.message = "扫描任务结束后才能加入新的解锁内容".into();
            return;
        }
        let id = self.desktop_ui.legal_risk.unlock_note_id.clone();
        let Some(note) = self
            .data
            .notes
            .iter()
            .find(|note| note.id == id && note.deleted_at_epoch_millis.is_none())
        else {
            self.desktop_ui.legal_risk.message = "这条笔记已不可用".into();
            return;
        };
        if note.encryption.is_none() {
            return;
        }
        let note = note.clone();
        let identity = self.ai_workspace_identity();
        let password = zeroize::Zeroizing::new(std::mem::take(
            &mut self.desktop_ui.legal_risk.unlock_password,
        ));
        self.desktop_ui
            .legal_risk
            .store_queue
            .push_back(DesktopLegalStoreRequest::Unlock {
                note,
                password,
                identity,
            });
        self.desktop_ui.legal_risk.message = "正在解锁笔记".into();
    }

    fn prepare_legal_risk(&mut self, ctx: &egui::Context) {
        self.ensure_legal_scope();
        if !self.legal_send_readiness().can_prepare() {
            self.desktop_ui.legal_risk.message = if self.desktop_ui.legal_risk.running {
                "分析仍在结束，请稍后重试".into()
            } else if self.desktop_ui.legal_risk.waiting_for_save
                || self.desktop_ui.legal_risk.prepared_rx.is_some()
            {
                "正在准备扫描范围，请等待完成或取消准备".into()
            } else {
                "当前工作区暂不能固定扫描快照".into()
            };
            return;
        }
        self.desktop_ui.legal_risk.send_consent = None;
        self.desktop_ui
            .legal_risk
            .cancel
            .store(true, AtomicOrdering::Release);
        self.desktop_ui.legal_risk.cancel = Arc::new(AtomicBool::new(false));
        self.desktop_ui.legal_risk.preview = None;
        self.desktop_ui.legal_risk.preview_version = None;
        self.desktop_ui.legal_risk.waiting_for_save = true;
        self.desktop_ui.legal_risk.message = "正在保存当前编辑并准备扫描范围".into();
        self.submit_draft_snapshot(ctx, true);
        ctx.request_repaint();
    }

    fn advance_legal_preparation(&mut self, ctx: &egui::Context) {
        if !self.desktop_ui.legal_risk.waiting_for_save {
            return;
        }
        if self
            .desktop_ui
            .legal_risk
            .cancel
            .load(AtomicOrdering::Acquire)
        {
            self.desktop_ui.legal_risk.waiting_for_save = false;
            return;
        }
        if self.persistence.pending() {
            return;
        }
        if self.persistence.failed() || !self.workspace_persistence_ready {
            self.desktop_ui.legal_risk.waiting_for_save = false;
            self.desktop_ui.legal_risk.message = "当前编辑未保存，无法固定扫描快照".into();
            return;
        }
        if self.note_dirty || self.slot_dirty || self.finance_dirty || self.theme_dirty {
            self.submit_draft_snapshot(ctx, false);
            return;
        }
        // Unlock jobs must finish before their plaintext can enter the frozen snapshot.
        if self.desktop_ui.legal_risk.store_rx.is_some()
            || !self.desktop_ui.legal_risk.store_queue.is_empty()
        {
            return;
        }
        self.desktop_ui.legal_risk.waiting_for_save = false;
        let scope = self.legal_scope();
        let version = self.data_version;
        let raw = self.state_json.clone();
        let identity = self.ai_workspace_identity();
        let unlocked = self.desktop_ui.legal_risk.unlocked_notes.clone();
        let cancellation = Arc::clone(&self.desktop_ui.legal_risk.cancel);
        let (tx, rx) = mpsc::channel();
        self.desktop_ui.legal_risk.prepared_rx = Some(rx);
        self.desktop_ui.legal_risk.preparing_version = Some(version);
        let wake = DesktopLegalTaskWake(ctx.clone());
        if let Err(error) = self.task_supervisor.spawn(
            RuntimeTaskKind::LegalPreparation,
            TaskDurability::Ephemeral,
            move |token| {
                let _wake = wake;
                let active =
                    || !token.is_cancelled() && !cancellation.load(AtomicOrdering::Acquire);
                let digest = format!("{:x}", Sha256::digest(raw.as_bytes()));
                let result = (|| {
                    if !active() {
                        return Err("扫描准备已取消".into());
                    }
                    let data =
                        serde_json::from_str::<DesktopAppData>(&raw).map_err(|e| e.to_string())?;
                    let attachments =
                        desktop_legal_attachment_inputs(&data, &unlocked, &identity, &active);
                    drop(data);
                    let unlocked_values = unlocked
                        .iter()
                        .filter_map(|(id, note)| {
                            serde_json::to_value(note).ok().and_then(|mut value| {
                                value
                                    .as_object_mut()?
                                    .insert("noteId".into(), Value::String(id.clone()));
                                Some(value)
                            })
                        })
                        .collect::<Vec<_>>();
                    let unlocked_json = json!(unlocked_values).to_string();
                    drop(unlocked);
                    let attachments_json = json!(attachments).to_string();
                    if !active() {
                        return Err("扫描准备已取消".into());
                    }
                    let prepared = gridtimer_native::legal_scan::prepare_scan(
                        &raw,
                        &format!("{:x}", Sha256::digest(scope.as_bytes())),
                        now_millis(),
                        &unlocked_json,
                        &attachments_json,
                    )?;
                    if !active() {
                        return Err("扫描准备已取消".into());
                    }
                    Ok(Arc::new(prepared))
                })();
                let _ = tx.send(DesktopLegalPreparedOutcome {
                    scope,
                    version,
                    cancellation,
                    digest,
                    result,
                });
            },
        ) {
            self.desktop_ui.legal_risk.prepared_rx = None;
            self.desktop_ui.legal_risk.preparing_version = None;
            self.desktop_ui.legal_risk.message = format!("无法准备扫描：{error}");
        }
    }

    fn legal_send_readiness(&self) -> desktop_legal_workflow::Readiness {
        let recipient = ai_client::legal_recipient_host(&self.sync.ai_base_url).ok();
        desktop_legal_workflow::Readiness {
            has_evidence: self
                .desktop_ui
                .legal_risk
                .preview
                .as_ref()
                .is_some_and(|scan| !scan.evidence.is_empty()),
            configured: !self.sync.ai_api_key.trim().is_empty()
                && !self.sync.ai_model.trim().is_empty()
                && recipient.is_some(),
            snapshot_current: self.legal_snapshot_current(),
            workspace_ready: self.workspace_persistence_ready
                && !self.workspace_edit_locked()
                && self.background_work_is_allowed(),
            preparing: self.desktop_ui.legal_risk.waiting_for_save
                || self.desktop_ui.legal_risk.prepared_rx.is_some(),
            running: self.desktop_ui.legal_risk.running,
        }
    }

    fn legal_send_binding(&self) -> Option<desktop_legal_workflow::SendBinding> {
        let prepared = self.desktop_ui.legal_risk.preview.as_ref()?;
        let scan = json!([
            self.desktop_ui.legal_risk.preview_digest,
            prepared.manifest.workspace_id,
            prepared.manifest.captured_at_epoch_millis,
            prepared.manifest.evidence_count,
            prepared.manifest.upload_bytes
        ]);
        let configuration = json!([
            self.sync.ai_base_url.trim(),
            self.sync.ai_model.trim(),
            format!(
                "{:x}",
                Sha256::digest(self.sync.ai_api_key.trim().as_bytes())
            )
        ]);
        Some(desktop_legal_workflow::SendBinding {
            workspace: self.legal_scope(),
            revision: self.data_version,
            scan_fingerprint: format!("{:x}", Sha256::digest(scan.to_string().as_bytes())),
            configuration_fingerprint: format!(
                "{:x}",
                Sha256::digest(configuration.to_string().as_bytes())
            ),
        })
    }

    fn request_legal_send_confirmation(&mut self) -> bool {
        if !self.legal_send_readiness().can_send() {
            self.desktop_ui.legal_risk.message = "请先核对扫描范围、AI 配置和当前资料".into();
            return false;
        }
        let Some(binding) = self.legal_send_binding() else {
            return false;
        };
        self.desktop_ui.legal_risk.send_consent =
            Some(desktop_legal_workflow::SendConsent::new(binding));
        true
    }

    fn start_legal_risk(&mut self, ctx: &egui::Context) {
        let Some(mut consent) = self.desktop_ui.legal_risk.send_consent.take() else {
            self.desktop_ui.legal_risk.message = "请先确认本次发送的接收方和资料范围".into();
            return;
        };
        let Some(binding) = self.legal_send_binding() else {
            return;
        };
        // Authorization is consumed before acquiring a worker or sending any request.
        let mut readiness = self.legal_send_readiness();
        readiness.snapshot_current = readiness.snapshot_current
            && format!("{:x}", Sha256::digest(self.state_json.as_bytes()))
                == self.desktop_ui.legal_risk.preview_digest;
        if !consent.consume(&binding, readiness) {
            self.desktop_ui.legal_risk.message = "请先核对扫描范围、AI 配置和当前资料".into();
            return;
        }
        let Some(prepared) = self.desktop_ui.legal_risk.preview.clone() else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        let scope = self.legal_scope();
        let state_path = self.state_path.clone();
        let version = self.data_version;
        let api_key = self.sync.ai_api_key.clone();
        let base_url = self.sync.ai_base_url.clone();
        let model = self.sync.ai_model.clone();
        let cancellation = Arc::clone(&self.desktop_ui.legal_risk.cancel);
        self.desktop_ui.legal_risk.running = true;
        self.desktop_ui.legal_risk.running_cancel = Some(Arc::clone(&cancellation));
        self.desktop_ui.legal_risk.running_binding = Some(binding);
        self.desktop_ui.legal_risk.outcome_rx = Some(rx);
        self.desktop_ui.legal_risk.message = "正在分析，取消后不再发送后续记录".into();
        let wake = DesktopLegalTaskWake(ctx.clone());
        if let Err(error) = self.task_supervisor.spawn(
            RuntimeTaskKind::LegalScan,
            TaskDurability::Recoverable,
            move |token| {
                let _wake = wake;
                let report = gridtimer_native::legal_scan::run_scan(
                    &prepared,
                    &api_key,
                    &base_url,
                    &model,
                    &gridtimer_native::legal_sources::verified_laws(),
                    |_, _| !cancellation.load(AtomicOrdering::Acquire) && !token.is_cancelled(),
                );
                let analysis_error = (!report.completed).then(|| {
                    format!(
                        "分析未完成：{}",
                        report
                            .errors
                            .first()
                            .map_or("请查看报告中的未覆盖项", String::as_str)
                    )
                });
                let id = new_legal_report_id();
                let saved = DesktopLegalReportStore::open(&state_path, &scope).and_then(|store| {
                    serde_json::to_vec(&report)
                        .map_err(|e| e.to_string())
                        .and_then(|raw| store.save(&id, now_millis(), &raw))
                });
                let _ = tx.send(DesktopLegalScanOutcome {
                    scope,
                    version,
                    cancellation,
                    saved_report_id: saved.as_ref().ok().map(|_| id),
                    error: saved.err().or(analysis_error),
                });
            },
        ) {
            self.desktop_ui.legal_risk.running = false;
            self.desktop_ui.legal_risk.running_cancel = None;
            self.desktop_ui.legal_risk.running_binding = None;
            self.desktop_ui.legal_risk.outcome_rx = None;
            self.desktop_ui.legal_risk.message = format!("无法启动分析：{error}");
        }
    }

    fn poll_legal_risk(&mut self) {
        let Some(rx) = self.desktop_ui.legal_risk.outcome_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(outcome) => {
                self.desktop_ui.legal_risk.running = false;
                self.desktop_ui.legal_risk.running_cancel = None;
                self.desktop_ui.legal_risk.running_binding = None;
                if outcome.scope == self.legal_scope() {
                    let current = outcome.version == self.data_version
                        && self.legal_snapshot_current()
                        && Arc::ptr_eq(&outcome.cancellation, &self.desktop_ui.legal_risk.cancel)
                        && !outcome.cancellation.load(AtomicOrdering::Acquire);
                    if current {
                        self.desktop_ui.legal_risk.message =
                            outcome.error.unwrap_or_else(|| "分析结果已保存".into());
                    }
                    self.refresh_legal_reports();
                    if let Some(id) = outcome.saved_report_id {
                        self.desktop_ui.legal_risk.reports_revision =
                            self.desktop_ui.legal_risk.reports_revision.wrapping_add(1);
                        if current && self.desktop_ui.legal_risk.open {
                            self.desktop_ui.legal_risk.selected_report_id = id;
                            self.load_selected_legal_report();
                        }
                        self.queue_legal_report_sync();
                    }
                }
            }
            Err(mpsc::TryRecvError::Empty) => self.desktop_ui.legal_risk.outcome_rx = Some(rx),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.desktop_ui.legal_risk.running = false;
                self.desktop_ui.legal_risk.running_cancel = None;
                self.desktop_ui.legal_risk.running_binding = None;
                self.desktop_ui.legal_risk.message = "分析任务意外中断".into();
            }
        }
    }

    fn load_selected_legal_report(&mut self) {
        let id = self.desktop_ui.legal_risk.selected_report_id.clone();
        self.desktop_ui.legal_risk.selected_report = None;
        self.desktop_ui
            .legal_risk
            .store_queue
            .retain(|job| !matches!(job, DesktopLegalStoreRequest::Read(_)));
        self.desktop_ui
            .legal_risk
            .store_queue
            .push_back(DesktopLegalStoreRequest::Read(id));
    }

    fn queue_legal_report_delete(&mut self, id: String) {
        self.desktop_ui
            .legal_risk
            .store_queue
            .push_back(DesktopLegalStoreRequest::Delete(id));
        // Stop subsequent upload batches immediately; serialize the durable deletion
        // after the current sync worker acknowledges cancellation.
        self.queue_legal_report_sync();
    }

    fn close_legal_risk(&mut self) {
        self.task_supervisor.cancel_kinds(&[
            RuntimeTaskKind::LegalPreparation,
            RuntimeTaskKind::LegalScan,
        ]);
        // A clicked deletion remains durable when only its page closes.
        if !self.desktop_ui.legal_risk.store_durable {
            self.desktop_ui
                .legal_risk
                .store_cancel
                .store(true, AtomicOrdering::Release);
        }
        self.desktop_ui
            .legal_risk
            .cancel
            .store(true, AtomicOrdering::Release);
        if let Some(cancellation) = &self.desktop_ui.legal_risk.running_cancel {
            cancellation.store(true, AtomicOrdering::Release);
        }
        self.desktop_ui.legal_risk.send_consent = None;
        self.desktop_ui.legal_risk.open = false;
        self.desktop_ui.legal_risk.preview = None;
        self.desktop_ui.legal_risk.selected_report = None;
        self.desktop_ui.legal_risk.preview_version = None;
        self.desktop_ui.legal_risk.waiting_for_save = false;
        self.desktop_ui.legal_risk.store_queue.retain(|job| {
            !matches!(
                job,
                DesktopLegalStoreRequest::Unlock { .. } | DesktopLegalStoreRequest::Read(_)
            )
        });
        self.desktop_ui.legal_risk.unlocked_notes.clear();
        self.desktop_ui.legal_risk.unlock_password.clear();
        self.finance_workbench.tab = 5;
    }

    fn open_legal_evidence(&mut self, source_path: &str) {
        let segments = source_path.split('/').collect::<Vec<_>>();
        if matches!(
            segments.as_slice(),
            ["finance", "day", ..]
                | ["finance", "month", ..]
                | ["session", ..]
                | ["archivedTask", ..]
        ) && !legal_source_available_in_data(&self.data, source_path)
        {
            self.desktop_ui.legal_risk.message =
                "接收设备没有这条原记录，暂时无法回跳；报告仍保留原始引文".into();
            return;
        }
        match segments.as_slice() {
            ["note", id, ..]
                if self
                    .data
                    .notes
                    .iter()
                    .any(|note| note.id == *id && note.deleted_at_epoch_millis.is_none()) =>
            {
                let note = self.data.notes.iter().find(|note| note.id == *id).unwrap();
                let document = note.kind.eq_ignore_ascii_case("DOCUMENT");
                self.close_legal_risk();
                if document {
                    self.switch_tab(AppTab::Knowledge);
                    self.open_knowledge_source(id);
                } else {
                    self.switch_tab(AppTab::Notes);
                    self.selected_note_id = (*id).to_string();
                    if segments.len() >= 4 && segments[2] == "versions" {
                        let _ = self.open_note_version(segments[3]);
                    }
                }
            }
            ["slot", id] => {
                if let Ok(id) = id.parse::<i32>() {
                    if self.data.slots.iter().any(|slot| slot.id == id) {
                        self.close_legal_risk();
                        self.switch_tab(AppTab::Board);
                        self.selected_slot_id = id;
                        return;
                    }
                }
                self.desktop_ui.legal_risk.message = "原任务已不可用，报告仍保留原始引文".into();
            }
            ["finance", "day", day, ..] => {
                self.close_legal_risk();
                self.finance_workbench.tab = 0;
                self.finance_workbench.day_key = (*day).to_string();
            }
            ["finance", "month", month, ..] => {
                self.close_legal_risk();
                self.finance_workbench.tab = 1;
                self.finance_workbench.month_key = (*month).to_string();
            }
            ["session", ..] | ["archivedTask", ..] => {
                self.close_legal_risk();
                self.switch_tab(AppTab::History);
            }
            _ => self.desktop_ui.legal_risk.message = "原记录已不可用，报告仍保留原始引文".into(),
        }
    }

    fn ui_legal_risk(&mut self, ui: &mut egui::Ui) {
        self.ensure_legal_scope();
        ui.horizontal(|ui| {
            section_heading(ui, "法律风险线索");
            if ui.small_button("返回风控").clicked() {
                self.close_legal_risk();
            }
        });
        if !self.desktop_ui.legal_risk.open {
            return;
        }
        ui.label(
            "按中国大陆法律寻找待核查线索。报告不是违法认定；未形成线索也不等于没有法律风险。",
        );
        if !self.desktop_ui.legal_risk.message.is_empty() {
            ui.label(
                egui::RichText::new(&self.desktop_ui.legal_risk.message).color(palette().muted),
            );
        }
        ui.add_space(10.0);
        let ready = self.legal_send_readiness();
        let mut prepare = false;
        let mut send = false;
        let mut cancel = false;
        let mut settings = false;
        ui.horizontal_wrapped(|ui| {
            prepare = ui
                .add_enabled(
                    ready.can_prepare(),
                    egui::Button::new(desktop_legal_workflow::PREPARE_ACTION_LABEL),
                )
                .clicked();
            if self.desktop_ui.legal_risk.preview.is_some() {
                send = ui
                    .add_enabled(
                        ready.can_send(),
                        egui::Button::new("发送并开始分析")
                            .fill(palette().accent)
                            .min_size(egui::vec2(170.0, 36.0)),
                    )
                    .clicked();
            }
            if ready.running {
                cancel = ui.button("停止后续发送").clicked();
            }
            settings = ui.small_button("AI 连接设置").clicked();
            if self.desktop_ui.legal_risk.preview.is_some() && !ready.configured {
                ui.colored_label(palette().warn, "请在“我的 → AI”填写连接配置");
            } else if self.desktop_ui.legal_risk.preview.is_some() && !ready.snapshot_current {
                ui.colored_label(palette().warn, "资料已变化，请重新整理");
            } else if self.desktop_ui.legal_risk.preview.is_some() && !ready.has_evidence {
                ui.colored_label(palette().warn, "没有可读取的资料，请查看未覆盖项");
            }
        });
        if prepare {
            self.prepare_legal_risk(ui.ctx());
        }
        if send {
            self.request_legal_send_confirmation();
        }
        if cancel {
            self.task_supervisor
                .cancel_kinds(&[RuntimeTaskKind::LegalScan]);
            if let Some(cancellation) = &self.desktop_ui.legal_risk.running_cancel {
                cancellation.store(true, AtomicOrdering::Release);
            }
            self.desktop_ui
                .legal_risk
                .cancel
                .store(true, AtomicOrdering::Release);
            self.desktop_ui.legal_risk.message = "已停止后续发送，正在等待在途请求结束".into();
        }
        if settings {
            self.close_legal_risk();
            self.switch_tab(AppTab::My);
            self.ai_settings_expanded = true;
            return;
        }
        if self.desktop_ui.legal_risk.waiting_for_save
            || self.desktop_ui.legal_risk.prepared_rx.is_some()
        {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("正在准备扫描范围");
                if ui.small_button("取消准备").clicked() {
                    self.desktop_ui
                        .legal_risk
                        .cancel
                        .store(true, AtomicOrdering::Release);
                    self.desktop_ui.legal_risk.waiting_for_save = false;
                }
            });
        }
        ui.separator();
        // Only details scroll; the preparation, send and cancel actions remain visible.
        egui::ScrollArea::vertical()
            .id_source("legal_risk_details")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.label("登录后，报告会保存到同步服务及备份。原始扫描资料在你确认发送后发给所示 AI 接口。");
                self.ui_legal_unlock(ui);
                if let Some(prepared) = self.desktop_ui.legal_risk.preview.clone() {
                    self.ui_legal_preview(ui, &prepared);
                }
                ui.add_space(18.0);
                self.ui_legal_reports(ui);
            });
    }

    fn ui_legal_send_confirmation(&mut self, ctx: &egui::Context) {
        if self.desktop_ui.legal_risk.send_consent.is_none() {
            return;
        }
        let current = self.legal_send_binding();
        let ready = self.legal_send_readiness().can_send()
            && current.as_ref().is_some_and(|binding| {
                self.desktop_ui
                    .legal_risk
                    .send_consent
                    .as_ref()
                    .is_some_and(|consent| consent.matches(binding))
            });
        let mut send = false;
        let mut cancel = false;
        egui::Window::new("确认本次 AI 分析")
            .id(egui::Id::new("legal_send_confirmation"))
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .collapsible(false)
            .resizable(false)
            .default_width(480.0)
            .show(ctx, |ui| {
                ui.label(format!(
                    "接收方：{}",
                    ai_client::legal_recipient_host(&self.sync.ai_base_url)
                        .unwrap_or_else(|_| "配置无效".into())
                ));
                ui.label(format!("接口：{}", self.sync.ai_base_url));
                ui.label(format!("模型：{}", self.sync.ai_model));
                if let Some(prepared) = &self.desktop_ui.legal_risk.preview {
                    ui.label(format!(
                        "{} 项可读记录，约 {:.2} MiB，预计最多 {} 次调用",
                        prepared.manifest.evidence_count,
                        prepared.manifest.upload_bytes as f64 / 1_048_576.0,
                        prepared.manifest.estimated_calls
                    ));
                    ui.label(format!(
                        "资料快照：{}",
                        desktop_local_timestamp(prepared.manifest.captured_at_epoch_millis)
                    ));
                }
                ui.label("可能包含本人及他人的个人信息。确认后会消耗接口额度；取消会停止后续记录，在途请求仍需结束。");
                if ai_client::legal_recipient_host(&self.sync.ai_base_url)
                    .is_ok_and(|host| host == "api.openai.com")
                {
                    ui.label("官方请求设置 store: false。");
                } else {
                    ui.label("服务的保存规则由接收方决定，请核对其数据规则。");
                }
                if !ready {
                    ui.colored_label(palette().warn, "资料、工作区或配置已变化，请取消并重新核对。");
                }
                ui.horizontal(|ui| {
                    cancel = ui.button("取消").clicked();
                    send = ui.add_enabled(ready, egui::Button::new("确认发送并分析")).clicked();
                });
            });
        if cancel {
            self.desktop_ui.legal_risk.send_consent = None;
        } else if send {
            self.start_legal_risk(ctx);
        }
    }

    fn ui_legal_unlock(&mut self, ui: &mut egui::Ui) {
        let locked = self
            .data
            .notes
            .iter()
            .filter(|note| note.encryption.is_some() && note.deleted_at_epoch_millis.is_none())
            .map(|note| (note.id.clone(), note.title.clone()))
            .collect::<Vec<_>>();
        if locked.is_empty() {
            return;
        }
        egui::CollapsingHeader::new(format!("加密笔记：{} 条", locked.len())).show(ui, |ui| {
            for (id, title) in locked {
                ui.horizontal(|ui| {
                    let unlocked = self.desktop_ui.legal_risk.unlocked_notes.contains_key(&id);
                    ui.label(format!(
                        "{}　{}",
                        title,
                        if unlocked {
                            "本次已解锁"
                        } else {
                            "未覆盖"
                        }
                    ));
                    if !unlocked && ui.small_button("逐条解锁").clicked() {
                        self.desktop_ui.legal_risk.unlock_note_id = id.clone();
                    }
                });
            }
            if !self.desktop_ui.legal_risk.unlock_note_id.is_empty() {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.desktop_ui.legal_risk.unlock_password)
                            .password(true)
                            .hint_text("此笔记密码"),
                    );
                    if ui
                        .add_enabled(
                            self.legal_send_readiness().can_prepare()
                                && self.desktop_ui.legal_risk.store_rx.is_none()
                                && self.desktop_ui.legal_risk.store_queue.is_empty(),
                            egui::Button::new("加入本次扫描"),
                        )
                        .clicked()
                    {
                        self.unlock_legal_note();
                    }
                });
            }
        });
    }

    fn ui_legal_preview(
        &mut self,
        ui: &mut egui::Ui,
        prepared: &gridtimer_native::legal_scan::LegalScanPrepared,
    ) {
        ui.add_space(12.0);
        card_frame().show(ui, |ui| {
            section_heading(ui, "发送前核对");
            let recipient = ai_client::legal_recipient_host(&self.sync.ai_base_url).ok();
            ui.label(format!(
                "接收域名：{}",
                recipient.as_deref().unwrap_or("地址未配置或无效")
            ));
            ui.label(format!("模型：{}", self.sync.ai_model));
            ui.label(format!(
                "记录 {} 项；预计上传 {:.2} MiB；预计最多 {} 次调用",
                prepared.manifest.evidence_count,
                prepared.manifest.upload_bytes as f64 / 1_048_576.0,
                prepared.manifest.estimated_calls
            ));
            for (category, coverage) in &prepared.manifest.coverage {
                ui.label(format!(
                    "{category}：已覆盖 {}，未覆盖 {}，已删除 {}",
                    coverage.included, coverage.unavailable, coverage.excluded_deleted
                ));
            }
            for omitted in &prepared.manifest.omissions {
                ui.label(format!(
                    "未覆盖 {}：{}",
                    omitted.source_path, omitted.reason
                ));
            }
            if ui
                .small_button(if self.desktop_ui.legal_risk.show_contents {
                    "收起发送内容"
                } else {
                    "查看发送内容"
                })
                .clicked()
            {
                self.desktop_ui.legal_risk.show_contents =
                    !self.desktop_ui.legal_risk.show_contents;
            }
            if self.desktop_ui.legal_risk.show_contents {
                egui::ScrollArea::vertical()
                    .id_source("legal_evidence_rows")
                    .max_height(240.0)
                    .show_rows(ui, 24.0, prepared.evidence.len(), |ui, rows| {
                        for index in rows {
                            let item = &prepared.evidence[index];
                            if ui
                                .selectable_label(
                                    self.desktop_ui.legal_risk.selected_evidence_id == item.id,
                                    format!("{} · {}", item.id, item.title),
                                )
                                .clicked()
                            {
                                self.desktop_ui.legal_risk.selected_evidence_id = item.id.clone();
                                self.desktop_ui.legal_risk.evidence_text_page = 0;
                            }
                        }
                    });
                if let Some(item) = prepared
                    .evidence
                    .iter()
                    .find(|item| item.id == self.desktop_ui.legal_risk.selected_evidence_id)
                {
                    let page = &mut self.desktop_ui.legal_risk.evidence_text_page;
                    let (_, pages) = desktop_legal_text_page(&item.text, *page);
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(*page > 0, egui::Button::new("上一段"))
                            .clicked()
                        {
                            *page -= 1;
                        }
                        ui.label(format!("第 {} / {} 段", *page + 1, pages));
                        if ui
                            .add_enabled(*page + 1 < pages, egui::Button::new("下一段"))
                            .clicked()
                        {
                            *page += 1;
                        }
                    });
                    ui.label(desktop_legal_text_page(&item.text, *page).0);
                    if item.image_data_url.is_some() {
                        ui.label("含图片输入");
                    }
                }
            }
            if !self.legal_snapshot_current() {
                ui.label("资料已有变化，请重新核对扫描范围");
            }
            if recipient
                .as_deref()
                .is_some_and(|host| host == "api.openai.com")
            {
                ui.label("官方接口请求设置 store: false。自定义服务的保存规则由服务提供方决定。");
            } else {
                ui.label("自定义服务可能保存请求内容，请核对其数据规则。");
            }
        });
    }

    fn ui_legal_reports(&mut self, ui: &mut egui::Ui) {
        section_heading(ui, "最近报告");
        if !self.desktop_ui.legal_risk.sync_message.is_empty() {
            ui.label(&self.desktop_ui.legal_risk.sync_message);
        }
        let reports = self.desktop_ui.legal_risk.reports.clone();
        if reports.is_empty() {
            ui.label("暂无报告");
            return;
        }
        for meta in reports {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "{} · {} · {} 条线索",
                    meta.created_at_epoch_millis,
                    if meta.completed {
                        "已完成"
                    } else {
                        "未完成"
                    },
                    meta.finding_count
                ));
                if ui.small_button("查看").clicked() {
                    self.desktop_ui.legal_risk.selected_report_id = meta.id.clone();
                    self.load_selected_legal_report();
                }
                if ui.small_button("删除").clicked() {
                    self.queue_legal_report_delete(meta.id.clone());
                }
            });
        }
        if let Some(report) = self.desktop_ui.legal_risk.selected_report.clone() {
            ui.add_space(10.0);
            card_frame().show(ui, |ui| {
                section_heading(
                    ui,
                    if report.completed {
                        "报告详情"
                    } else {
                        "未完成的报告"
                    },
                );
                for error in &report.errors {
                    ui.label(format!("未完成原因：{error}"));
                }
                if report.findings.is_empty() {
                    ui.label("本次已覆盖资料未形成待核查线索。仍需结合未覆盖资料和实际情况判断。");
                }
                for (index, finding) in report.findings.iter().enumerate() {
                    egui::CollapsingHeader::new(&finding.title)
                        .id_source((
                            "legal_report_finding",
                            &self.desktop_ui.legal_risk.selected_report_id,
                            index,
                        ))
                        .show(ui, |ui| {
                            ui.label(format!("问题领域：{}", finding.area));
                            ui.label("可能涉及的问题");
                            desktop_legal_long_text(ui, ("fact", index), &finding.fact);
                            if let Some(at) = finding.event_at_epoch_millis {
                                ui.label(format!("事发时间戳：{at}"));
                            }
                            for (quote_index, quote) in finding.evidence.iter().enumerate() {
                                ui.label(format!("{} · {}", quote.evidence_id, quote.title));
                                desktop_legal_long_text(
                                    ui,
                                    ("quote", index, quote_index),
                                    &quote.quote,
                                );
                                if ui.small_button("打开原记录").clicked() {
                                    self.open_legal_evidence(&quote.source_path);
                                }
                            }
                            ui.label("仍缺少的事实");
                            desktop_legal_long_text(ui, ("missing", index), &finding.missing_facts);
                            ui.label("处理建议");
                            desktop_legal_long_text(ui, ("advice", index), &finding.recommendation);
                            for law in &finding.laws {
                                ui.hyperlink_to(
                                    format!(
                                        "{} 第{}条（{}；核对于 {}）",
                                        law.title, law.article_number, law.version, law.checked_on
                                    ),
                                    &law.url,
                                );
                                ui.label(format!("原文摘录：{}", law.article_text));
                            }
                        });
                }
                for omission in &report.manifest.omissions {
                    ui.label(format!(
                        "未覆盖 {}：{}",
                        omission.source_path, omission.reason
                    ));
                }
            });
        }
    }
}

#[cfg(test)]
mod legal_risk_tests {
    use super::*;

    #[test]
    fn report_source_requires_an_existing_original_record() {
        let mut data = DesktopAppData::default();
        for path in [
            "finance/day/2026-09-01",
            "finance/month/2026-09",
            "session/s-1",
            "archivedTask/a-1",
        ] {
            assert!(!legal_source_available_in_data(&data, path));
        }
        data.finance_profile.extra.insert(
            "dailyLedgers".into(),
            json!({"2026-09-01":{"expenses":[
                {"id":"paid","name":"订金","amount":100},
                {"id":"deleted","name":"旧账","amount":1,"deletedAtEpochMillis":4}
            ]}}),
        );
        data.finance_profile.extra.insert(
            "monthlySnapshots".into(),
            json!({"2026-09":{"liabilities":[{"id":"loan","name":"借款","amount":500}]}}),
        );
        data.sessions.push(DesktopSession {
            id: "s-1".into(),
            ..Default::default()
        });
        data.archived_tasks.push(DesktopArchivedTask {
            id: "a-1".into(),
            ..Default::default()
        });
        for path in [
            "finance/day/2026-09-01/expenses/paid",
            "finance/month/2026-09/liabilities/loan",
            "session/s-1",
            "archivedTask/a-1",
        ] {
            assert!(legal_source_available_in_data(&data, path));
        }
        assert!(!legal_source_available_in_data(
            &data,
            "finance/day/2026-09-01/expenses/deleted"
        ));
        assert!(!legal_source_available_in_data(
            &data,
            "finance/day/2026-09-01/expenses/missing"
        ));
        data.extra.insert(
            "tombstones".into(),
            json!([{"entityType":"financeMonthSnapshot","entityId":"2026-09",
                "deletedAtEpochMillis":5}]),
        );
        assert!(!legal_source_available_in_data(
            &data,
            "finance/month/2026-09/liabilities/loan"
        ));
    }
}
