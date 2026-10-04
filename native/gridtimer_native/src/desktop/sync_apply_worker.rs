// v1.1.0.3 Windows - Serialize ordinary sync commits with local drafts off the frame thread.

struct DesktopSyncRepaintOnDrop(Option<egui::Context>);

impl Drop for DesktopSyncRepaintOnDrop {
    fn drop(&mut self) {
        if let Some(context) = self.0.as_ref() {
            context.request_repaint();
        }
    }
}

#[derive(Default)]
struct DesktopSyncApplyState {
    receiver: Option<mpsc::Receiver<DesktopSyncApplyCompletion>>,
    media_receiver: Option<mpsc::Receiver<DesktopMediaApplyCompletion>>,
    cleanup_receiver: Option<mpsc::Receiver<(AiWorkspaceIdentity, u64, Result<(), String>)>>,
    completed_sync: Option<(SyncTaskResult, Option<Vec<sync_core::SyncClientResult>>)>,
    completed_media: Option<(NoteMediaSyncTaskResult, bool)>,
    media_index_committed: bool,
    cancelled: bool,
    frame_context: Option<egui::Context>,
}

struct DesktopSyncApplyInput {
    workspace: AiWorkspaceIdentity,
    version: u64,
    state: String,
    sync: DesktopSyncSession,
    sync_path: PathBuf,
    secrets_path: PathBuf,
    task: SyncTaskResult,
}

struct DesktopSyncApplySaved {
    state: String,
    data: DesktopAppData,
    changes: DesktopContentChanges,
    projector: app_data::TimerProjector,
    sync: DesktopSyncSession,
    result: sync_core::SyncClientResult,
    warning: Option<String>,
    sync_files_saved: bool,
}

struct DesktopSyncApplyCompletion {
    workspace: AiWorkspaceIdentity,
    version: u64,
    task: SyncTaskResult,
    outcome: Result<Option<DesktopSyncApplySaved>, String>,
}

fn sync_apply_identity_matches(
    sync: &DesktopSyncSession,
    expected: &SyncResultIdentityExpectation,
) -> bool {
    let SyncResultIdentityExpectation::BoundAccount {
        requested_server_url,
        ..
    } = expected
    else {
        return false;
    };
    let route_matches = normalize_credential_server_url(requested_server_url)
        .zip(normalize_credential_server_url(&sync.server_url))
        .is_some_and(|(requested, current)| requested == current);
    route_matches && sync_matches_bound_expectation(sync, expected)
}

fn media_apply_identity(sync: &DesktopSyncSession) -> SyncResultIdentityExpectation {
    let checkpoint = workspace_checkpoint_for_account(
        sync,
        &sync.server_instance_id,
        &sync.account_namespace,
        &sync.user_id,
    );
    // Keep only authorization scope, not the editable model/device settings or
    // credential plaintext. The token hash is checked against the live token.
    SyncResultIdentityExpectation::BoundAccount {
        requested_server_url: sync.server_url.clone(),
        user_id: sync.user_id.clone(),
        token_id: sync.token_id.clone(),
        server_instance_id: sync.server_instance_id.clone(),
        account_namespace: sync.account_namespace.clone(),
        workspace_id: checkpoint.workspace_id,
        workspace_proof: checkpoint.workspace_capability,
        acknowledged_generation: sync.acknowledged_generation,
        operation: BoundSyncOperation::MergeSync,
    }
}

fn prepare_ordinary_sync_commit(
    input: &DesktopSyncApplyInput,
    cancellation: &CancellationToken,
) -> Result<Option<DesktopSyncApplySaved>, String> {
    // Authentication, explicit restore and capability recovery keep their
    // separate state machines. Only an ordinary, already-bound merge enters
    // this path; its full response validation still occurs on the worker.
    if input.task.kind != SyncTaskKind::Sync || input.task.results.len() != 1 {
        return Ok(None);
    }
    let step = &input.task.results[0];
    if !matches!(
        step.identity,
        SyncResultIdentityExpectation::BoundAccount {
            operation: BoundSyncOperation::MergeSync,
            ..
        }
    ) {
        return Ok(None);
    }
    if !sync_apply_identity_matches(&input.sync, &step.identity) {
        return Err("同步保存的接收地址或账户凭据已变化，未写入本机数据".into());
    }
    let mut result: sync_core::SyncClientResult =
        serde_json::from_str(&step.raw).map_err(|_| "同步响应无法解析")?;
    validate_sync_result_identity(&result, &step.identity)?;
    if !result.ok || result.restore_required || result.workspace_identity_rebound {
        return Ok(None);
    }
    let Some(remote) = result.app_data_json.as_deref() else {
        return Ok(None);
    };
    if result.current_generation < input.sync.acknowledged_generation {
        return Err("服务端恢复代次倒退，已拒绝覆盖本机数据".into());
    }
    if cancellation.is_cancelled() {
        return Err("同步结果应用已取消，本机资料未改变".into());
    }
    let now = now_millis();
    let merged = sync_core::merge_sync_app_data_json(remote, &input.state, now)
        .and_then(|raw| {
            gridtimer_native::timer_sync::localize_snapshot_json(&raw, &input.state, now)
        })
        .and_then(|raw| app_data::sanitize_app_data_json(&raw, now))
        .ok_or("同步响应无法与本机最新编辑安全合并")?;
    let owner = desktop_state_owner(
        &input.sync.server_instance_id,
        &input.sync.account_namespace,
        &input.sync.user_id,
    )
    .map_err(|e| e.to_string())?;
    let store =
        open_desktop_state_store(&input.workspace.namespace_root).map_err(|e| e.to_string())?;
    let state = store
        .privacy_policy(&owner)
        .and_then(|(policy, _)| policy.including_snapshot(&merged))
        .and_then(|policy| policy.project_current_json(&merged))
        .map_err(|e| e.to_string())?
        .unwrap_or(merged);
    let previous = decode_data(&input.state);
    let data = decode_data(&state);
    let changes = DesktopContentChanges::between(&previous, &data);
    let projector = app_data::TimerProjector::parse(&state).map_err(|e| e.to_string())?;
    let mut sync = input.sync.clone();
    if !result.token.is_empty() {
        sync.token = result.token.clone();
    }
    if !result.token_id.is_empty() {
        sync.token_id = result.token_id.clone();
    }
    sync.acknowledged_generation = result.current_generation.max(0);
    sync.restore_receipt = result.restore_receipt.trim().to_owned();
    remember_account_generation_and_capability(
        &mut sync,
        &result.server_instance_id,
        &result.account_namespace,
        &result.user_id,
        result.current_generation.max(0),
        &result.workspace_id,
        &result.workspace_proof,
    );
    sync.last_message = desktop_sync_message(&result);
    sync.last_sync_at_epoch_millis = now;
    if cancellation.is_cancelled() {
        return Err("同步结果应用已取消，本机资料未改变".into());
    }
    let source = if result.mode == "baseline_required"
        && result.baseline_merge_required
        && result.current_generation == 0
    {
        "baseline_merge"
    } else {
        "server_merge"
    };
    let saved = save_state_snapshot_with_store(
        &input.workspace.namespace_root,
        &input.workspace.state_path,
        &owner,
        &state,
        &sync,
        now,
        source,
    )
    .map_err(|e| e.to_string())?;
    // Once the journal pair committed, always deliver its receipt even when
    // cancellation races the commit. Reverting the old checkpoint is unsafe.
    let mut warning = saved.warning().map(str::to_owned);
    let sync_files_result = save_sync_files(&input.sync_path, &input.secrets_path, &sync);
    let sync_files_saved = sync_files_result.is_ok();
    if let Err(error) = sync_files_result {
        let message = format!("资料已保存，同步状态副本待恢复：{error}");
        warning = Some(append_status(warning.unwrap_or_default(), message));
        result.ok = false;
    }
    if let Some(message) = warning.as_ref() {
        result.message = append_status(result.message, message.clone());
    }
    Ok(Some(DesktopSyncApplySaved {
        state,
        data,
        changes,
        projector,
        sync,
        result,
        warning,
        sync_files_saved,
    }))
}

impl TimerWindowsClient {
    fn external_workspace_save_pending(&self) -> bool {
        self.sync_apply.receiver.is_some()
            || self.sync_apply.media_receiver.is_some()
            || self.sync_apply.cleanup_receiver.is_some()
    }

    fn start_ordinary_sync_apply(&mut self, task: SyncTaskResult) {
        self.sync_apply.cancelled = false;
        let input = DesktopSyncApplyInput {
            workspace: self.ai_workspace_identity(),
            version: self.data_version,
            state: self.state_json.clone(),
            sync: self.sync.clone(),
            sync_path: self.sync_path.clone(),
            secrets_path: self.secrets_path.clone(),
            task,
        };
        let (sender, receiver) = mpsc::channel();
        let context = self.sync_apply.frame_context.clone();
        self.sync_apply.receiver = Some(receiver);
        if let Err(error) = self.task_supervisor.spawn(
            RuntimeTaskKind::Sync,
            TaskDurability::CommitSensitive,
            move |cancellation| {
                let outcome = prepare_ordinary_sync_commit(&input, &cancellation);
                let _ = sender.send(DesktopSyncApplyCompletion {
                    workspace: input.workspace,
                    version: input.version,
                    task: input.task,
                    outcome,
                });
                if let Some(ctx) = context {
                    ctx.request_repaint();
                }
            },
        ) {
            self.sync_apply.receiver = None;
            self.sync_task = None;
            self.status = format!("无法启动同步保存：{error}");
            self.cancel_waiting_timer_action("同步保存任务未能启动，请重新操作计时");
            let message = self.status.clone();
            self.reconcile_active_background_job(&message);
        }
    }

    fn poll_ordinary_sync_apply(&mut self) {
        let Some(receiver) = self.sync_apply.receiver.take() else {
            return;
        };
        let completion = match receiver.try_recv() {
            Ok(completion) => completion,
            Err(mpsc::TryRecvError::Empty) => {
                self.sync_apply.receiver = Some(receiver);
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.workspace_persistence_ready = false;
                self.sync_task = None;
                self.status = "同步保存结果无法确认，请重新打开客户端核验工作区".into();
                self.cancel_waiting_timer_action("同步保存结果无法确认，计时未改变");
                return;
            }
        };
        if completion.workspace != self.ai_workspace_identity()
            || completion.version != self.data_version
            || completion
                .task
                .results
                .first()
                .is_some_and(|step| !sync_apply_identity_matches(&self.sync, &step.identity))
            || (completion.task.results.is_empty() && matches!(&completion.outcome, Ok(Some(_))))
        {
            self.sync_task = None;
            self.workspace_persistence_ready = false;
            self.status = "同步保存回执与当前工作区不匹配，请重新打开核验".into();
            self.cancel_waiting_timer_action("工作区已改变，计时请求已取消");
            return;
        }
        match completion.outcome {
            Ok(Some(saved)) => {
                self.state_json = saved.state;
                let previous = std::mem::replace(&mut self.data, saved.data);
                self.data_version = self.data_version.saturating_add(1);
                self.apply_content_changes(saved.changes);
                // Settings edited during the background commit remain drafts.
                // Only the server-confirmed checkpoint/session fields move.
                self.sync.token = saved.sync.token;
                self.sync.token_id = saved.sync.token_id;
                self.sync.acknowledged_generation = saved.sync.acknowledged_generation;
                self.sync.restore_receipt = saved.sync.restore_receipt;
                self.sync.account_generation_checkpoints =
                    saved.sync.account_generation_checkpoints;
                self.sync.last_message = saved.sync.last_message;
                self.sync.last_sync_at_epoch_millis = saved.sync.last_sync_at_epoch_millis;
                self.sync_dirty |= !saved.sync_files_saved;
                if self.sync_dirty {
                    self.sync_save_due_epoch_millis =
                        save_clock_millis() + SYNC_SETTINGS_DEBOUNCE_MILLIS;
                }
                self.last_state_mirror_warning = saved.warning;
                self.refresh_clean_drafts_after_external_replace(&previous);
                if !self.finance_dirty {
                    self.finance_draft = self.data.finance_profile.clone();
                }
                let now = now_millis();
                self.desktop_ui.projection = Some(Arc::new(saved.projector.project(now)));
                self.desktop_ui.projector = Some(saved.projector);
                self.desktop_ui.projection_version = self.timers_cache_version();
                self.rebase_synced_interval_bell_markers(&previous, now);
                self.reconcile_bell_markers(now);
                self.status = desktop_sync_message(&saved.result);
                // Deallocate large replaced note/history trees away from input.
                let _ = self.task_supervisor.spawn(
                    RuntimeTaskKind::Sync,
                    TaskDurability::Ephemeral,
                    move |_| drop(previous),
                );
                if !self.sync_apply.cancelled {
                    self.resume_queued_timer_action();
                    self.sync_apply.completed_sync =
                        Some((completion.task, Some(vec![saved.result])));
                } else {
                    self.status = "同步已取消，已提交的本机资料已保留".into();
                }
            }
            Ok(None) => {
                if !self.sync_apply.cancelled {
                    self.sync_apply.completed_sync = Some((completion.task, None));
                    self.resume_queued_timer_action();
                }
            }
            Err(error) => {
                self.sync_task = None;
                self.status = format!("同步结果未写入：{error}");
                self.cancel_waiting_timer_action("前项同步保存失败，计时未改变，请重试");
                let message = self.status.clone();
                self.reconcile_active_background_job(&message);
            }
        }
    }

    fn poll_deferred_sync_finish(&mut self) {
        if self.sync_apply.cancelled {
            return;
        }
        if self.persistence.pending()
            || self.persistence.pending_timer_action.is_some()
            || self.external_workspace_save_pending()
        {
            return;
        }
        if let Some((task, prepared)) = self.sync_apply.completed_sync.take() {
            self.finish_received_sync_task(task, prepared);
        }
        if let Some((task, changed)) = self.sync_apply.completed_media.take() {
            self.sync_apply.media_index_committed = changed;
            self.finish_note_media_sync_task(task);
        }
    }
}

struct DesktopMediaApplyCompletion {
    workspace: AiWorkspaceIdentity,
    version: u64,
    identity: SyncResultIdentityExpectation,
    task: NoteMediaSyncTaskResult,
    outcome: Result<
        Option<(
            String,
            DesktopAppData,
            DesktopContentChanges,
            app_data::TimerProjector,
        )>,
        String,
    >,
}

impl TimerWindowsClient {
    fn start_synced_media_cleanup(&mut self) {
        if self.persistence.waiting_to_close
            || !self.workspace_persistence_ready
            || self.external_workspace_save_pending()
        {
            return;
        }
        let workspace = self.ai_workspace_identity();
        let version = self.data_version;
        let state = self.state_json.clone();
        let context = self.sync_apply.frame_context.clone();
        let (sender, receiver) = mpsc::channel();
        self.sync_apply.cleanup_receiver = Some(receiver);
        if let Err(error) = self.task_supervisor.spawn(
            RuntimeTaskKind::MediaSync,
            TaskDurability::CommitSensitive,
            move |cancellation| {
                let result = (|| -> Result<(), String> {
                    if cancellation.is_cancelled() {
                        return Ok(());
                    }
                    let references = workspace_retained_note_media_references(&state, &workspace)
                        .map_err(|e| e.to_string())?;
                    if !references.is_complete() {
                        return Err("附件引用尚未确认，已暂缓清理".into());
                    }
                    if cancellation.is_cancelled() {
                        return Ok(());
                    }
                    let store = note_media_store_for_workspace(&workspace)?;
                    let report = store
                        .cleanup_unreferenced_committed(&references.ids())
                        .map_err(|e| e.to_string())?;
                    if !report.is_clear() {
                        return Err("无引用附件清理尚未完成，已保留恢复标记".into());
                    }
                    Ok(())
                })();
                let _ = sender.send((workspace, version, result));
                if let Some(ctx) = context {
                    ctx.request_repaint();
                }
            },
        ) {
            self.sync_apply.cleanup_receiver = None;
            self.status = append_status(self.status.clone(), format!("附件清理暂缓：{error}"));
        }
    }

    fn poll_synced_media_cleanup(&mut self) {
        let Some(receiver) = self.sync_apply.cleanup_receiver.take() else {
            return;
        };
        match receiver.try_recv() {
            Ok((workspace, version, result)) => {
                if workspace == self.ai_workspace_identity() && version == self.data_version {
                    if let Err(error) = result {
                        self.status = append_status(self.status.clone(), error);
                    }
                    self.resume_queued_timer_action();
                } else {
                    self.cancel_waiting_timer_action("工作区已变化，等待中的计时请求已取消");
                }
            }
            Err(mpsc::TryRecvError::Empty) => self.sync_apply.cleanup_receiver = Some(receiver),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.status =
                    append_status(self.status.clone(), "附件清理已中断，保留恢复资料".into());
                self.resume_queued_timer_action();
            }
        }
    }

    fn start_media_result_apply(&mut self, mut task: NoteMediaSyncTaskResult) {
        self.sync_apply.cancelled = false;
        let workspace = self.ai_workspace_identity();
        let version = self.data_version;
        let state = self.state_json.clone();
        let sync = self.sync.clone();
        let identity = media_apply_identity(&sync);
        let context = self.sync_apply.frame_context.clone();
        let (sender, receiver) = mpsc::channel();
        self.sync_apply.media_receiver = Some(receiver);
        if let Err(error) = self.task_supervisor.spawn(
            RuntimeTaskKind::MediaSync,
            TaskDurability::CommitSensitive,
            move |cancellation| {
                let outcome = (|| -> Result<_, String> {
                    let summary = task.result.as_mut().map_err(|e| e.clone())?;
                    if cancellation.is_cancelled() {
                        return Err("附件索引写回已取消".into());
                    }
                    if !summary.verified_private_media.is_empty() {
                        persist_private_media_replies(
                            &workspace,
                            &state,
                            &sync,
                            &summary.verified_private_media,
                        )
                        .map_err(|e| e.to_string())?;
                        summary.verified_private_media.clear();
                    }
                    let (next, changed) = apply_note_media_sync_summary_tracked(
                        &state,
                        &summary.resolved_sha256_by_attachment_id,
                        &summary.remote_deleted_at_by_attachment_id,
                    )
                    .ok_or("附件索引无法解析")?;
                    if !changed {
                        // These indexes were checked against this exact state on
                        // the worker. Do not parse them again on the frame thread.
                        summary.resolved_sha256_by_attachment_id.clear();
                        summary.remote_deleted_at_by_attachment_id.clear();
                        return Ok(None);
                    }
                    let now = now_millis();
                    let next =
                        app_data::sanitize_app_data_json(&next, now).ok_or("附件索引无效")?;
                    let owner = desktop_state_owner(
                        &sync.server_instance_id,
                        &sync.account_namespace,
                        &sync.user_id,
                    )
                    .map_err(|e| e.to_string())?;
                    let store = open_desktop_state_store(&workspace.namespace_root)
                        .map_err(|e| e.to_string())?;
                    let next = store
                        .privacy_policy(&owner)
                        .and_then(|(policy, _)| policy.including_snapshot(&next))
                        .and_then(|policy| policy.project_current_json(&next))
                        .map_err(|e| e.to_string())?
                        .unwrap_or(next);
                    let previous = decode_data(&state);
                    let data = decode_data(&next);
                    let changes = DesktopContentChanges::between(&previous, &data);
                    let projector =
                        app_data::TimerProjector::parse(&next).map_err(|e| e.to_string())?;
                    if cancellation.is_cancelled() {
                        return Err("附件索引写回已取消".into());
                    }
                    let saved = save_state_snapshot_with_store(
                        &workspace.namespace_root,
                        &workspace.state_path,
                        &owner,
                        &next,
                        &sync,
                        now,
                        "local_save",
                    )
                    .map_err(|e| e.to_string())?;
                    if let Some(warning) = saved.warning() {
                        summary.failure_messages.push(warning.into());
                    }
                    summary.resolved_sha256_by_attachment_id.clear();
                    summary.remote_deleted_at_by_attachment_id.clear();
                    Ok(Some((next, data, changes, projector)))
                })();
                let _ = sender.send(DesktopMediaApplyCompletion {
                    workspace,
                    version,
                    identity,
                    task,
                    outcome,
                });
                if let Some(ctx) = context {
                    ctx.request_repaint();
                }
            },
        ) {
            self.sync_apply.media_receiver = None;
            self.sync_task = None;
            self.status = format!("无法启动附件索引保存：{error}");
            self.cancel_waiting_timer_action("附件索引保存任务未能启动，请重新操作计时");
            let message = self.status.clone();
            self.reconcile_active_background_job(&message);
        }
    }

    fn poll_media_result_apply(&mut self) {
        let Some(receiver) = self.sync_apply.media_receiver.take() else {
            return;
        };
        let completion = match receiver.try_recv() {
            Ok(value) => value,
            Err(mpsc::TryRecvError::Empty) => {
                self.sync_apply.media_receiver = Some(receiver);
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.workspace_persistence_ready = false;
                self.sync_task = None;
                self.status = "附件索引保存结果无法确认，请重新打开核验".into();
                self.cancel_waiting_timer_action("附件保存结果无法确认，计时未改变");
                return;
            }
        };
        if completion.workspace != self.ai_workspace_identity()
            || completion.version != self.data_version
            || !sync_apply_identity_matches(&self.sync, &completion.identity)
        {
            self.workspace_persistence_ready = false;
            self.sync_task = None;
            self.status = "附件保存回执与当前工作区不匹配，请重新打开核验".into();
            self.cancel_waiting_timer_action("工作区已改变，计时请求已取消");
            return;
        }
        match completion.outcome {
            Ok(saved) => {
                let changed = saved.is_some();
                if let Some((state, data, changes, projector)) = saved {
                    self.state_json = state;
                    let previous = std::mem::replace(&mut self.data, data);
                    self.data_version = self.data_version.saturating_add(1);
                    self.apply_content_changes(changes);
                    self.refresh_clean_drafts_after_external_replace(&previous);
                    self.desktop_ui.projection = Some(Arc::new(projector.project(now_millis())));
                    self.desktop_ui.projector = Some(projector);
                    self.desktop_ui.projection_version = self.timers_cache_version();
                    let _ = self.task_supervisor.spawn(
                        RuntimeTaskKind::MediaSync,
                        TaskDurability::Ephemeral,
                        move |_| drop(previous),
                    );
                }
                if !self.sync_apply.cancelled {
                    self.resume_queued_timer_action();
                    self.sync_apply.completed_media = Some((completion.task, changed));
                } else {
                    self.status = "同步已取消，已提交的附件索引已保留".into();
                }
            }
            Err(error) => {
                self.sync_task = None;
                self.status = format!("附件索引未完成写回：{error}");
                self.cancel_waiting_timer_action("前项附件保存失败，计时未改变，请重试");
                let message = self.status.clone();
                self.reconcile_active_background_job(&message);
            }
        }
    }
}
