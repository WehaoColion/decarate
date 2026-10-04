// v1.1.0.2 Windows - Keep startup recovery off the window thread and open only verified data.
struct LoadedWorkspace {
    state_path: PathBuf,
    sync_path: PathBuf,
    settings_path: PathBuf,
    secrets_path: PathBuf,
    revocations_path: PathBuf,
    state_json: String,
    data: DesktopAppData,
    sync: DesktopSyncSession,
    settings: DesktopClientSettings,
    bell_markers: Vec<SlotBellMarker>,
    selected_slot_id: i32,
    finance_draft: DesktopFinanceProfile,
    pending_revocations: PendingTokenRevocationQueue,
    background_job_store: Option<DesktopBackgroundJobStore>,
    pending_reconciliation_jobs: Vec<BackgroundJobRecord>,
    workspace_persistence_ready: bool,
    startup_sync_due_epoch_millis: i64,
    messages: Vec<String>,
    now: i64,
}

type StartupStateHead = (
    DesktopStateStore,
    gridtimer_native::desktop_state_store::DesktopPrivacyPolicy,
    bool,
    Option<Option<gridtimer_native::desktop_state_store::DesktopStateSnapshot>>,
);

fn load_startup_state_head(root: &Path, owner: &str, now: i64) -> io::Result<StartupStateHead> {
    #[cfg(target_os = "windows")]
    if let Some((store, head)) = DesktopStateStore::open_existing_startup_workspace(
        &desktop_state_store_path(root),
        owner,
        now,
        |evidence| {
            verify_and_refresh_desktop_state_evidence_value(root, evidence)
                .map_err(|error| error.to_string())
        },
    )
    .map_err(desktop_state_store_io_error)?
    {
        return Ok((
            store,
            head.privacy_policy,
            head.owner_initialized,
            Some(head.latest),
        ));
    }
    // First initialization, missing-journal rejection, and old-format migration
    // retain their original marker/owner/privacy recovery path.
    let store = open_desktop_state_store(root)?;
    store
        .repair_privacy_history(owner)
        .map_err(desktop_state_store_io_error)?;
    let privacy_policy = store
        .privacy_policy(owner)
        .map_err(desktop_state_store_io_error)?
        .0;
    let initialized = store
        .owner_initialized(owner)
        .map_err(desktop_state_store_io_error)?;
    Ok((store, privacy_policy, initialized, None))
}

fn load_desktop_workspace(
    root_dir: PathBuf,
    control: &DesktopStartupControl,
) -> Result<LoadedWorkspace, StartupFailure> {
    #[cfg(target_os = "windows")]
    let _read_session =
        gridtimer_native::desktop_startup_read_session::DesktopStartupReadSession::enter();
    let legacy_state_path = root_dir.join("timer_state.json");
    let sync_path = root_dir.join("sync_account.json");
    let settings_path = root_dir.join("client_settings.json");
    let secrets_path = root_dir.join("sync_account.secrets");
    let revocations_path = root_dir.join("sync_account.revocations.secrets");
    let now = now_millis();
    control.checkpoint(1)?;
    let mut messages = Vec::new();
    let mut pending_reconciliation_jobs = Vec::new();
    let background_job_store =
        match DesktopBackgroundJobStore::open(root_dir.join(BACKGROUND_JOB_STORE_FILE)) {
            Ok(store) => {
                match store.recover_interrupted(now) {
                    Ok(recovered) if !recovered.is_empty() => {
                        messages.push(format!(
                            "检测到 {} 项上次未完成的后台同步，已安排重新核验",
                            recovered.len()
                        ));
                        pending_reconciliation_jobs = recovered;
                    }
                    Ok(_) => {}
                    Err(error) => messages.push(format!("后台同步恢复记录读取失败：{error}")),
                }
                if let Err(error) =
                    store.prune_terminal_before(now.saturating_sub(BACKGROUND_JOB_RETENTION_MILLIS))
                {
                    messages.push(format!("后台同步历史清理失败：{error}"));
                }
                Some(store)
            }
            Err(error) => {
                messages.push(format!("后台同步日志不可用，数据同步已保持禁用：{error}"));
                None
            }
        };
    control.checkpoint(2)?;
    let scope_media_recovery_ready = match recover_scope_media_transactions(&root_dir) {
        Ok(()) => true,
        Err(error) => {
            messages.push(format!(
                "跨账号媒体事务恢复失败；本地工作区保持只读：{error}"
            ));
            false
        }
    };
    control.checkpoint(3)?;
    let state_load = load_state_json(&legacy_state_path, now);
    if let Some(message) = state_load.message.clone() {
        messages.push(message);
    }
    let legacy_state_json = state_load.value;
    let secrets_rewrite_required = sync_secrets_need_secure_rewrite(&secrets_path);
    let (sync_load, sync_rewrite_required) = load_committed_sync_session(&sync_path, &secrets_path);
    if let Some(message) = sync_load.message.clone() {
        messages.push(message);
    }
    let mut sync = sync_load.value;
    let initial_acknowledged_generation = sync.acknowledged_generation;
    let initial_restore_receipt = sync.restore_receipt.clone();
    let initial_account_generation_checkpoints = sync.account_generation_checkpoints.clone();
    let revocations_load = load_pending_token_revocations(&revocations_path);
    if let Some(message) = revocations_load.message.clone() {
        messages.push(message);
    }
    let mut pending_revocations = revocations_load.value;
    if arm_committed_pending_revocations(&mut pending_revocations, &sync) {
        if let Err(error) = save_pending_token_revocations(&revocations_path, &pending_revocations)
        {
            messages.push(format!("待撤销凭据恢复失败：{error}"));
        }
    }
    let guest_state_path = state_path_for_user(&root_dir, "");
    if let Err(error) = copy_global_legacy_state_to_guest_if_needed(
        &legacy_state_path,
        &guest_state_path,
        &legacy_state_json,
        now,
    ) {
        messages.push(format!("访客数据兼容迁移失败：{error}"));
    }
    let scoped_state_path =
        state_path_for_account(&root_dir, &sync.account_namespace, &sync.user_id);
    if sync.account_namespace.trim().is_empty() {
        if let Err(error) =
            copy_legacy_scoped_state_if_needed(&root_dir, &sync.user_id, &scoped_state_path)
        {
            messages.push(format!("账号数据兼容迁移失败：{error}"));
        }
    }
    let state_path = scoped_state_path;
    let state_owner = desktop_state_owner(
        &sync.server_instance_id,
        &sync.account_namespace,
        &sync.user_id,
    );
    let mut workspace_persistence_ready = false;
    let mut protected_workspace_sync_state = Vec::new();
    control.checkpoint(4)?;
    let scoped_load = match state_owner.as_ref() {
        Ok(owner) => match load_state_snapshot_with_store(&root_dir, &state_path, owner, now) {
            Ok((loaded, protected_sync_state)) => {
                workspace_persistence_ready = scope_media_recovery_ready;
                protected_workspace_sync_state = protected_sync_state;
                loaded
            }
            Err(error) => {
                messages.push(format!(
                        "The verified local state journal is unavailable; this workspace is read-only: {error}"
                    ));
                load_state_json(&state_path, now)
            }
        },
        Err(error) => {
            messages.push(format!(
                    "The local workspace owner could not be verified; this workspace is read-only: {error}"
                ));
            load_state_json(&state_path, now)
        }
    };
    if let Some(message) = scoped_load.message {
        messages.push(message);
    }
    let state_json = scoped_load.value;
    if workspace_persistence_ready
        && !matches!(
            app_data::app_data_json_compatibility(&state_json, now),
            app_data::AppDataJsonCompatibility::LegacyMigratable
                | app_data::AppDataJsonCompatibility::CurrentKnown
        )
    {
        workspace_persistence_ready = false;
        messages.push(
            "The workspace uses an unsupported data schema; local writes and sync are disabled"
                .to_string(),
        );
    }
    if workspace_persistence_ready && !protected_workspace_sync_state.is_empty() {
        let expected_server_instance_id = sync.server_instance_id.clone();
        let expected_account_namespace = sync.account_namespace.clone();
        let expected_user_id = sync.user_id.clone();
        let recovered = decode_workspace_journal_sync_state(&protected_workspace_sync_state)
            .and_then(|state| {
                merge_workspace_journal_sync_state(
                    &mut sync,
                    &state,
                    &expected_server_instance_id,
                    &expected_account_namespace,
                    &expected_user_id,
                )
            });
        if let Err(error) = recovered {
            workspace_persistence_ready = false;
            messages.push(format!(
                    "The workspace sync checkpoint could not be verified; local writes and sync are disabled: {error}"
                ));
        }
    }
    if workspace_persistence_ready && sync_identity_namespace_is_stable(&sync) {
        let server_instance_id = sync.server_instance_id.clone();
        let account_namespace = sync.account_namespace.clone();
        let user_id = sync.user_id.clone();
        ensure_workspace_checkpoint(&mut sync, &server_instance_id, &account_namespace, &user_id);
        sync.acknowledged_generation =
            sync.acknowledged_generation
                .max(remembered_account_generation(
                    &sync,
                    &server_instance_id,
                    &account_namespace,
                    &user_id,
                ));
    }
    if workspace_persistence_ready
        && startup_workspace_requires_initialization_commit(&protected_workspace_sync_state, &sync)
    {
        // An unowned global legacy file is never inferred to belong to the
        // account that merely happens to be signed in during an upgrade.
        // The exact scoped snapshot and its sync checkpoint are committed
        // together before the UI is allowed to mutate either value.
        if let Some(error) = state_owner.as_ref().ok().and_then(|owner| {
            save_state_snapshot_with_store(
                &root_dir,
                &state_path,
                owner,
                &state_json,
                &sync,
                now,
                "workspace_initialization",
            )
            .err()
        }) {
            workspace_persistence_ready = false;
            messages.push(format!("账号数据初始化失败：{error}"));
        }
    }
    control.checkpoint(5)?;
    let journal_sync_rewrite_required = initial_acknowledged_generation
        != sync.acknowledged_generation
        || initial_restore_receipt != sync.restore_receipt
        || initial_account_generation_checkpoints != sync.account_generation_checkpoints;
    control.checkpoint(6)?;
    let data = decode_data(&state_json);
    let settings_load =
        load_recoverable_json::<DesktopClientSettings>(&settings_path, "client_settings.json");
    if let Some(message) = settings_load.message.clone() {
        messages.push(message);
    }
    let settings = settings_load.value;
    let bell_markers = build_bell_markers(&data, settings.timer_bell_interval_minutes, now);
    let selected_slot_id = data.slots.first().map(|slot| slot.id).unwrap_or(1);
    let finance_draft = data.finance_profile.clone();
    let startup_sync_due_epoch_millis =
        if workspace_persistence_ready && sync_identity_is_bound(&sync) {
            now.saturating_add(1_200)
        } else {
            0
        };
    control.checkpoint(7)?;
    let workspace = AiWorkspaceIdentity {
        namespace_root: root_dir,
        state_path: state_path.clone(),
        server_instance_id: sync.server_instance_id.clone(),
        account_namespace: sync.account_namespace.clone(),
        user_id: sync.user_id.clone(),
    };
    let mut media_status = String::new();
    recover_loaded_workspace_note_media(
        workspace_persistence_ready,
        &state_json,
        &workspace,
        &mut media_status,
    );
    if !media_status.is_empty() {
        messages.push(media_status);
    }
    control.checkpoint(8)?;
    if sync_rewrite_required || secrets_rewrite_required || journal_sync_rewrite_required {
        if let Err(error) = save_sync_files(&sync_path, &secrets_path, &sync) {
            messages.push(format!("敏感信息迁移失败：{error}"));
        }
    }
    control.checkpoint(9)?;
    Ok(LoadedWorkspace {
        state_path,
        sync_path,
        settings_path,
        secrets_path,
        revocations_path,
        state_json,
        data,
        sync,
        settings,
        bell_markers,
        selected_slot_id,
        finance_draft,
        pending_revocations,
        background_job_store,
        pending_reconciliation_jobs,
        workspace_persistence_ready,
        startup_sync_due_epoch_millis,
        messages,
        now,
    })
}

fn recover_loaded_workspace_note_media(
    ready: bool,
    snapshot: &str,
    workspace: &AiWorkspaceIdentity,
    status: &mut String,
) {
    if !ready {
        *status = append_status(
            status.clone(),
            "未验证工作区处于只读状态，已延后媒体恢复与清理".to_string(),
        );
        return;
    }
    let store = match note_media_store_for_workspace(workspace) {
        Ok(store) => store,
        Err(error) => {
            *status = append_status(status.clone(), format!("媒体存储恢复未完成：{error}"));
            return;
        }
    };
    let references = match workspace_retained_note_media_references(snapshot, workspace)
        .map_err(|error| error.to_string())
    {
        Ok(references) => references,
        Err(error) => {
            *status = append_status(
                status.clone(),
                format!("附件引用核验失败，已保留文件：{error}"),
            );
            return;
        }
    };
    match store.retry_cleanup_blocks(&references) {
        Ok(report) if report.is_clear() => {}
        Ok(_) => {
            *status = append_status(
                status.clone(),
                "仍有媒体清理阻断，解决前不会启用文档加密".to_string(),
            );
        }
        Err(error) => {
            *status = append_status(status.clone(), format!("媒体清理阻断恢复失败：{error}"));
            return;
        }
    }
    let attachment_ids = references.ids();
    if let Err(error) = store.recover_pending_referenced(&attachment_ids) {
        *status = append_status(status.clone(), format!("媒体待提交日志恢复失败：{error}"));
        return;
    }
    if !references.is_complete() {
        *status = append_status(status.clone(), "附件引用尚未确认，已暂缓清理".to_string());
        return;
    }
    match store.cleanup_unreferenced_pending(attachment_ids) {
        Ok(report) if report.is_clear() => {}
        Ok(_) => {
            *status = append_status(
                status.clone(),
                "无引用媒体的安全回滚未完成，已保留阻断证据".to_string(),
            );
        }
        Err(error) => {
            *status = append_status(status.clone(), format!("无引用媒体回滚失败：{error}"));
        }
    }
    match store.cleanup_unreferenced_committed(&attachment_ids) {
        Ok(report) if report.is_clear() => {}
        Ok(_) => {
            *status = append_status(
                status.clone(),
                "无引用已提交媒体的安全清理尚未完成，已保留恢复标记".to_string(),
            );
        }
        Err(error) => {
            *status = append_status(status.clone(), format!("无引用已提交媒体清理失败：{error}"));
        }
    }
}

fn prepare_startup_namespace() -> Result<(), StartupFailure> {
    #[cfg(target_os = "windows")]
    let app_root = app_dir();
    #[cfg(target_os = "windows")]
    let legacy_root = legacy_app_dir();
    #[cfg(target_os = "windows")]
    let migration_requires_legacy_exclusion =
        match app_namespace_migration_requires_legacy_exclusion(&app_root) {
            Ok(required) => required,
            Err(error) => {
                return Err(StartupFailure::new(
                    "GTW-DATA-001",
                    format!("无法核验隔离数据目录：{error}"),
                ));
            }
        };
    #[cfg(target_os = "windows")]
    if migration_requires_legacy_exclusion {
        match other_grid_timer_windows_client_processes() {
            Ok(processes) if processes.is_empty() => {}
            Ok(processes) => {
                append_client_runtime_log(&format!(
                    "EXIT legacy_migration_blocked_process_count={}",
                    processes.len()
                ));
                return Err(StartupFailure::new(
                    "GTW-MIGRATE-001",
                    format!(
                        "检测到另一个可信旧版计时器客户端（{} 个）；请先关闭旧客户端，再进行一次性数据迁移。",
                        processes.len()
                    ),
                ));
            }
            Err(error) => {
                return Err(StartupFailure::new(
                    "GTW-MIGRATE-002",
                    format!("无法核验旧版客户端进程，已停止一次性数据迁移：{error}"),
                ));
            }
        }
    }
    #[cfg(target_os = "windows")]
    if let Err(error) = ensure_isolated_app_namespace_with_hooks(
        &legacy_root,
        &app_root,
        || Ok(()),
        ensure_no_detected_legacy_client_processes,
    ) {
        return Err(StartupFailure::new(
            "GTW-MIGRATE-003",
            format!("无法安全迁移本地数据：{error}"),
        ));
    }
    Ok(())
}
