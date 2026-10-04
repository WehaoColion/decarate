// v1.1.0.2 Windows - Present a responsive loading window while a worker verifies local data.
// v1.0.2.1 - Reuse an unchanged, verified startup snapshot and sync checkpoint.

include!("startup_fonts.rs");

#[derive(Clone, Default)]
struct DesktopStartupControl {
    cancelled: Arc<AtomicBool>,
    phase: Arc<std::sync::atomic::AtomicU8>,
}

impl DesktopStartupControl {
    fn checkpoint(&self, phase: u8) -> Result<(), StartupFailure> {
        if self.cancelled.load(AtomicOrdering::Acquire) {
            return Err(StartupFailure::new("GTW-START-CANCELLED", "已取消启动"));
        }
        self.phase.store(phase, AtomicOrdering::Release);
        Ok(())
    }

    fn cancel(&self) {
        self.cancelled.store(true, AtomicOrdering::Release);
    }

    fn label(&self) -> &'static str {
        match self.phase.load(AtomicOrdering::Acquire) {
            0 => "正在准备本地工作区",
            1..=3 => "正在恢复上次状态",
            4..=5 => "正在核验本地资料",
            6 => "正在准备界面",
            7 => "正在核验附件",
            _ => "即将完成",
        }
    }
}

enum DesktopStartupState {
    Loading,
    Ready(Box<TimerWindowsClient>),
    Recovery {
        message: String,
        workspace: Option<Box<LoadedWorkspace>>,
    },
}

struct DesktopStartupApp {
    state: DesktopStartupState,
    control: DesktopStartupControl,
    receiver: mpsc::Receiver<Result<LoadedWorkspace, StartupFailure>>,
    worker: Option<thread::JoinHandle<()>>,
    fonts: DesktopStartupFonts,
    pending_workspace: Option<LoadedWorkspace>,
    activation_requested: Arc<AtomicBool>,
    close_requested: bool,
    #[cfg(target_os = "windows")]
    window_handle: Option<isize>,
    started: Instant,
    first_frame_recorded: bool,
    first_ready_frame_recorded: bool,
}

impl DesktopStartupApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        activation_requested: Arc<AtomicBool>,
        fonts: DesktopStartupFonts,
    ) -> Self {
        let control = DesktopStartupControl::default();
        let worker_control = control.clone();
        let (sender, receiver) = mpsc::channel();
        let context = cc.egui_ctx.clone();
        let started = Instant::now();
        let worker = thread::Builder::new()
            .name("desktop-startup".into())
            .spawn(move || {
                // Cancellation is observed between complete atomic operations. A
                // close never releases the single-instance guards mid-recovery.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    worker_control.checkpoint(0)?;
                    prepare_startup_namespace()?;
                    worker_control.checkpoint(1)?;
                    load_desktop_workspace(app_dir(), &worker_control)
                }))
                .unwrap_or_else(|_| {
                    Err(StartupFailure::new(
                        "GTW-LOAD-PANIC",
                        "本地资料加载中断，请重新启动重试",
                    ))
                });
                let _ = sender.send(result);
                context.request_repaint();
            });
        let (worker, state) = match worker {
            Ok(worker) => (Some(worker), DesktopStartupState::Loading),
            Err(error) => (
                None,
                DesktopStartupState::Recovery {
                    message: format!("无法启动资料加载任务：{error}"),
                    workspace: None,
                },
            ),
        };
        #[cfg(target_os = "windows")]
        let window_handle = {
            use wry::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            cc.window_handle()
                .ok()
                .and_then(|handle| match handle.as_raw() {
                    RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
                    _ => None,
                })
        };
        Self {
            state,
            control,
            receiver,
            worker,
            fonts,
            pending_workspace: None,
            activation_requested,
            close_requested: false,
            #[cfg(target_os = "windows")]
            window_handle,
            started,
            first_frame_recorded: false,
            first_ready_frame_recorded: false,
        }
    }

    fn install_workspace(&mut self, workspace: LoadedWorkspace, ctx: &egui::Context) {
        if self.close_requested
            || self.control.cancelled.load(AtomicOrdering::Acquire)
            || !self.fonts.is_ready()
        {
            return;
        }
        let mut client = TimerWindowsClient::from_loaded_workspace(
            workspace,
            Arc::clone(&self.activation_requested),
        );
        #[cfg(target_os = "windows")]
        if let Some(handle) = self.window_handle {
            client.start_desktop_tray_for_window(ctx, handle);
        }
        append_client_runtime_log(&format!(
            "STARTUP_WORKSPACE_READY elapsed_ms={} writable={}",
            self.started.elapsed().as_millis(),
            client.workspace_persistence_ready,
        ));
        self.state = DesktopStartupState::Ready(Box::new(client));
    }

    fn poll_completion(&mut self, ctx: &egui::Context) {
        // A native close and a completed load can arrive in the same frame.
        // Observe the close before installing a client or starting its workers.
        if !matches!(self.state, DesktopStartupState::Ready(_))
            && ctx.input(|input| input.viewport().close_requested())
        {
            self.close_requested = true;
            self.control.cancel();
            self.fonts.cancel();
            if self
                .worker
                .as_ref()
                .is_some_and(|worker| !worker.is_finished())
                || !self.fonts.is_finished()
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
        }
        if self.control.cancelled.load(AtomicOrdering::Acquire) {
            self.fonts.cancel();
        }
        self.fonts.poll(ctx, self.first_frame_recorded);
        if !matches!(self.state, DesktopStartupState::Loading) {
            return;
        }
        let result = if let Some(workspace) = self.pending_workspace.take() {
            Ok(workspace)
        } else {
            match self.receiver.try_recv() {
                Ok(result) => result,
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => Err(StartupFailure::new(
                    "GTW-LOAD-DISCONNECTED",
                    "资料加载任务已结束，请重新启动重试",
                )),
            }
        };
        // Only a completed worker is joined on the window thread.
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
        if self.close_requested || self.control.cancelled.load(AtomicOrdering::Acquire) {
            self.state = DesktopStartupState::Recovery {
                message: "正在关闭".into(),
                workspace: None,
            };
            if self.fonts.is_finished() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        }
        match result {
            Ok(workspace) if workspace.workspace_persistence_ready => {
                if self.fonts.is_ready() {
                    self.install_workspace(workspace, ctx);
                } else {
                    self.pending_workspace = Some(workspace);
                }
            }
            Ok(workspace) => {
                self.state = DesktopStartupState::Recovery {
                    message: workspace.messages.join("；"),
                    workspace: Some(Box::new(workspace)),
                };
            }
            Err(error) => {
                append_client_runtime_log(&format!("STARTUP_RECOVERY code={}", error.code));
                self.state = DesktopStartupState::Recovery {
                    message: error.message,
                    workspace: None,
                };
            }
        }
    }
}

impl eframe::App for DesktopStartupApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if !self.first_frame_recorded {
            append_client_runtime_log(&format!(
                "STARTUP_FIRST_FRAME elapsed_ms={}",
                self.started.elapsed().as_millis()
            ));
        }
        self.poll_completion(ctx);
        self.first_frame_recorded = true;
        if let DesktopStartupState::Ready(client) = &mut self.state {
            client.update(ctx, frame);
            if !self.first_ready_frame_recorded {
                self.first_ready_frame_recorded = true;
                append_client_runtime_log(&format!(
                    "STARTUP_READY_FRAME writable={}",
                    client.workspace_persistence_ready
                ));
            }
            return;
        }
        if self
            .activation_requested
            .swap(false, AtomicOrdering::AcqRel)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        #[cfg(target_os = "windows")]
        if take_windows_release_upgrade_request() {
            // Loading and recovery screens have not accepted editor changes.
            // Cancel at the existing recovery checkpoints and retain all disk
            // evidence, then wait for workers before releasing the instance.
            self.close_requested = true;
            self.control.cancel();
            self.fonts.cancel();
            append_client_runtime_log("RELEASE_UPGRADE_CANCEL_STARTUP_AT_SAFE_BOUNDARY");
        }
        let mut open_recovery = false;
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() * 0.32).max(20.0));
                if !self.fonts.is_ready() {
                    // The native title already identifies this Chinese app.
                    // Keep the first frame independent of system-font disk I/O.
                    ui.spinner();
                    return;
                }
                ui.heading("十倍率");
                ui.add_space(16.0);
                match &self.state {
                    DesktopStartupState::Loading => {
                        ui.spinner();
                        ui.label(if self.close_requested {
                            "正在安全结束本地恢复"
                        } else {
                            self.control.label()
                        });
                    }
                    DesktopStartupState::Recovery { message, workspace } => {
                        ui.label("本地资料未通过完整核验");
                        ui.add_space(10.0);
                        ui.label(message);
                        ui.add_space(12.0);
                        if workspace.is_some() && ui.button("以只读方式查看").clicked() {
                            open_recovery = true;
                        }
                        if ui.button("关闭").clicked() {
                            self.close_requested = true;
                            self.control.cancel();
                            self.fonts.cancel();
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                    DesktopStartupState::Ready(_) => {}
                }
            });
        });
        if open_recovery {
            if let DesktopStartupState::Recovery { workspace, .. } = &mut self.state {
                if let Some(workspace) = workspace.take() {
                    self.install_workspace(*workspace, ctx);
                }
            }
        }
        if matches!(self.state, DesktopStartupState::Loading)
            || self.close_requested
            || !self.fonts.is_ready()
        {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if self.close_requested
            && self
                .worker
                .as_ref()
                .is_none_or(|worker| worker.is_finished())
            && self.fonts.is_finished()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        self.control.cancel();
        self.fonts.cancel();
        if let DesktopStartupState::Ready(client) = &mut self.state {
            client.on_exit(gl);
        }
        // On a forced OS exit, wait for an in-progress atomic recovery boundary
        // rather than release the instance lock while it still writes files.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.pending_workspace = None;
        self.fonts.cancel_and_join();
    }
}

fn startup_workspace_requires_initialization_commit(
    protected_sync_state: &[u8],
    sync: &DesktopSyncSession,
) -> bool {
    // The caller has already loaded and verified the authoritative journal,
    // repaired privacy history/mirrors, and merged its checkpoint. An exact
    // nonempty pair is already durable; committing it again would repeat a
    // whole journal audit and rewrite an unchanged JSON mirror. Empty metadata
    // keeps first-import, guest, and legacy account initialization unchanged.
    if protected_sync_state.is_empty() {
        return true;
    }
    match (
        decode_workspace_journal_sync_state(protected_sync_state),
        workspace_journal_sync_state(sync),
    ) {
        (Ok(committed), Ok(expected)) => committed != expected,
        _ => true,
    }
}
