// v1.1.0.2 Windows - Read system fonts before the UI needs them without blocking its first frame.
#[derive(Default)]
struct DesktopUiFontBytes {
    symbols: Option<Vec<u8>>,
    chinese: Option<Vec<u8>>,
}

impl DesktopUiFontBytes {
    fn byte_count(&self) -> usize {
        self.symbols.as_ref().map_or(0, Vec::len) + self.chinese.as_ref().map_or(0, Vec::len)
    }

    fn into_definitions(self) -> egui::FontDefinitions {
        let mut fonts = egui::FontDefinitions::default();
        for (name, bytes) in [
            ("system_symbols", self.symbols),
            ("system_chinese", self.chinese),
        ] {
            let Some(bytes) = bytes else { continue };
            fonts
                .font_data
                .insert(name.to_string(), egui::FontData::from_owned(bytes));
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .push(name.to_string());
            }
        }
        fonts
    }
}

fn read_desktop_ui_font_bytes(cancelled: &AtomicBool) -> DesktopUiFontBytes {
    if cancelled.load(AtomicOrdering::Acquire) {
        return DesktopUiFontBytes::default();
    }
    let symbols = fs::read(r"C:\Windows\Fonts\seguisym.ttf").ok();
    let candidates = [
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\Deng.ttf",
        r"C:\Windows\Fonts\AlibabaPuHuiTi-2-55-Regular.ttf",
        r"C:\Windows\Fonts\AlibabaPuHuiTi.ttf",
        r"C:\Windows\Fonts\simhei.ttf",
    ];
    let mut chinese = None;
    for path in candidates {
        // An in-progress OS read finishes normally; cancellation prevents the
        // next read and discards owned bytes at the next safe boundary.
        if cancelled.load(AtomicOrdering::Acquire) {
            return DesktopUiFontBytes::default();
        }
        if let Ok(bytes) = fs::read(path) {
            chinese = Some(bytes);
            break;
        }
    }
    DesktopUiFontBytes { symbols, chinese }
}

#[derive(Clone, Copy)]
enum DesktopStartupFontStage {
    Pending,
    Applied(u64),
    Ready,
}

struct DesktopStartupFonts {
    receiver: mpsc::Receiver<DesktopUiFontBytes>,
    worker: Option<thread::JoinHandle<()>>,
    cancelled: Arc<AtomicBool>,
    stage: DesktopStartupFontStage,
}

impl DesktopStartupFonts {
    fn start() -> Self {
        let (sender, receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let worker = thread::Builder::new()
            .name("desktop-startup-fonts".into())
            .spawn(move || {
                let started = Instant::now();
                let bytes = read_desktop_ui_font_bytes(&worker_cancelled);
                append_client_runtime_log(&format!(
                    "STARTUP_FONTS_LOADED elapsed_ms={} bytes={} chinese={} cancelled={}",
                    started.elapsed().as_millis(),
                    bytes.byte_count(),
                    bytes.chinese.is_some(),
                    worker_cancelled.load(AtomicOrdering::Acquire),
                ));
                if !worker_cancelled.load(AtomicOrdering::Acquire) {
                    let _ = sender.send(bytes);
                }
            })
            .ok();
        if worker.is_none() {
            append_client_runtime_log("STARTUP_FONTS_WORKER_UNAVAILABLE fallback=true");
        }
        Self {
            receiver,
            worker,
            cancelled,
            stage: DesktopStartupFontStage::Pending,
        }
    }

    fn is_ready(&self) -> bool {
        matches!(self.stage, DesktopStartupFontStage::Ready)
    }

    fn is_finished(&self) -> bool {
        self.worker
            .as_ref()
            .is_none_or(|worker| worker.is_finished())
    }

    fn cancel(&self) {
        self.cancelled.store(true, AtomicOrdering::Release);
    }

    fn poll(&mut self, ctx: &egui::Context, first_frame_completed: bool) {
        if self.is_finished() {
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
        if self.cancelled.load(AtomicOrdering::Acquire) {
            while self.receiver.try_recv().is_ok() {}
            return;
        }
        if let DesktopStartupFontStage::Applied(frame) = self.stage {
            // set_fonts takes effect in the next egui frame. No editable UI or
            // automatic sync may start while the old font atlas is still active.
            if ctx.frame_nr() > frame {
                self.stage = DesktopStartupFontStage::Ready;
                append_client_runtime_log("STARTUP_FONTS_READY");
            }
            return;
        }
        if self.is_ready() || !first_frame_completed {
            return;
        }
        let bytes = match self.receiver.try_recv() {
            Ok(bytes) => bytes,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                append_client_runtime_log("STARTUP_FONTS_UNAVAILABLE fallback=true");
                DesktopUiFontBytes::default()
            }
        };
        let count = bytes.byte_count();
        let chinese = bytes.chinese.is_some();
        ctx.set_fonts(bytes.into_definitions());
        self.stage = DesktopStartupFontStage::Applied(ctx.frame_nr());
        append_client_runtime_log(&format!(
            "STARTUP_FONTS_APPLIED bytes={count} chinese={chinese}",
        ));
        ctx.request_repaint();
    }

    fn cancel_and_join(&mut self) {
        self.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        while self.receiver.try_recv().is_ok() {}
    }
}

impl Drop for DesktopStartupFonts {
    fn drop(&mut self) {
        // Also covers run_native failing before it invokes the app creator.
        // The worker and any queued font payload never outlive their owner.
        self.cancel_and_join();
    }
}
