use eframe::egui;
use gridtimer_native::ai_client;
use gridtimer_native::app_data;
use gridtimer_native::sync_core;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1180.0, 760.0]),
        ..Default::default()
    };
    eframe::run_native(
        "格子计时",
        options,
        Box::new(|cc| {
            install_ui_fonts(&cc.egui_ctx);
            install_ui_style(&cc.egui_ctx);
            Box::new(TimerWindowsClient::load())
        }),
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AppTab {
    Board,
    Notes,
    History,
    Finance,
    My,
}

impl AppTab {
    const ALL: [AppTab; 5] = [
        AppTab::Board,
        AppTab::Notes,
        AppTab::History,
        AppTab::Finance,
        AppTab::My,
    ];

    fn title(self) -> &'static str {
        match self {
            AppTab::Board => "计时台",
            AppTab::Notes => "便签",
            AppTab::History => "历史",
            AppTab::Finance => "风控",
            AppTab::My => "我的",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            AppTab::Board => "当下",
            AppTab::Notes => "手记",
            AppTab::History => "复盘",
            AppTab::Finance => "现金流",
            AppTab::My => "账户",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopAppData {
    #[serde(default)]
    categories: Vec<DesktopCategory>,
    #[serde(default)]
    slots: Vec<DesktopSlot>,
    #[serde(default)]
    sessions: Vec<DesktopSession>,
    #[serde(default)]
    archived_tasks: Vec<DesktopArchivedTask>,
    #[serde(default)]
    notes: Vec<DesktopNote>,
    #[serde(default)]
    finance_profile: DesktopFinanceProfile,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopCategory {
    id: String,
    name: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopSlot {
    id: i32,
    title: String,
    note: String,
    accumulated_millis: i64,
    running_since_epoch_millis: Option<i64>,
    updated_at: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopSession {
    id: String,
    slot_id: i32,
    slot_title: String,
    started_at_epoch_millis: i64,
    ended_at_epoch_millis: i64,
    duration_millis: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopArchivedTask {
    id: String,
    original_slot_id: i32,
    title: String,
    note: String,
    accumulated_millis: i64,
    archived_at_epoch_millis: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopNote {
    id: String,
    title: String,
    content: String,
    #[serde(default)]
    document: DesktopNoteDocument,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    created_at_epoch_millis: i64,
    #[serde(default)]
    updated_at_epoch_millis: i64,
    #[serde(default)]
    deleted_at_epoch_millis: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopNoteDocument {
    #[serde(default)]
    rich_text_plain_text: String,
    #[serde(default)]
    blocks: Vec<DesktopNoteBlock>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopNoteBlock {
    #[serde(default)]
    text: String,
    #[serde(default)]
    caption: String,
    #[serde(default)]
    contact_name: String,
    #[serde(default)]
    call_contact_name: String,
    #[serde(default)]
    call_phone_number: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopFinanceProfile {
    #[serde(default)]
    active_income_monthly: i64,
    #[serde(default)]
    asset_income_monthly: i64,
    #[serde(default)]
    living_expense_monthly: i64,
    #[serde(default)]
    liability_payment_monthly: i64,
    #[serde(default)]
    cash_reserve: i64,
    #[serde(default)]
    productive_asset_value: i64,
    #[serde(default)]
    liability_balance: i64,
    #[serde(flatten)]
    extra: serde_json::Map<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopSyncSession {
    server_url: String,
    email: String,
    user_id: String,
    token: String,
    device_name: String,
    last_sync_at_epoch_millis: i64,
    last_message: String,
    #[serde(default)]
    ai_api_key: String,
    #[serde(default = "default_ai_base_url")]
    ai_base_url: String,
    #[serde(default = "default_ai_model")]
    ai_model: String,
    #[serde(default)]
    ai_last_message: String,
}

impl Default for DesktopSyncSession {
    fn default() -> Self {
        Self {
            server_url: sync_core::default_server_url(),
            email: String::new(),
            user_id: String::new(),
            token: String::new(),
            device_name: default_device_name(),
            last_sync_at_epoch_millis: 0,
            last_message: "未登录".to_string(),
            ai_api_key: String::new(),
            ai_base_url: default_ai_base_url(),
            ai_model: default_ai_model(),
            ai_last_message: String::new(),
        }
    }
}

struct AiNoteTaskResult {
    result: ai_client::AiCompletionResult,
}

struct TimerWindowsClient {
    tab: AppTab,
    root_dir: PathBuf,
    state_path: PathBuf,
    sync_path: PathBuf,
    state_json: String,
    data: DesktopAppData,
    sync: DesktopSyncSession,
    selected_slot_id: i32,
    selected_note_id: String,
    note_title_draft: String,
    note_content_draft: String,
    note_search_draft: String,
    password_draft: String,
    ai_instruction_draft: String,
    ai_pending: bool,
    ai_result_rx: Option<mpsc::Receiver<AiNoteTaskResult>>,
    status: String,
}

impl TimerWindowsClient {
    fn load() -> Self {
        let root_dir = app_dir();
        let state_path = root_dir.join("timer_state.json");
        let sync_path = root_dir.join("sync_account.json");
        let now = now_millis();
        let fallback_json = app_data::default_app_data_json(now);
        let state_json = fs::read_to_string(&state_path)
            .ok()
            .and_then(|raw| app_data::sanitize_app_data_json(&raw, now))
            .unwrap_or(fallback_json);
        let data = decode_data(&state_json);
        let sync = fs::read_to_string(&sync_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<DesktopSyncSession>(&raw).ok())
            .unwrap_or_default();
        let selected_slot_id = data.slots.first().map(|slot| slot.id).unwrap_or(1);
        let app = Self {
            tab: AppTab::Board,
            root_dir,
            state_path,
            sync_path,
            state_json,
            data,
            sync,
            selected_slot_id,
            selected_note_id: String::new(),
            note_title_draft: String::new(),
            note_content_draft: String::new(),
            note_search_draft: String::new(),
            password_draft: String::new(),
            ai_instruction_draft: "整理这篇笔记，保留事实，补足结构。".to_string(),
            ai_pending: false,
            ai_result_rx: None,
            status: String::new(),
        };
        app.save_state();
        app
    }

    fn replace_state(&mut self, next_json: Option<String>, status: &str) {
        if let Some(next_json) = next_json {
            let now = now_millis();
            if let Some(sanitized) = app_data::sanitize_app_data_json(&next_json, now) {
                self.state_json = sanitized;
                self.data = decode_data(&self.state_json);
                self.save_state();
                self.status = status.to_string();
            }
        }
    }

    fn save_state(&self) {
        let _ = fs::create_dir_all(&self.root_dir);
        let _ = fs::write(&self.state_path, &self.state_json);
    }

    fn save_sync(&self) {
        let _ = fs::create_dir_all(&self.root_dir);
        if let Ok(encoded) = serde_json::to_string_pretty(&self.sync) {
            let _ = fs::write(&self.sync_path, encoded);
        }
    }

    fn selected_slot(&self) -> Option<DesktopSlot> {
        self.data
            .slots
            .iter()
            .find(|slot| slot.id == self.selected_slot_id)
            .cloned()
    }

    fn update_slot_title(&mut self, slot_id: i32, title: &str) {
        self.replace_state(
            app_data::update_slot_title_app_data_json(
                &self.state_json,
                slot_id,
                title,
                now_millis(),
            ),
            "已保存标题",
        );
    }

    fn update_slot_note(&mut self, slot_id: i32, note: &str) {
        self.replace_state(
            app_data::update_slot_note_app_data_json(&self.state_json, slot_id, note, now_millis()),
            "已保存备注",
        );
    }

    fn toggle_slot(&mut self, slot: &DesktopSlot) {
        let next = if slot.running_since_epoch_millis.is_some() {
            app_data::pause_slots_app_data_json(&self.state_json, &[slot.id], now_millis())
        } else {
            app_data::start_slot_app_data_json(&self.state_json, slot.id, now_millis())
        };
        self.replace_state(next, "计时已更新");
    }

    fn reset_slot(&mut self, slot_id: i32) {
        self.replace_state(
            app_data::reset_slot_app_data_json(&self.state_json, slot_id, now_millis()),
            "已重置",
        );
    }

    fn archive_slot(&mut self, slot_id: i32) {
        let archive_id = format!("archive-{}-{slot_id}", now_millis());
        self.replace_state(
            app_data::archive_slot_app_data_json(
                &self.state_json,
                slot_id,
                &archive_id,
                now_millis(),
            ),
            "已归档",
        );
    }

    fn pause_running_slots(&mut self) {
        let running_ids = self
            .data
            .slots
            .iter()
            .filter(|slot| slot.running_since_epoch_millis.is_some())
            .map(|slot| slot.id)
            .collect::<Vec<_>>();
        if running_ids.is_empty() {
            self.status = "现在没有运行中的格子".to_string();
            return;
        }
        self.replace_state(
            app_data::pause_slots_app_data_json(&self.state_json, &running_ids, now_millis()),
            "已暂停全部",
        );
    }

    fn select_note(&mut self, note: &DesktopNote) {
        self.selected_note_id = note.id.clone();
        self.note_title_draft = note.title.clone();
        self.note_content_draft = note_body_text(note);
    }

    fn new_note(&mut self) {
        self.selected_note_id.clear();
        self.note_title_draft.clear();
        self.note_content_draft.clear();
    }

    fn save_note(&mut self) {
        let now = now_millis();
        let note_id = if self.selected_note_id.trim().is_empty() {
            format!("note-{now}")
        } else {
            self.selected_note_id.clone()
        };
        let existing = self
            .data
            .notes
            .iter()
            .find(|note| note.id == note_id)
            .cloned();
        let note = json!({
            "id": note_id,
            "title": self.note_title_draft,
            "content": self.note_content_draft,
            "kind": "STICKY",
            "accentSeed": "amber",
            "pinned": existing.as_ref().map(|note| note.pinned).unwrap_or(false),
            "createdAtEpochMillis": existing.as_ref().map(|note| note.created_at_epoch_millis).unwrap_or(now),
            "updatedAtEpochMillis": now
        });
        let note_json = serde_json::to_string(&note).unwrap_or_else(|_| "{}".to_string());
        self.replace_state(
            app_data::upsert_note_app_data_json(&self.state_json, &note_json, now),
            "便签已保存",
        );
        self.selected_note_id = note["id"].as_str().unwrap_or_default().to_string();
    }

    fn delete_note(&mut self) {
        if self.selected_note_id.trim().is_empty() {
            return;
        }
        self.replace_state(
            app_data::delete_note_app_data_json(
                &self.state_json,
                &self.selected_note_id,
                now_millis(),
            ),
            "便签已移入回收站",
        );
        self.new_note();
    }

    fn update_finance_profile(&mut self, next: DesktopFinanceProfile) {
        let profile_json = serde_json::to_string(&next).unwrap_or_else(|_| "{}".to_string());
        self.replace_state(
            app_data::update_finance_profile_app_data_json(
                &self.state_json,
                &profile_json,
                now_millis(),
            ),
            "财务已保存",
        );
    }

    fn register_account(&mut self) {
        let result = sync_core::register_account_json(
            &self.sync.server_url,
            &self.sync.email,
            &self.password_draft,
            &self.sync.device_name,
        );
        let result = self.apply_sync_result(&result, true);
        if result.ok {
            self.sync_now();
        }
    }

    fn login_account(&mut self) {
        let result = sync_core::login_account_json(
            &self.sync.server_url,
            &self.sync.email,
            &self.password_draft,
            &self.sync.device_name,
        );
        let result = self.apply_sync_result(&result, true);
        if result.ok {
            self.sync_now();
        }
    }

    fn sync_now(&mut self) {
        let revision = sync_core::app_data_revision_millis(&self.state_json, now_millis());
        let result = sync_core::sync_app_data_json(
            &self.sync.server_url,
            &self.sync.token,
            &self.state_json,
            revision,
            &self.sync.device_name,
        );
        self.apply_sync_result(&result, false);
    }

    fn apply_sync_result(&mut self, raw: &str, keep_password: bool) -> sync_core::SyncClientResult {
        let result =
            serde_json::from_str::<sync_core::SyncClientResult>(raw).unwrap_or_else(|_| {
                sync_core::SyncClientResult {
                    ok: false,
                    message: "同步响应无法解析".to_string(),
                    ..Default::default()
                }
        });
        self.sync.last_message = desktop_sync_message(&result);
        if !result.resolved_server_url.trim().is_empty() {
            self.sync.server_url = result.resolved_server_url.clone();
        }
        if result.ok {
            if !result.user_id.is_empty() {
                self.sync.user_id = result.user_id.clone();
            }
            if !result.token.is_empty() {
                self.sync.token = result.token.clone();
            }
            self.sync.last_sync_at_epoch_millis = now_millis();
            if let Some(app_data_json) = result.app_data_json.clone() {
                self.replace_state(Some(app_data_json), "同步完成");
            }
            if !keep_password {
                self.password_draft.clear();
            }
        }
        self.status = self.sync.last_message.clone();
        self.save_sync();
        result
    }

    fn logout(&mut self) {
        let server_url = self.sync.server_url.clone();
        let device_name = self.sync.device_name.clone();
        self.sync = DesktopSyncSession {
            server_url,
            device_name,
            last_message: "已退出登录".to_string(),
            ..DesktopSyncSession::default()
        };
        self.password_draft.clear();
        self.save_sync();
    }

    fn launch_note_ai(&mut self) {
        if self.ai_pending {
            self.status = "模型正在处理上一条笔记".to_string();
            return;
        }
        if self.sync.ai_api_key.trim().is_empty() {
            self.status = "请先在“我的”里填写 OpenAI API Key".to_string();
            return;
        }
        let title = self.note_title_draft.clone();
        let content = self.note_content_draft.clone();
        if title.trim().is_empty() && content.trim().is_empty() {
            self.status = "先写一点内容，再调用模型".to_string();
            return;
        }
        let api_key = self.sync.ai_api_key.clone();
        let base_url = self.sync.ai_base_url.clone();
        let model = self.sync.ai_model.clone();
        let instruction = self.ai_instruction_draft.clone();
        let (tx, rx) = mpsc::channel();
        self.ai_pending = true;
        self.ai_result_rx = Some(rx);
        self.status = "模型正在整理笔记...".to_string();
        std::thread::spawn(move || {
            let result = ai_client::complete_note(
                &api_key,
                &base_url,
                &model,
                &title,
                &content,
                &instruction,
            );
            let _ = tx.send(AiNoteTaskResult { result });
        });
    }

    fn poll_ai_result(&mut self) {
        let Some(rx) = self.ai_result_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(task) => {
                self.ai_pending = false;
                self.sync.ai_last_message = task.result.message.clone();
                if task.result.ok {
                    self.note_content_draft = task.result.content;
                    self.status = "模型已写回正文".to_string();
                    self.save_note();
                } else {
                    self.status = task.result.message;
                }
                self.save_sync();
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.ai_result_rx = Some(rx);
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.ai_pending = false;
                self.status = "模型请求已中断".to_string();
            }
        }
    }
}

impl eframe::App for TimerWindowsClient {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(500));
        self.poll_ai_result();

        let p = palette();
        egui::SidePanel::left("app_nav")
            .exact_width(212.0)
            .resizable(false)
            .frame(
                egui::Frame::none()
                    .fill(p.nav)
                    .inner_margin(egui::Margin::symmetric(18.0, 20.0)),
            )
            .show(ctx, |ui| self.ui_nav(ui));

        egui::TopBottomPanel::top("page_header")
            .exact_height(78.0)
            .frame(
                egui::Frame::none()
                    .fill(p.bg)
                    .inner_margin(egui::Margin::symmetric(22.0, 14.0)),
            )
            .show(ctx, |ui| self.ui_header(ui));

        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(34.0)
            .frame(
                egui::Frame::none()
                    .fill(p.bg)
                    .inner_margin(egui::Margin::symmetric(22.0, 6.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(if self.status.is_empty() {
                            "本地数据已就绪"
                        } else {
                            &self.status
                        })
                        .size(12.0)
                        .color(p.muted),
                    );
                });
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(p.bg)
                    .inner_margin(egui::Margin::symmetric(22.0, 12.0)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.tab {
                        AppTab::Board => self.ui_board(ui),
                        AppTab::Notes => self.ui_notes(ui),
                        AppTab::History => self.ui_history(ui),
                        AppTab::Finance => self.ui_finance(ui),
                        AppTab::My => self.ui_my(ui),
                    });
            });
    }
}

impl TimerWindowsClient {
    fn ui_nav(&mut self, ui: &mut egui::Ui) {
        let p = palette();
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("格子计时")
                .size(24.0)
                .strong()
                .color(p.text),
        );
        ui.add_space(2.0);
        ui.label(egui::RichText::new("Windows").size(12.0).color(p.muted));
        ui.add_space(26.0);

        for tab in AppTab::ALL {
            if nav_button(ui, self.tab == tab, tab.title()).clicked() {
                self.tab = tab;
            }
            ui.add_space(7.0);
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            ui.label(
                egui::RichText::new(if self.sync.token.is_empty() {
                    "未登录"
                } else {
                    "已登录"
                })
                .size(13.0)
                .color(if self.sync.token.is_empty() {
                    p.warn
                } else {
                    p.good
                }),
            );
            ui.add_space(8.0);
            pill(ui, "本地保存", p.panel_alt, p.muted);
        });
    }

    fn ui_header(&mut self, ui: &mut egui::Ui) {
        let p = palette();
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(self.tab.title())
                        .size(25.0)
                        .strong()
                        .color(p.text),
                );
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(self.tab.subtitle())
                        .size(13.0)
                        .color(p.muted),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if action_button(ui, "暂停全部", ButtonTone::Quiet).clicked() {
                    self.pause_running_slots();
                }
                ui.add_space(8.0);
                pill(ui, &active_summary(&self.data), p.panel_alt, p.text);
            });
        });
    }

    fn ui_board(&mut self, ui: &mut egui::Ui) {
        let running = self
            .data
            .slots
            .iter()
            .filter(|slot| slot.running_since_epoch_millis.is_some())
            .count();
        let total = self
            .data
            .slots
            .iter()
            .map(slot_elapsed_millis)
            .sum::<i64>();
        ui.columns(4, |columns| {
            metric_tile(&mut columns[0], "运行中", &running.to_string(), palette().accent);
            metric_tile(
                &mut columns[1],
                "格子累计",
                &format_duration(total),
                palette().good,
            );
            metric_tile(
                &mut columns[2],
                "历史",
                &self.data.sessions.len().to_string(),
                palette().blue,
            );
            metric_tile(
                &mut columns[3],
                "归档",
                &self.data.archived_tasks.len().to_string(),
                palette().warn,
            );
        });
        ui.add_space(16.0);

        let slots = self.data.slots.clone();
        let columns = board_columns(ui.available_width());
        let gap = 12.0;
        let card_width = ((ui.available_width() - gap * (columns.saturating_sub(1) as f32))
            / columns as f32)
            .max(250.0);
        for row in slots.chunks(columns) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for slot in row {
                    self.ui_slot_card(ui, slot, card_width);
                }
            });
            ui.add_space(12.0);
        }
        if slots.is_empty() {
            empty_state(ui, "还没有格子");
        }

        ui.add_space(8.0);
        if let Some(slot) = self.selected_slot() {
            self.ui_slot_editor(ui, &slot);
        }
    }

    fn ui_slot_card(&mut self, ui: &mut egui::Ui, slot: &DesktopSlot, width: f32) {
        let p = palette();
        let selected = self.selected_slot_id == slot.id;
        let mut frame = card_frame();
        if selected {
            frame = frame
                .fill(p.selected)
                .stroke(egui::Stroke::new(1.0, p.accent));
        }
        let response = frame
            .show(ui, |ui| {
                ui.set_width((width - 30.0).max(210.0));
                ui.horizontal(|ui| {
                    pill(ui, &format!("{:02}", slot.id), p.panel_alt, p.muted);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        pill(
                            ui,
                            if slot.running_since_epoch_millis.is_some() {
                                "运行"
                            } else {
                                "待命"
                            },
                            if slot.running_since_epoch_millis.is_some() {
                                p.accent_soft
                            } else {
                                p.panel_alt
                            },
                            if slot.running_since_epoch_millis.is_some() {
                                p.accent
                            } else {
                                p.muted
                            },
                        );
                    });
                });
                ui.add_space(10.0);
                let title = if slot.title.trim().is_empty() {
                    format!("格子 {:02}", slot.id)
                } else {
                    slot.title.clone()
                };
                ui.label(
                    egui::RichText::new(title)
                        .size(17.0)
                        .strong()
                        .color(p.text),
                );
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(slot_duration_label(slot))
                        .size(30.0)
                        .strong()
                        .color(if slot.running_since_epoch_millis.is_some() {
                            p.accent
                        } else {
                            p.text
                        }),
                );
                ui.add_space(4.0);
                ui.add(
                    egui::ProgressBar::new(slot_progress(slot))
                        .desired_width(ui.available_width())
                        .fill(if slot.running_since_epoch_millis.is_some() {
                            p.accent
                        } else {
                            p.line
                        }),
                );
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(preview_text(&slot.note, "还没有备注"))
                        .size(13.0)
                        .color(p.muted),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let running = slot.running_since_epoch_millis.is_some();
                    if action_button(
                        ui,
                        if running { "暂停" } else { "开始" },
                        ButtonTone::Primary,
                    )
                    .clicked()
                    {
                        self.toggle_slot(slot);
                    }
                    if action_button(ui, "重置", ButtonTone::Quiet).clicked() {
                        self.reset_slot(slot.id);
                    }
                    if action_button(ui, "归档", ButtonTone::Quiet).clicked() {
                        self.archive_slot(slot.id);
                    }
                });
            })
            .response;
        if response.clicked() {
            self.selected_slot_id = slot.id;
        }
    }

    fn ui_slot_editor(&mut self, ui: &mut egui::Ui, slot: &DesktopSlot) {
        card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("格子 {:02}", slot.id))
                        .size(18.0)
                        .strong()
                        .color(palette().text),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    pill(ui, &format_duration(slot_elapsed_millis(slot)), palette().panel_alt, palette().muted);
                });
            });
            ui.add_space(12.0);
            let mut title = slot.title.clone();
            ui.label(egui::RichText::new("名称").size(12.0).color(palette().muted));
            if ui
                .add(
                    egui::TextEdit::singleline(&mut title)
                        .desired_width(f32::INFINITY)
                        .hint_text("格子名称"),
                )
                .changed()
            {
                self.update_slot_title(slot.id, &title);
            }
            ui.add_space(10.0);
            let mut note = slot.note.clone();
            ui.label(egui::RichText::new("备注").size(12.0).color(palette().muted));
            if ui
                .add(
                    egui::TextEdit::multiline(&mut note)
                        .desired_width(f32::INFINITY)
                        .desired_rows(4),
                )
                .changed()
            {
                self.update_slot_note(slot.id, &note);
            }
        });
    }

    fn ui_notes(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if action_button(ui, "新建", ButtonTone::Primary).clicked() {
                self.new_note();
            }
            if action_button(ui, "保存", ButtonTone::Quiet).clicked() {
                self.save_note();
            }
            if action_button(ui, "删除", ButtonTone::Danger).clicked() {
                self.delete_note();
            }
            if ui
                .add_enabled_ui(!self.ai_pending, |ui| {
                    action_button(ui, "模型整理", ButtonTone::Quiet)
                })
                .inner
                .clicked()
            {
                self.launch_note_ai();
            }
        });
        ui.add_space(12.0);

        let width = ui.available_width();
        let list_width = (width * 0.32).clamp(260.0, 380.0);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_width(list_width);
                ui.add(
                    egui::TextEdit::singleline(&mut self.note_search_draft)
                        .desired_width(f32::INFINITY)
                        .hint_text("搜索便签"),
                );
                ui.add_space(10.0);
                let query = self.note_search_draft.trim().to_lowercase();
                let notes = self
                    .data
                    .notes
                    .clone()
                    .into_iter()
                    .filter(|note| note.deleted_at_epoch_millis.is_none())
                    .filter(|note| {
                        if query.is_empty() {
                            return true;
                        }
                        format!("{} {}", note.title, note_body_text(note))
                            .to_lowercase()
                            .contains(&query)
                    })
                    .collect::<Vec<_>>();
                egui::ScrollArea::vertical()
                    .max_height(650.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for note in notes {
                            let selected = self.selected_note_id == note.id;
                            if note_row(ui, &note, selected).clicked() {
                                self.select_note(&note);
                            }
                            ui.add_space(8.0);
                        }
                        if self.data.notes.is_empty() {
                            empty_state(ui, "还没有便签");
                        }
                    });
            });

            ui.add_space(12.0);
            ui.vertical(|ui| {
                self.ui_note_editor(ui);
            });
        });
    }

    fn ui_note_editor(&mut self, ui: &mut egui::Ui) {
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new("标题").size(12.0).color(palette().muted));
            ui.add(
                egui::TextEdit::singleline(&mut self.note_title_draft)
                    .desired_width(f32::INFINITY)
                    .hint_text("标题"),
            );
            ui.add_space(10.0);
            ui.label(
                egui::RichText::new("模型要求")
                    .size(12.0)
                    .color(palette().muted),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.ai_instruction_draft)
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(10.0);
            ui.label(egui::RichText::new("内容").size(12.0).color(palette().muted));
            let response = ui.add(
                egui::TextEdit::multiline(&mut self.note_content_draft)
                    .desired_width(f32::INFINITY)
                    .desired_rows(24),
            );
            if response.changed() && self.note_content_draft.ends_with("  ") {
                self.note_content_draft.pop();
                self.launch_note_ai();
            }
            if self.ai_pending {
                ui.add_space(8.0);
                pill(ui, "正在等待模型返回", palette().accent_soft, palette().accent);
            }
        });
    }

    fn ui_history(&mut self, ui: &mut egui::Ui) {
        let total = self
            .data
            .sessions
            .iter()
            .map(|session| session.duration_millis)
            .sum::<i64>();
        let last = self
            .data
            .sessions
            .iter()
            .max_by_key(|session| session.ended_at_epoch_millis)
            .map(|session| format_relative_time(session.ended_at_epoch_millis))
            .unwrap_or_else(|| "无".to_string());
        ui.columns(3, |columns| {
            metric_tile(
                &mut columns[0],
                "记录",
                &self.data.sessions.len().to_string(),
                palette().blue,
            );
            metric_tile(
                &mut columns[1],
                "总时长",
                &format_duration(total),
                palette().good,
            );
            metric_tile(&mut columns[2], "最近", &last, palette().warn);
        });
        ui.add_space(16.0);

        let mut sessions = self.data.sessions.clone();
        sessions.sort_by(|left, right| {
            right
                .ended_at_epoch_millis
                .cmp(&left.ended_at_epoch_millis)
                .then_with(|| right.started_at_epoch_millis.cmp(&left.started_at_epoch_millis))
        });
        for session in sessions {
            history_row(ui, &session);
            ui.add_space(8.0);
        }
        if self.data.sessions.is_empty() {
            empty_state(ui, "还没有历史记录");
        }
    }

    fn ui_finance(&mut self, ui: &mut egui::Ui) {
        let mut profile = self.data.finance_profile.clone();
        let income = profile.active_income_monthly + profile.asset_income_monthly;
        let outflow = profile.living_expense_monthly + profile.liability_payment_monthly;
        let net = income - outflow;
        let reserve_months = if outflow > 0 {
            format!("{:.1} 月", profile.cash_reserve as f64 / outflow as f64)
        } else {
            "∞".to_string()
        };
        ui.columns(4, |columns| {
            metric_tile(&mut columns[0], "月收入", &money_label(income), palette().good);
            metric_tile(&mut columns[1], "月支出", &money_label(outflow), palette().warn);
            metric_tile(
                &mut columns[2],
                "月净额",
                &money_label(net),
                if net >= 0 {
                    palette().accent
                } else {
                    palette().danger
                },
            );
            metric_tile(&mut columns[3], "现金垫", &reserve_months, palette().blue);
        });
        ui.add_space(16.0);

        let mut changed = false;
        card_frame().show(ui, |ui| {
            egui::Grid::new("finance_grid")
                .num_columns(2)
                .spacing([18.0, 12.0])
                .show(ui, |ui| {
                    changed |= money_field(ui, "主动收入/月", &mut profile.active_income_monthly);
                    ui.end_row();
                    changed |= money_field(ui, "资产收入/月", &mut profile.asset_income_monthly);
                    ui.end_row();
                    changed |= money_field(ui, "生活支出/月", &mut profile.living_expense_monthly);
                    ui.end_row();
                    changed |=
                        money_field(ui, "负债还款/月", &mut profile.liability_payment_monthly);
                    ui.end_row();
                    changed |= money_field(ui, "现金储备", &mut profile.cash_reserve);
                    ui.end_row();
                    changed |= money_field(ui, "生产资产", &mut profile.productive_asset_value);
                    ui.end_row();
                    changed |= money_field(ui, "负债余额", &mut profile.liability_balance);
                    ui.end_row();
                });
        });
        if changed {
            self.update_finance_profile(profile);
        }
    }

    fn ui_my(&mut self, ui: &mut egui::Ui) {
        ui.columns(2, |columns| {
            card_frame().show(&mut columns[0], |ui| {
                ui.label(
                    egui::RichText::new("账号")
                        .size(18.0)
                        .strong()
                        .color(palette().text),
                );
                ui.add_space(10.0);
                pill(
                    ui,
                    if self.sync.token.is_empty() {
                        "未登录"
                    } else {
                        "已登录"
                    },
                    palette().panel_alt,
                    if self.sync.token.is_empty() {
                        palette().warn
                    } else {
                        palette().good
                    },
                );
                ui.add_space(14.0);
                if text_field_row(ui, "同步地址", &mut self.sync.server_url, false) {
                    self.save_sync();
                }
                if text_field_row(ui, "邮箱", &mut self.sync.email, false) {
                    self.save_sync();
                }
                if text_field_row(ui, "设备名", &mut self.sync.device_name, false) {
                    self.save_sync();
                }
                ui.add_space(8.0);
                text_field_row(ui, "密码", &mut self.password_draft, true);
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if action_button(ui, "注册", ButtonTone::Primary).clicked() {
                        self.register_account();
                    }
                    if action_button(ui, "登录", ButtonTone::Quiet).clicked() {
                        self.login_account();
                    }
                    if action_button(ui, "同步", ButtonTone::Quiet).clicked() {
                        self.sync_now();
                    }
                    if action_button(ui, "退出", ButtonTone::Danger).clicked() {
                        self.logout();
                    }
                });
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new(&self.sync.last_message)
                        .size(12.0)
                        .color(palette().muted),
                );
            });

            card_frame().show(&mut columns[1], |ui| {
                ui.label(
                    egui::RichText::new("模型")
                        .size(18.0)
                        .strong()
                        .color(palette().text),
                );
                ui.add_space(10.0);
                if text_field_row(ui, "API Key", &mut self.sync.ai_api_key, true) {
                    self.save_sync();
                }
                if text_field_row(ui, "接口地址", &mut self.sync.ai_base_url, false) {
                    self.save_sync();
                }
                if text_field_row(ui, "模型", &mut self.sync.ai_model, false) {
                    self.save_sync();
                }
                if !self.sync.ai_last_message.is_empty() {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(&self.sync.ai_last_message)
                            .size(12.0)
                            .color(palette().muted),
                    );
                }
            });
        });
    }
}

fn nav_button(ui: &mut egui::Ui, selected: bool, label: &str) -> egui::Response {
    let p = palette();
    ui.add_sized(
        [ui.available_width(), 40.0],
        egui::Button::new(
            egui::RichText::new(label)
                .size(15.0)
                .strong()
                .color(if selected { dark_ink() } else { p.text }),
        )
        .fill(if selected { p.accent } else { p.nav })
        .stroke(if selected {
            egui::Stroke::NONE
        } else {
            egui::Stroke::new(1.0, p.line)
        })
        .rounding(8.0),
    )
}

fn action_button(ui: &mut egui::Ui, label: &str, tone: ButtonTone) -> egui::Response {
    let p = palette();
    let (fill, stroke, text) = match tone {
        ButtonTone::Primary => (p.accent, egui::Stroke::NONE, dark_ink()),
        ButtonTone::Quiet => (p.panel_alt, egui::Stroke::new(1.0, p.line), p.text),
        ButtonTone::Danger => (p.danger_soft, egui::Stroke::new(1.0, p.danger), p.danger),
    };
    ui.add(
        egui::Button::new(egui::RichText::new(label).size(13.0).strong().color(text))
            .fill(fill)
            .stroke(stroke)
            .rounding(7.0)
            .min_size(egui::vec2(70.0, 32.0)),
    )
}

fn metric_tile(ui: &mut egui::Ui, label: &str, value: &str, accent: egui::Color32) {
    card_frame().show(ui, |ui| {
        ui.set_min_height(72.0);
        ui.label(
            egui::RichText::new(label)
                .size(12.0)
                .strong()
                .color(palette().muted),
        );
        ui.add_space(7.0);
        ui.label(
            egui::RichText::new(value)
                .size(21.0)
                .strong()
                .color(accent),
        );
    });
}

fn note_row(ui: &mut egui::Ui, note: &DesktopNote, selected: bool) -> egui::Response {
    let p = palette();
    let body = note_body_text(note);
    let mut frame = card_frame();
    frame = frame.fill(if selected { p.selected } else { p.panel });
    if selected {
        frame = frame.stroke(egui::Stroke::new(1.0, p.accent));
    }
    frame
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let title = if note.title.trim().is_empty() {
                preview_text(&body, "未命名")
            } else {
                note.title.clone()
            };
            ui.label(
                egui::RichText::new(title)
                    .size(15.0)
                    .strong()
                    .color(p.text),
            );
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(preview_text(&body, "空白"))
                    .size(12.0)
                    .color(p.muted),
            );
        })
        .response
}

fn history_row(ui: &mut egui::Ui, session: &DesktopSession) {
    card_frame().show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(session_title(session))
                        .size(16.0)
                        .strong()
                        .color(palette().text),
                );
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(format_relative_time(session.ended_at_epoch_millis))
                        .size(12.0)
                        .color(palette().muted),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(format_duration(session.duration_millis))
                        .size(20.0)
                        .strong()
                        .color(palette().accent),
                );
            });
        });
    });
}

fn text_field_row(ui: &mut egui::Ui, label: &str, value: &mut String, password: bool) -> bool {
    let mut changed = false;
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(label)
            .size(12.0)
            .strong()
            .color(palette().muted),
    );
    let mut edit = egui::TextEdit::singleline(value).desired_width(f32::INFINITY);
    if password {
        edit = edit.password(true);
    }
    if ui.add(edit).changed() {
        changed = true;
    }
    changed
}

fn money_field(ui: &mut egui::Ui, label: &str, value: &mut i64) -> bool {
    ui.label(
        egui::RichText::new(label)
            .size(13.0)
            .strong()
            .color(palette().text),
    );
    ui.add_sized(
        [220.0, 30.0],
        egui::DragValue::new(value)
            .speed(100.0)
            .clamp_range(0..=999_999_999)
            .suffix(" 元"),
    )
    .changed()
}

fn pill(ui: &mut egui::Ui, label: &str, fill: egui::Color32, text: egui::Color32) {
    egui::Frame::none()
        .fill(fill)
        .rounding(6.0)
        .inner_margin(egui::Margin::symmetric(8.0, 4.0))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(label)
                    .size(12.0)
                    .strong()
                    .color(text),
            );
        });
}

fn empty_state(ui: &mut egui::Ui, label: &str) {
    card_frame().show(ui, |ui| {
        ui.set_min_height(88.0);
        ui.centered_and_justified(|ui| {
            ui.label(
                egui::RichText::new(label)
                    .size(15.0)
                    .strong()
                    .color(palette().muted),
            );
        });
    });
}

fn card_frame() -> egui::Frame {
    let p = palette();
    egui::Frame::none()
        .fill(p.panel)
        .stroke(egui::Stroke::new(1.0, p.line))
        .rounding(8.0)
        .inner_margin(egui::Margin::symmetric(14.0, 13.0))
}

#[derive(Clone, Copy)]
enum ButtonTone {
    Primary,
    Quiet,
    Danger,
}

fn dark_ink() -> egui::Color32 {
    egui::Color32::from_rgb(24, 26, 23)
}

#[derive(Clone, Copy)]
struct Palette {
    bg: egui::Color32,
    nav: egui::Color32,
    panel: egui::Color32,
    panel_alt: egui::Color32,
    selected: egui::Color32,
    text: egui::Color32,
    muted: egui::Color32,
    line: egui::Color32,
    accent: egui::Color32,
    accent_soft: egui::Color32,
    good: egui::Color32,
    warn: egui::Color32,
    blue: egui::Color32,
    danger: egui::Color32,
    danger_soft: egui::Color32,
}

fn palette() -> Palette {
    Palette {
        bg: egui::Color32::from_rgb(20, 21, 19),
        nav: egui::Color32::from_rgb(27, 28, 25),
        panel: egui::Color32::from_rgb(34, 35, 31),
        panel_alt: egui::Color32::from_rgb(45, 46, 41),
        selected: egui::Color32::from_rgb(39, 48, 42),
        text: egui::Color32::from_rgb(236, 234, 225),
        muted: egui::Color32::from_rgb(156, 154, 143),
        line: egui::Color32::from_rgb(60, 61, 55),
        accent: egui::Color32::from_rgb(146, 211, 141),
        accent_soft: egui::Color32::from_rgb(46, 69, 48),
        good: egui::Color32::from_rgb(120, 205, 173),
        warn: egui::Color32::from_rgb(233, 176, 98),
        blue: egui::Color32::from_rgb(125, 176, 224),
        danger: egui::Color32::from_rgb(239, 128, 118),
        danger_soft: egui::Color32::from_rgb(70, 42, 39),
    }
}

fn board_columns(width: f32) -> usize {
    if width < 660.0 {
        1
    } else if width < 1020.0 {
        2
    } else {
        3
    }
}

fn active_summary(data: &DesktopAppData) -> String {
    let running = data
        .slots
        .iter()
        .filter(|slot| slot.running_since_epoch_millis.is_some())
        .collect::<Vec<_>>();
    match running.as_slice() {
        [] => "无运行".to_string(),
        [slot] => format!("运行中：{}", slot_title(slot)),
        slots => format!("{} 个格子运行中", slots.len()),
    }
}

fn slot_title(slot: &DesktopSlot) -> String {
    if slot.title.trim().is_empty() {
        format!("格子 {:02}", slot.id)
    } else {
        slot.title.clone()
    }
}

fn session_title(session: &DesktopSession) -> String {
    if session.slot_title.trim().is_empty() {
        format!("格子 {:02}", session.slot_id)
    } else {
        session.slot_title.clone()
    }
}

fn slot_elapsed_millis(slot: &DesktopSlot) -> i64 {
    slot.accumulated_millis
        + slot
            .running_since_epoch_millis
            .map(|started| now_millis().saturating_sub(started))
            .unwrap_or(0)
}

fn slot_progress(slot: &DesktopSlot) -> f32 {
    let elapsed = slot_elapsed_millis(slot).max(0) as f32;
    if elapsed <= 0.0 {
        return 0.03;
    }
    ((elapsed % 3_600_000.0) / 3_600_000.0).clamp(0.05, 1.0)
}

fn note_body_text(note: &DesktopNote) -> String {
    if !note.content.trim().is_empty() {
        return note.content.clone();
    }
    if !note.document.rich_text_plain_text.trim().is_empty() {
        return note.document.rich_text_plain_text.clone();
    }
    note.document
        .blocks
        .iter()
        .flat_map(|block| {
            [
                block.text.as_str(),
                block.caption.as_str(),
                block.contact_name.as_str(),
                block.call_contact_name.as_str(),
                block.call_phone_number.as_str(),
            ]
        })
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn preview_text(text: &str, fallback: &str) -> String {
    let compact = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let source = if compact.is_empty() {
        fallback.to_string()
    } else {
        compact
    };
    let mut preview = source.chars().take(52).collect::<String>();
    if source.chars().count() > 52 {
        preview.push_str("...");
    }
    preview
}

fn money_label(value: i64) -> String {
    format!("{} 元", value)
}

fn desktop_sync_message(result: &sync_core::SyncClientResult) -> String {
    if !result.ok {
        return result.message.clone();
    }
    match result.mode.as_str() {
        "registered" => "账号已创建，正在同步。".to_string(),
        "logged_in" => "登录成功，正在同步。".to_string(),
        "downloaded" => "已拉取账户里的最新数据。".to_string(),
        "uploaded" => "已把本机最新数据同步到账户。".to_string(),
        "uploaded_local" => "本机数据已强制上传到账户。".to_string(),
        _ => {
            if result.message.trim().is_empty() {
                "同步完成。".to_string()
            } else {
                result.message.clone()
            }
        }
    }
}

fn format_relative_time(epoch_millis: i64) -> String {
    if epoch_millis <= 0 {
        return "无时间".to_string();
    }
    let delta = now_millis().saturating_sub(epoch_millis).max(0);
    let minutes = delta / 60_000;
    if minutes < 1 {
        "刚刚".to_string()
    } else if minutes < 60 {
        format!("{minutes} 分钟前")
    } else if minutes < 24 * 60 {
        format!("{} 小时前", minutes / 60)
    } else {
        format!("{} 天前", minutes / (24 * 60))
    }
}

fn decode_data(raw: &str) -> DesktopAppData {
    serde_json::from_str::<DesktopAppData>(raw).unwrap_or_default()
}

fn slot_duration_label(slot: &DesktopSlot) -> String {
    let elapsed = slot.accumulated_millis
        + slot
            .running_since_epoch_millis
            .map(|started| now_millis().saturating_sub(started))
            .unwrap_or(0);
    format_duration(elapsed)
}

fn format_duration(duration_millis: i64) -> String {
    let seconds = duration_millis.max(0) / 1000;
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn app_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("GridTimerClient")
}

fn default_device_name() -> String {
    std::env::var("COMPUTERNAME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "Windows".to_string())
}

fn default_ai_base_url() -> String {
    ai_client::DEFAULT_AI_BASE_URL.to_string()
}

fn default_ai_model() -> String {
    ai_client::DEFAULT_AI_MODEL.to_string()
}

fn install_ui_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let font_name = "system_chinese".to_string();
    let candidates = [
        r"C:\Windows\Fonts\Deng.ttf",
        r"C:\Windows\Fonts\AlibabaPuHuiTi-2-55-Regular.ttf",
        r"C:\Windows\Fonts\AlibabaPuHuiTi.ttf",
        r"C:\Windows\Fonts\simhei.ttf",
    ];
    let Some(bytes) = candidates.iter().find_map(|path| fs::read(path).ok()) else {
        ctx.set_fonts(fonts);
        return;
    };
    fonts
        .font_data
        .insert(font_name.clone(), egui::FontData::from_owned(bytes));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, font_name.clone());
    }
    ctx.set_fonts(fonts);
}

fn install_ui_style(ctx: &egui::Context) {
    let p = palette();
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.text_edit_width = 260.0;
    style.visuals = egui::Visuals::dark();
    style.visuals.override_text_color = Some(p.text);
    style.visuals.panel_fill = p.bg;
    style.visuals.window_fill = p.panel;
    style.visuals.extreme_bg_color = p.bg;
    style.visuals.faint_bg_color = p.panel_alt;
    style.visuals.selection.bg_fill = p.accent_soft;
    style.visuals.selection.stroke = egui::Stroke::new(1.0, p.accent);
    style.visuals.hyperlink_color = p.blue;
    style.visuals.widgets.noninteractive.bg_fill = p.panel;
    style.visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, p.text);
    style.visuals.widgets.inactive.bg_fill = p.panel_alt;
    style.visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, p.text);
    style.visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(55, 57, 50);
    style.visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, p.text);
    style.visuals.widgets.active.bg_fill = p.accent_soft;
    style.visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, p.accent);
    ctx.set_style(style);
}
