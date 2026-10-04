// v1.1.0.3 Windows - Share the application identity across its window, taskbar and tray.
fn desktop_window_icon() -> Arc<egui::IconData> {
    const PIXELS: &[u8; 256 * 256 * 4] = include_bytes!("../../assets/windows/tenrate.rgba");
    Arc::new(egui::IconData {
        rgba: PIXELS.to_vec(),
        width: 256,
        height: 256,
    })
}
// v1.0.3.17 Windows - Queue tray pause actions with the timer save worker.
// v1.0.3.16 Windows - Rebuild tray text only when a displayed timer value changes.

#[derive(Default)]
struct DesktopTray {
    #[cfg(target_os = "windows")]
    native: Option<parity_tray_native::Tray>,
    #[cfg(target_os = "windows")]
    tip_data_version: Option<u64>,
    #[cfg(target_os = "windows")]
    tip_running_slots: Vec<(i32, i64, bool)>,
    force_exit: bool,
    hidden: bool,
}

impl TimerWindowsClient {
    fn start_desktop_tray(&mut self, cc: &eframe::CreationContext<'_>) {
        #[cfg(target_os = "windows")]
        {
            use wry::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let Ok(handle) = cc.window_handle() else {
                return;
            };
            let RawWindowHandle::Win32(handle) = handle.as_raw() else {
                return;
            };
            self.start_desktop_tray_for_window(&cc.egui_ctx, handle.hwnd.get());
        }
    }

    #[cfg(target_os = "windows")]
    fn start_desktop_tray_for_window(&mut self, ctx: &egui::Context, handle: isize) {
        match parity_tray_native::Tray::start(ctx.clone(), handle) {
            Ok(tray) => {
                self.desktop_ui.parity.tray.native = Some(tray);
                self.desktop_ui.parity.tray.tip_data_version = None;
            }
            Err(e) => append_client_runtime_log(&format!("TRAY_INIT_FAILED {e}")),
        }
    }

    fn poll_desktop_tray(&mut self, ctx: &egui::Context) {
        #[cfg(target_os = "windows")]
        {
            let events = self
                .desktop_ui
                .parity
                .tray
                .native
                .as_ref()
                .map(|t| t.events.try_iter().collect::<Vec<_>>())
                .unwrap_or_default();
            for event in events {
                match event {
                    1 => {
                        self.desktop_ui.parity.tray.hidden = false;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                    2 => {
                        if !self.workspace_edit_locked() {
                            self.pause_running_slots(ctx);
                        }
                    }
                    3 => {
                        self.request_desktop_tray_exit(ctx);
                    }
                    _ => {}
                }
            }
            let tip_cache = &mut self.desktop_ui.parity.tray;
            if tip_cache.native.is_none() {
                return;
            }
            let mut tip_changed = tip_cache.tip_data_version != Some(self.data_version);
            let mut running_count = 0;
            if let Some(projection) = self.desktop_ui.projection.as_ref() {
                for slot in projection.slots.iter().filter(|slot| slot.is_running) {
                    let signature = (
                        slot.id,
                        slot.accumulated_millis.max(0) / 1_000,
                        slot.micro_break_phase == app_data::TimerViewPhase::Break,
                    );
                    if let Some(cached) = tip_cache.tip_running_slots.get_mut(running_count) {
                        if *cached != signature {
                            *cached = signature;
                            tip_changed = true;
                        }
                    } else {
                        tip_cache.tip_running_slots.push(signature);
                        tip_changed = true;
                    }
                    running_count += 1;
                }
            }
            if tip_cache.tip_running_slots.len() != running_count {
                tip_cache.tip_running_slots.truncate(running_count);
                tip_changed = true;
            }
            tip_cache.tip_data_version = Some(self.data_version);
            if tip_changed {
                let tray = tip_cache.native.as_mut().expect("native tray is available");
                let running = self
                    .desktop_ui
                    .projection
                    .as_ref()
                    .map(|p| {
                        p.slots
                            .iter()
                            .filter(|s| s.is_running)
                            .map(|s| {
                                format!(
                                    "{:02} {} {}",
                                    s.id,
                                    if s.micro_break_phase == app_data::TimerViewPhase::Break {
                                        "休息"
                                    } else {
                                        "专注"
                                    },
                                    format_duration(s.accumulated_millis)
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();
                tray.set_tip(&if running.is_empty() {
                    "十倍率 · 已暂停".into()
                } else {
                    format!("十倍率\n{running}")
                });
            }
        }
    }

    fn request_desktop_tray_exit(&mut self, ctx: &egui::Context) {
        self.desktop_ui.parity.tray.force_exit = true;
        // The close barrier can retain the window for a pending write or an
        // error. Its state must agree with the restored native visibility so
        // update keeps painting the progress and recovery controls.
        self.desktop_ui.parity.tray.hidden = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn close_to_desktop_tray(&mut self, ctx: &egui::Context) -> bool {
        #[cfg(target_os = "windows")]
        if self.settings.close_to_tray
            && self.desktop_ui.parity.tray.native.is_some()
            && !self.desktop_ui.parity.tray.force_exit
            && matches!(self.shutdown_state, ClientShutdownState::Running)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.rich_editor.active {
                self.status = "请先保存并关闭富文本编辑器".into();
                return true;
            }
            if let Err(error) = self.flush_all_pending_saves() {
                self.status = format!("后台运行前保存失败：{error}");
                return true;
            }
            self.desktop_ui.parity.tray.hidden = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            return true;
        }
        false
    }
}

#[cfg(target_os = "windows")]
mod parity_tray_native {
    use super::*;
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicBool, AtomicIsize};
    type Handle = isize;
    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }
    #[repr(C)]
    struct Msg {
        hwnd: Handle,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        point: Point,
        private: u32,
    }
    #[repr(C)]
    struct WindowClass {
        style: u32,
        proc: Option<unsafe extern "system" fn(Handle, u32, usize, isize) -> isize>,
        class_extra: i32,
        window_extra: i32,
        instance: Handle,
        icon: Handle,
        cursor: Handle,
        background: Handle,
        menu: *const u16,
        name: *const u16,
    }
    #[repr(C)]
    struct NotifyIcon {
        size: u32,
        window: Handle,
        id: u32,
        flags: u32,
        callback: u32,
        icon: Handle,
        tip: [u16; 128],
        state: u32,
        state_mask: u32,
        info: [u16; 256],
        timeout: u32,
        info_title: [u16; 64],
        info_flags: u32,
        guid: [u8; 16],
        balloon: Handle,
    }
    #[link(name = "user32")]
    extern "system" {
        fn RegisterClassW(class: *const WindowClass) -> u16;
        fn CreateWindowExW(
            ex: u32,
            class: *const u16,
            title: *const u16,
            style: u32,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            parent: Handle,
            menu: Handle,
            instance: Handle,
            param: *const std::ffi::c_void,
        ) -> Handle;
        fn DefWindowProcW(window: Handle, msg: u32, w: usize, l: isize) -> isize;
        fn GetMessageW(msg: *mut Msg, window: Handle, min: u32, max: u32) -> i32;
        fn DispatchMessageW(msg: *const Msg) -> isize;
        fn PostMessageW(window: Handle, msg: u32, w: usize, l: isize) -> i32;
        fn DestroyWindow(window: Handle) -> i32;
        fn UnregisterClassW(class: *const u16, instance: Handle) -> i32;
        fn PostQuitMessage(code: i32);
        fn LoadIconW(instance: Handle, name: *const u16) -> Handle;
        fn CreatePopupMenu() -> Handle;
        fn AppendMenuW(menu: Handle, flags: u32, id: usize, text: *const u16) -> i32;
        fn TrackPopupMenu(
            menu: Handle,
            flags: u32,
            x: i32,
            y: i32,
            reserved: i32,
            window: Handle,
            rect: *const std::ffi::c_void,
        ) -> u32;
        fn DestroyMenu(menu: Handle) -> i32;
        fn GetCursorPos(point: *mut Point) -> i32;
        fn SetForegroundWindow(window: Handle) -> i32;
        fn IsWindowVisible(window: Handle) -> i32;
        fn IsIconic(window: Handle) -> i32;
    }
    #[link(name = "shell32")]
    extern "system" {
        fn Shell_NotifyIconW(message: u32, data: *const NotifyIcon) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> Handle;
    }
    thread_local! {static EVENTS:RefCell<Option<(mpsc::Sender<u8>,egui::Context)>>=const{RefCell::new(None)};}
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }
    fn send(command: u8) {
        EVENTS.with(|e| {
            if let Some((tx, ctx)) = e.borrow().as_ref() {
                let _ = tx.send(command);
                ctx.request_repaint();
            }
        });
    }
    unsafe extern "system" fn procedure(hwnd: Handle, msg: u32, w: usize, l: isize) -> isize {
        if msg == 0x8001 {
            match l as u32 {
                0x202 | 0x203 => send(1),
                0x205 => {
                    let menu = CreatePopupMenu();
                    for (id, label) in [(1, "打开十倍率"), (2, "暂停全部计时"), (3, "退出")]
                    {
                        AppendMenuW(menu, 0, id, wide(label).as_ptr());
                    }
                    let mut point = Point { x: 0, y: 0 };
                    GetCursorPos(&mut point);
                    SetForegroundWindow(hwnd);
                    let command =
                        TrackPopupMenu(menu, 0x180, point.x, point.y, 0, hwnd, std::ptr::null());
                    DestroyMenu(menu);
                    if command > 0 {
                        send(command as u8);
                    }
                }
                _ => {}
            }
            return 0;
        }
        if msg == 0x10 {
            DestroyWindow(hwnd);
            return 0;
        }
        if msg == 2 {
            PostQuitMessage(0);
            return 0;
        }
        DefWindowProcW(hwnd, msg, w, l)
    }

    pub struct Tray {
        pub events: mpsc::Receiver<u8>,
        window: Arc<AtomicIsize>,
        stop: Arc<AtomicBool>,
        tip: String,
    }
    impl Tray {
        pub fn start(ctx: egui::Context, app_window: Handle) -> io::Result<Self> {
            let (tx, events) = mpsc::channel();
            let (ready_tx, ready_rx) = mpsc::sync_channel(1);
            let window = Arc::new(AtomicIsize::new(0));
            let target = window.clone();
            let stop = Arc::new(AtomicBool::new(false));
            let stopped = stop.clone();
            std::thread::Builder::new()
                .name("timer-notification-area".into())
                .spawn(move || unsafe {
                    let instance = GetModuleHandleW(std::ptr::null());
                    let class_name = wide(&format!("TenRateTray{}", std::process::id()));
                    let class = WindowClass {
                        style: 0,
                        proc: Some(procedure),
                        class_extra: 0,
                        window_extra: 0,
                        instance,
                        icon: 0,
                        cursor: 0,
                        background: 0,
                        menu: std::ptr::null(),
                        name: class_name.as_ptr(),
                    };
                    if RegisterClassW(&class) == 0 {
                        let _ = ready_tx.send(false);
                        return;
                    }
                    let hwnd = CreateWindowExW(
                        0,
                        class_name.as_ptr(),
                        wide("十倍率").as_ptr(),
                        0,
                        0,
                        0,
                        0,
                        0,
                        0,
                        0,
                        instance,
                        std::ptr::null(),
                    );
                    if hwnd == 0 {
                        UnregisterClassW(class_name.as_ptr(), instance);
                        let _ = ready_tx.send(false);
                        return;
                    }
                    let mut icon: NotifyIcon = std::mem::zeroed();
                    icon.size = std::mem::size_of::<NotifyIcon>() as u32;
                    icon.window = hwnd;
                    icon.id = 1;
                    icon.flags = 7;
                    icon.callback = 0x8001;
                    icon.icon = LoadIconW(instance, 101usize as *const u16);
                    if icon.icon == 0 {
                        DestroyWindow(hwnd);
                        UnregisterClassW(class_name.as_ptr(), instance);
                        let _ = ready_tx.send(false);
                        return;
                    }
                    for (dst, src) in icon.tip.iter_mut().zip(wide("十倍率")) {
                        *dst = src;
                    }
                    if Shell_NotifyIconW(0, &icon) == 0 {
                        DestroyWindow(hwnd);
                        UnregisterClassW(class_name.as_ptr(), instance);
                        let _ = ready_tx.send(false);
                        return;
                    }
                    EVENTS.with(|e| *e.borrow_mut() = Some((tx, ctx.clone())));
                    target.store(hwnd, AtomicOrdering::Release);
                    let wake = stopped.clone();
                    std::thread::spawn(move || {
                        while !wake.load(AtomicOrdering::Acquire) {
                            if IsWindowVisible(app_window) == 0 || IsIconic(app_window) != 0 {
                                // winit 0.29 invalidates the paint region on request_redraw,
                                // but Windows never delivers it to a hidden window.
                                // Deliver WM_PAINT through the owning thread's queue so the
                                // existing timer, save and reminder logic keeps running.
                                PostMessageW(app_window, 0x0f, 0, 0);
                            }
                            std::thread::sleep(Duration::from_millis(500));
                        }
                    });
                    if ready_tx.send(true).is_err() || stopped.load(AtomicOrdering::Acquire) {
                        PostMessageW(hwnd, 0x10, 0, 0);
                    }
                    let mut message: Msg = std::mem::zeroed();
                    while GetMessageW(&mut message, 0, 0, 0) > 0 {
                        DispatchMessageW(&message);
                    }
                    stopped.store(true, AtomicOrdering::Release);
                    Shell_NotifyIconW(2, &icon);
                    target.store(0, AtomicOrdering::Release);
                    UnregisterClassW(class_name.as_ptr(), instance);
                })?;
            if ready_rx
                .recv_timeout(Duration::from_secs(3))
                .unwrap_or(false)
            {
                Ok(Self {
                    events,
                    window,
                    stop,
                    tip: String::new(),
                })
            } else {
                stop.store(true, AtomicOrdering::Release);
                let hwnd = window.load(AtomicOrdering::Acquire);
                if hwnd != 0 {
                    unsafe {
                        PostMessageW(hwnd, 0x10, 0, 0);
                    }
                }
                Err(io::Error::other("无法创建通知区图标"))
            }
        }
        pub fn set_tip(&mut self, text: &str) {
            if text == self.tip {
                return;
            }
            self.tip = text.into();
            let hwnd = self.window.load(AtomicOrdering::Acquire);
            if hwnd == 0 {
                return;
            }
            unsafe {
                let mut icon: NotifyIcon = std::mem::zeroed();
                icon.size = std::mem::size_of::<NotifyIcon>() as u32;
                icon.window = hwnd;
                icon.id = 1;
                icon.flags = 4;
                // Do not split a UTF-16 surrogate pair at the shell's 127-unit limit.
                let mut units = text.encode_utf16().take(127).collect::<Vec<_>>();
                if units.last().is_some_and(|c| (0xd800..=0xdbff).contains(c)) {
                    units.pop();
                }
                icon.tip[..units.len()].copy_from_slice(&units);
                Shell_NotifyIconW(1, &icon);
            }
        }
    }
    impl Drop for Tray {
        fn drop(&mut self) {
            self.stop.store(true, AtomicOrdering::Release);
            let hwnd = self.window.load(AtomicOrdering::Acquire);
            if hwnd != 0 {
                unsafe {
                    PostMessageW(hwnd, 0x10, 0, 0);
                }
            }
        }
    }
}
