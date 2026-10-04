// v1.1.0.3 Windows - Native rendered timer timing, with programmatic dispatch.
// The real production App::update is used. This measures neither OS mouse
// delivery nor monitor scanout. Screenshot receipt bounds GPU readback completion.
#[cfg(target_os = "windows")]
mod timer_native_latency_tests {
    use super::*;
    use std::sync::Mutex;
    use winit::platform::windows::EventLoopBuilderExtWindows;

    #[derive(Clone, Copy)]
    enum CaptureKind {
        Feedback,
        Confirmed,
    }

    struct Operation {
        index: usize,
        start: bool,
        action_id: u64,
        click: Instant,
        dispatch_return: Instant,
        feedback_update: Option<Instant>,
        feedback_rendered: Option<Instant>,
        confirmed_update: Option<Instant>,
        confirmed_frame_work_micros: Option<u64>,
        feedback_was_pending: bool,
    }

    #[derive(Default)]
    struct Outcome {
        samples: Vec<Value>,
        images: Vec<(String, Arc<egui::ColorImage>)>,
        error: Option<String>,
        completed: bool,
    }

    struct NativeTimerApp {
        client: TimerWindowsClient,
        outcome: Arc<Mutex<Outcome>>,
        variant: String,
        started: Instant,
        warm_frames: usize,
        idle_frames: usize,
        index: usize,
        operation: Option<Operation>,
        capture: Option<CaptureKind>,
        state_bytes: usize,
        journal_bytes: u64,
    }

    fn micros(start: Instant, end: Instant) -> u64 {
        end.saturating_duration_since(start)
            .as_micros()
            .min(u64::MAX as u128) as u64
    }

    impl NativeTimerApp {
        fn capture_name(operation: &Operation, kind: &str) -> String {
            format!(
                "{}_{:02}_{kind}.png",
                if operation.start { "start" } else { "pause" },
                operation.index / 2
            )
        }

        fn retain_capture(&self, kind: &str, image: Arc<egui::ColorImage>) {
            let operation = self.operation.as_ref().unwrap();
            if [0, 1, 58, 59].contains(&operation.index) {
                self.outcome
                    .lock()
                    .unwrap()
                    .images
                    .push((Self::capture_name(operation, kind), image));
            }
        }

        fn finish_operation(&mut self, captured: Instant) -> Result<(), String> {
            let operation = self.operation.take().ok_or("capture without operation")?;
            let times = self
                .client
                .persistence
                .timer_latency_probe
                .as_ref()
                .ok_or("missing action timing")?;
            if times.action_id != operation.action_id {
                return Err("action identity changed".into());
            }
            let submitted = times.submitted.ok_or("missing submitted timestamp")?;
            let worker_started = times
                .worker_started
                .ok_or("missing worker start timestamp")?;
            let transformed = times
                .transform_completed
                .ok_or("missing transform timestamp")?;
            let receipt_ready = times
                .receipt_ready
                .ok_or("missing receipt ready timestamp")?;
            let saved = times
                .save_completed
                .ok_or("missing durable save timestamp")?;
            let receipt = times
                .receipt_applied
                .ok_or("missing applied receipt timestamp")?;
            let feedback = operation
                .feedback_rendered
                .ok_or("missing feedback screenshot")?;
            if !(operation.click <= times.accepted
                && times.accepted <= submitted
                && submitted <= worker_started
                && worker_started <= transformed
                && transformed <= saved
                && saved <= receipt_ready
                && receipt_ready <= receipt
                && receipt <= captured)
            {
                return Err("native timing order invalid".into());
            }
            self.outcome.lock().unwrap().samples.push(json!({
                "variant": self.variant, "caseMiB": 68, "sample": operation.index / 2,
                "action": if operation.start { "start" } else { "pause" }, "actionId": operation.action_id,
                "stateBytes": self.state_bytes, "journalBytes": self.journal_bytes,
                "dispatchReturnMicros": micros(operation.click, operation.dispatch_return),
                "acceptedMicros": micros(operation.click, times.accepted),
                "submittedMicros": micros(operation.click, submitted),
                "workerStartedMicros": micros(operation.click, worker_started),
                "transformCompletedMicros": micros(operation.click, transformed),
                "feedbackUpdateReturnMicros": micros(operation.click, operation.feedback_update.unwrap()),
                "feedbackRenderedUpperBoundMicros": micros(operation.click, feedback),
                "saveCompletedMicros": micros(operation.click, saved),
                "receiptReadyMicros": micros(operation.click, receipt_ready),
                "receiptAppliedMicros": micros(operation.click, receipt),
                "confirmedUpdateReturnMicros": micros(operation.click, operation.confirmed_update.unwrap()),
                "confirmedRenderedUpperBoundMicros": micros(operation.click, captured),
                "confirmedFrameWorkMicros": operation.confirmed_frame_work_micros.unwrap(),
                "prepareMainThreadMicros": micros(times.accepted, submitted),
                "workerQueueMicros": micros(submitted, worker_started),
                "workerTransformMicros": micros(worker_started, transformed),
                "durableSaveMicros": micros(transformed, saved),
                "workerPostprocessMicros": micros(saved, receipt_ready),
                "receiptDeliveryAndApplyMicros": micros(receipt_ready, receipt),
                "feedbackWasPending": operation.feedback_was_pending,
                "feedbackVisible": self.variant == "candidate" || !operation.feedback_was_pending,
                "feedbackVisibilityBasis": "candidate pending label or confirmed timer state; baseline pending first frame has no pending label",
                "profile": "real production eframe update and native GPU viewport; programmatic dispatch; screenshot callback upper bounds; no OS mouse or monitor scanout measurement"
            }));
            self.index += 1;
            self.idle_frames = 2;
            Ok(())
        }

        fn step(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) -> Result<(), String> {
            if let Some(operation) = &self.operation {
                if operation.click.elapsed() > Duration::from_secs(90) {
                    return Err("native timer operation exceeded 90 seconds".into());
                }
            } else if self.warm_frames < 8 && self.started.elapsed() > Duration::from_secs(90) {
                return Err("native window warmup exceeded 90 seconds".into());
            }
            let captures = ctx.input(|input| {
                input
                    .events
                    .iter()
                    .filter_map(|event| {
                        if let egui::Event::Screenshot { image, .. } = event {
                            Some(image.clone())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            });
            for image in captures {
                let captured = Instant::now();
                if image.width() == 0 || image.height() == 0 {
                    return Err("empty native framebuffer".into());
                }
                match self.capture.take().ok_or("unsolicited native screenshot")? {
                    CaptureKind::Feedback => {
                        self.retain_capture("feedback", image.clone());
                        let operation = self.operation.as_mut().unwrap();
                        operation.feedback_rendered = Some(captured);
                        if !operation.feedback_was_pending {
                            self.retain_capture("confirmed", image);
                            self.finish_operation(captured)?;
                        }
                    }
                    CaptureKind::Confirmed => {
                        self.retain_capture("confirmed", image);
                        self.finish_operation(captured)?;
                    }
                }
            }
            if self.index == 60 {
                self.outcome.lock().unwrap().completed = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return Ok(());
            }
            if self.warm_frames >= 8 && self.operation.is_none() && self.idle_frames == 0 {
                if self.client.persistence.pending()
                    || self.client.persistence.pending_timer_action.is_some()
                {
                    return Err("unexpected pending write before native action".into());
                }
                let slot = self
                    .client
                    .data
                    .slots
                    .first()
                    .ok_or("no synthetic timer")?
                    .clone();
                let start = self.index % 2 == 0;
                if slot.running_since_epoch_millis.is_some() == start {
                    return Err("unexpected timer state before dispatch".into());
                }
                let click = Instant::now();
                self.client.toggle_slot(&slot, ctx);
                let dispatch_return = Instant::now();
                let action_id = self
                    .client
                    .persistence
                    .pending_timer_action
                    .as_ref()
                    .ok_or("native action rejected")?
                    .id;
                self.operation = Some(Operation {
                    index: self.index,
                    start,
                    action_id,
                    click,
                    dispatch_return,
                    feedback_update: None,
                    feedback_rendered: None,
                    confirmed_update: None,
                    confirmed_frame_work_micros: None,
                    feedback_was_pending: true,
                });
            }
            // This is the full production update, including its native Frame adapters.
            let update_started = Instant::now();
            eframe::App::update(&mut self.client, ctx, frame);
            let updated = Instant::now();
            if !self.client.workspace_persistence_ready {
                return Err(format!(
                    "workspace became read-only: {}",
                    self.client.status
                ));
            }
            self.warm_frames += 1;
            if self.idle_frames > 0 {
                self.idle_frames -= 1;
            }
            if let Some(operation) = self.operation.as_mut() {
                let confirmed = self.client.persistence.pending_timer_action.is_none()
                    && !self.client.persistence.pending()
                    && self.client.data.slots.first().is_some_and(|slot| {
                        slot.running_since_epoch_millis.is_some() == operation.start
                    });
                if confirmed && operation.confirmed_update.is_none() {
                    operation.confirmed_update = Some(updated);
                    operation.confirmed_frame_work_micros = Some(micros(update_started, updated));
                }
                if operation.feedback_update.is_none() {
                    operation.feedback_update = Some(updated);
                    operation.feedback_was_pending = !confirmed;
                    self.capture = Some(CaptureKind::Feedback);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
                } else if operation.feedback_rendered.is_some()
                    && self.capture.is_none()
                    && confirmed
                {
                    self.capture = Some(CaptureKind::Confirmed);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
                }
            }
            ctx.request_repaint_after(Duration::from_millis(16));
            Ok(())
        }
    }

    impl eframe::App for NativeTimerApp {
        fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
            if let Err(error) = self.step(ctx, frame) {
                self.outcome.lock().unwrap().error = Some(error);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
            eframe::App::on_exit(&mut self.client, gl);
        }
    }

    #[test]
    #[ignore = "Explicit isolated real native window timer benchmark; closes its own viewport"]
    fn timer_native_window_end_to_end_benchmark() {
        assert!(!cfg!(debug_assertions), "use the release test binary");
        let root = timer_action_performance_tests::fixture_root();
        let local_app_data = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap());
        assert!(
            local_app_data.starts_with(&root),
            "LOCALAPPDATA must be isolated below the synthetic fixture"
        );
        let output = PathBuf::from(
            std::env::var_os("GRIDTIMER_TIMER_NATIVE_OUTPUT").expect("native output directory"),
        );
        assert!(
            output.is_absolute() && !output.exists(),
            "use a new absolute output directory"
        );
        fs::create_dir_all(&output).unwrap();
        let client = timer_action_performance_tests::make_client(&root, 68);
        let initial_sessions = client.data.sessions.len();
        let initial_notes = serde_json::to_vec(&client.data.notes).unwrap();
        let state_bytes = client.state_json.len();
        let journal_bytes = fs::metadata(desktop_state_store_path(&root)).unwrap().len();
        let outcome = Arc::new(Mutex::new(Outcome::default()));
        let app_outcome = outcome.clone();
        let variant = std::env::var("GRIDTIMER_TIMER_BENCHMARK_VARIANT")
            .unwrap_or_else(|_| "unspecified".into());
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1280.0, 900.0])
                .with_position([40.0, 40.0]),
            event_loop_builder: Some(Box::new(|builder| {
                builder.with_any_thread(true);
            })),
            persist_window: false,
            ..Default::default()
        };
        eframe::run_native(
            "Timer latency benchmark",
            options,
            Box::new(move |cc| {
                install_ui_fonts(&cc.egui_ctx);
                install_ui_style(&cc.egui_ctx);
                Box::new(NativeTimerApp {
                    client,
                    outcome: app_outcome,
                    variant,
                    started: Instant::now(),
                    warm_frames: 0,
                    idle_frames: 0,
                    index: 0,
                    operation: None,
                    capture: None,
                    state_bytes,
                    journal_bytes,
                })
            }),
        )
        .unwrap();
        let result = outcome.lock().unwrap();
        fs::write(
            output.join("native_timer_samples.json"),
            serde_json::to_vec_pretty(&result.samples).unwrap(),
        )
        .unwrap();
        fs::write(output.join("native_timer_completion.json"), serde_json::to_vec_pretty(&json!({
            "completed": result.completed, "error": result.error, "samples": result.samples.len(),
            "programmaticDispatch": true, "productionNativeUpdate": true,
            "captureTiming": "screenshot event arrival upper bound; includes GPU readback and event delivery"
        })).unwrap()).unwrap();
        for (name, image) in &result.images {
            let pixels: Vec<u8> = image
                .pixels
                .iter()
                .flat_map(|pixel| pixel.to_array())
                .collect();
            image::save_buffer(
                output.join(name),
                &pixels,
                image.width() as u32,
                image.height() as u32,
                image::ColorType::Rgba8,
            )
            .unwrap();
        }
        assert!(result.error.is_none(), "{:?}", result.error);
        assert!(
            result.completed && result.samples.len() == 60,
            "native window closed before completion"
        );
        drop(result);
        let reopened =
            TimerWindowsClient::load_from_root(root.clone(), Arc::new(AtomicBool::new(false)));
        assert!(reopened.workspace_persistence_ready, "{}", reopened.status);
        assert!(reopened.sync.user_id.is_empty() && reopened.sync.token.is_empty());
        assert!(reopened
            .data
            .slots
            .iter()
            .all(|slot| slot.running_since_epoch_millis.is_none()));
        assert_eq!(reopened.data.sessions.len(), initial_sessions + 30);
        assert_eq!(
            serde_json::to_vec(&reopened.data.notes).unwrap(),
            initial_notes
        );
        fs::write(
            output.join("native_timer_reopen.json"),
            serde_json::to_vec_pretty(&json!({
                "verified": true, "signedOut": true, "runningSlots": 0,
                "additionalSessions": 30, "notesUnchanged": true
            }))
            .unwrap(),
        )
        .unwrap();
        drop(reopened);
        println!("\nTIMER_NATIVE_COMPLETE {}", output.display());
    }
}
