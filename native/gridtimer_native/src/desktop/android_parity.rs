// v1.0.3.17 Windows - Route timer detail controls through the responsive save path.
// v1.0.3.16 Windows - Reuse the frame's timer projection in the detail view.
// v2.22.48 - Match Android dock labels and open timer records without dropping unsaved edits.
// v2.22.47 - Complete timer details, drag ordering, sticky tools and Android dock languages.

static ACTIVE_DESKTOP_LANGUAGE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

#[derive(Default)]
struct AndroidParityUi {
    tray: DesktopTray,
    image_export: NoteImageExport,
    diagnostics: DesktopDiagnosticCapture,
    timer_stats_key: Option<(u64, i64)>,
    timer_stats: BTreeMap<i32, AndroidTimerStats>,
    outline_target: Option<String>,
}

#[derive(Clone, Default)]
struct AndroidTimerStats {
    today_millis: i64,
    today_count: usize,
    count: usize,
    longest: i64,
    recent: Vec<DesktopSession>,
}

#[derive(Clone)]
struct TimerDragPayload {
    slot_id: i32,
    workspace: String,
}

fn android_timer_stats(
    sessions: &[DesktopSession],
    window: (i64, i64),
) -> BTreeMap<i32, AndroidTimerStats> {
    let mut result = BTreeMap::<i32, AndroidTimerStats>::new();
    for session in sessions {
        let stats = result.entry(session.slot_id).or_default();
        stats.count += 1;
        stats.longest = stats.longest.max(session.duration_millis.max(0));
        if session.ended_at_epoch_millis >= window.0 && session.ended_at_epoch_millis < window.1 {
            stats.today_millis = stats
                .today_millis
                .saturating_add(session.duration_millis.max(0));
            stats.today_count += 1;
        }
        let position = stats
            .recent
            .partition_point(|other| other.ended_at_epoch_millis >= session.ended_at_epoch_millis);
        if position < 20 {
            stats.recent.insert(position, session.clone());
            stats.recent.truncate(20);
        }
    }
    result
}

fn reordered_timer_ids(order: &[i32], from: i32, to: i32) -> Option<Vec<i32>> {
    if from == to {
        return None;
    }
    let origin = order.iter().position(|id| *id == from)?;
    let destination = order.iter().position(|id| *id == to)?;
    let mut result = order.to_vec();
    result.remove(origin);
    result.insert(destination, from);
    Some(result)
}

fn desktop_language_code(preference: &str) -> u8 {
    match preference {
        "en" => 1,
        "ja" => 2,
        "zh" => 0,
        _ => {
            #[cfg(target_os = "windows")]
            {
                #[link(name = "kernel32")]
                extern "system" {
                    fn GetUserDefaultUILanguage() -> u16;
                }
                match unsafe { GetUserDefaultUILanguage() } & 0x3ff {
                    9 => 1,
                    17 => 2,
                    _ => 0,
                }
            }
            #[cfg(not(target_os = "windows"))]
            {
                0
            }
        }
    }
}

fn desktop_dock_label(tab: AppTab) -> &'static str {
    // Android currently localizes navigation and theme controls. User content
    // and the remaining Chinese editor labels are preserved on both platforms.
    let labels = match ACTIVE_DESKTOP_LANGUAGE.load(AtomicOrdering::Relaxed) {
        1 => [
            "Timer",
            "Quick notes",
            "Knowledge",
            "Timer records",
            "Risk Control",
            "Me",
        ],
        2 => ["計時", "付箋", "知識", "計時記録", "リスク管理", "Me"],
        _ => ["计时", "便签", "知识", "计时记录", "风控", "我的"],
    };
    labels[match tab {
        AppTab::Board => 0,
        AppTab::Notes => 1,
        AppTab::Knowledge => 2,
        AppTab::History => 3,
        AppTab::Finance => 4,
        AppTab::My => 5,
    }]
}

fn desktop_language_text<'a>(chinese: &'a str, english: &'a str, japanese: &'a str) -> &'a str {
    match ACTIVE_DESKTOP_LANGUAGE.load(AtomicOrdering::Relaxed) {
        1 => english,
        2 => japanese,
        _ => chinese,
    }
}

impl TimerWindowsClient {
    fn refresh_android_timer_summary(&mut self) {
        let window = desktop_local_day_window(now_millis(), 0);
        let key = (self.timers_cache_version(), window.0);
        if self.desktop_ui.parity.timer_stats_key != Some(key) {
            self.desktop_ui.parity.timer_stats = android_timer_stats(&self.data.sessions, window);
            self.desktop_ui.parity.timer_stats_key = Some(key);
        }
    }

    fn reorder_timer_from_drop(&mut self, payload: &TimerDragPayload, target: i32) -> bool {
        if self.workspace_edit_locked()
            || payload.workspace != self.background_job_workspace_fingerprint()
            || self.flush_slot_draft().is_err()
        {
            return false;
        }
        self.refresh_timer_projection(now_millis());
        let Some(projection) = &self.desktop_ui.projection else {
            return false;
        };
        let order = desktop_timer::ordered_slot_ids(
            &projection.slots.iter().map(|s| s.id).collect::<Vec<_>>(),
            &projection.slot_order,
        );
        let Some(order) = reordered_timer_ids(&order, payload.slot_id, target) else {
            return false;
        };
        self.replace_state(
            app_data::set_slot_order_app_data_json(&self.state_json, &order, now_millis()),
            "排序已保存",
        )
    }

    fn ui_android_timer_detail(&mut self, ui: &mut egui::Ui, slot: &DesktopSlot) {
        self.refresh_android_timer_summary();
        if self.desktop_ui.projection_version != self.timers_cache_version()
            || self.desktop_ui.projection.is_none()
        {
            self.refresh_timer_projection(now_millis());
        }
        let stats = self
            .desktop_ui
            .parity
            .timer_stats
            .get(&slot.id)
            .cloned()
            .unwrap_or_default();
        card_frame().show(ui, |ui| {
            if let Some(view) = self
                .desktop_ui
                .projection
                .as_ref()
                .and_then(|p| p.slots.iter().find(|s| s.id == slot.id))
            {
                ui.heading(format_duration(view.accumulated_millis));
                let phase = match view.micro_break_phase {
                    app_data::TimerViewPhase::Break => "微休息",
                    _ => "专注",
                };
                let remaining = view
                    .micro_break_phase_target_millis
                    .saturating_sub(view.micro_break_phase_progress_millis)
                    .max(0);
                ui.label(format!(
                    "{} · {phase}剩余 {}",
                    if view.is_running {
                        "进行中"
                    } else {
                        "已暂停"
                    },
                    format_duration(remaining)
                ));
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button(if slot.running_since_epoch_millis.is_some() {
                        "暂停计时"
                    } else {
                        "继续计时"
                    })
                    .clicked()
                {
                    self.toggle_slot(slot, ui.ctx());
                }
                let can_archive = self
                    .desktop_ui
                    .projection
                    .as_ref()
                    .and_then(|p| p.slots.iter().find(|s| s.id == slot.id))
                    .is_some_and(|view| !view.is_running && !desktop_timer::is_blank_timer(view));
                if ui
                    .add_enabled(can_archive, egui::Button::new("归档"))
                    .clicked()
                {
                    self.archive_slot(slot.id);
                }
                if ui.button("重置累计").clicked() {
                    self.request_timer_reset(slot.id);
                }
            });
            ui.separator();
            ui.columns(3, |columns| {
                for (column, (label, value)) in columns.iter_mut().zip([
                    ("今日计时", format_duration(stats.today_millis)),
                    ("最长一次", format_duration(stats.longest)),
                    (
                        "今日 / 全部",
                        format!("{} / {} 次", stats.today_count, stats.count),
                    ),
                ]) {
                    column.label(egui::RichText::new(label).size(11.0).color(palette().muted));
                    column.label(egui::RichText::new(value).strong());
                }
            });
            if let Some(last) = stats.recent.first() {
                finance_detail_row(
                    ui,
                    "最近结束",
                    &desktop_local_timestamp(last.ended_at_epoch_millis),
                );
            }
            egui::CollapsingHeader::new("近期记录").show(ui, |ui| {
                if stats.recent.is_empty() {
                    empty_state(ui, "还没有已结束的计时记录");
                }
                for session in &stats.recent {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(desktop_local_timestamp(session.ended_at_epoch_millis));
                        ui.label(format_duration(session.duration_millis));
                    });
                }
            });
            if ui.button("查看全部记录").clicked() {
                self.open_timer_history(Some(slot.id), false);
            }
        });
        ui.add_space(10.0);
    }

    fn ui_sticky_android_tools(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(!self.note_undo_stack.is_empty(), egui::Button::new("撤销"))
                .clicked()
            {
                self.undo_note_draft();
            }
            if ui
                .add_enabled(!self.note_redo_stack.is_empty(), egui::Button::new("重做"))
                .clicked()
            {
                self.redo_note_draft();
            }
            ui.menu_button("格式", |ui| {
                for (label, prefix) in [
                    ("标题", "# "),
                    ("编号", "1. "),
                    ("待办", "- [ ] "),
                    ("引用", "> "),
                ] {
                    if ui.button(label).clicked() {
                        self.apply_note_prefix(prefix);
                        ui.close_menu();
                    }
                }
                if ui.button("加粗").clicked() {
                    self.wrap_active_note_block_bold();
                    ui.close_menu();
                }
            });
            ui.menu_button("纸张与分类", |ui| self.ui_note_properties(ui, false));
            ui.menu_button("插入", |ui| self.ui_note_insert_menu(ui));
        });
    }

    fn ui_note_outline(&mut self, ui: &mut egui::Ui) {
        let headings = self
            .note_blocks_draft
            .iter()
            .filter_map(|block| {
                if !block_is_plain_text(block) {
                    return None;
                }
                let text = block
                    .text
                    .lines()
                    .find(|line| line.trim_start().starts_with('#'))?;
                let title = text.trim_start().trim_start_matches('#').trim();
                (!title.is_empty())
                    .then(|| (block.id.clone(), title.chars().take(80).collect::<String>()))
            })
            .collect::<Vec<_>>();
        if headings.is_empty() {
            return;
        }
        ui.menu_button("文档目录", |ui| {
            for (id, title) in headings {
                if ui.button(title).clicked() {
                    self.desktop_ui.parity.outline_target = Some(id);
                    ui.close_menu();
                }
            }
        });
    }

    fn ui_android_language(&mut self, ui: &mut egui::Ui) {
        let before = self.settings.language.clone();
        ui.horizontal_wrapped(|ui| {
            ui.label("语言");
            for (code, label) in [
                (
                    "",
                    desktop_language_text("跟随系统", "Follow system", "システムに従う"),
                ),
                ("zh", "简体中文"),
                ("en", "English"),
                ("ja", "日本語"),
            ] {
                ui.selectable_value(&mut self.settings.language, code.into(), label);
            }
        });
        if before != self.settings.language {
            if let Err(error) = self.persist_settings() {
                self.settings.language = before;
                self.status = format!("语言设置未保存：{error}");
            }
            ACTIVE_DESKTOP_LANGUAGE.store(
                desktop_language_code(&self.settings.language),
                AtomicOrdering::Relaxed,
            );
        }
    }
}
