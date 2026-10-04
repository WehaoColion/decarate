// Windows - Reuse indexed, versioned quick navigation results across unchanged frames.
// v1.0.3.16 Windows - Render only visible knowledge home rows and reuse cached page counts.
// v1.0.3 Windows - Build recent-page cards without copying document history.
// v2.22.54 - Breadcrumbs read the lightweight navigation index.
// v2.22.53 - A quiet workspace with scoped navigation and contextual page tools.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KnowledgeInspector {
    Outline,
    Properties,
    Discussion,
    History,
    Tools,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum KnowledgeQuickChoice {
    Page(String),
    NewPage,
    NewDatabase,
    Home,
    Focus,
    Navigator,
}

struct KnowledgeExperienceState {
    workspace: String,
    trail: Vec<String>,
    cursor: usize,
    window_width: f32,
    navigator_hidden: bool,
    navigator_drawer_open: bool,
    navigator_width: f32,
    focus_mode: bool,
    inspector: Option<KnowledgeInspector>,
    palette_open: bool,
    palette_focus: bool,
    palette_query: String,
    palette_selected: usize,
    record_query: String,
    record_page: String,
}

impl Default for KnowledgeExperienceState {
    fn default() -> Self {
        Self {
            workspace: String::new(),
            trail: Vec::new(),
            cursor: 0,
            window_width: 1240.0,
            navigator_hidden: false,
            navigator_drawer_open: false,
            navigator_width: 238.0,
            focus_mode: false,
            inspector: None,
            palette_open: false,
            palette_focus: false,
            palette_query: String::new(),
            palette_selected: 0,
            record_query: String::new(),
            record_page: String::new(),
        }
    }
}

impl KnowledgeExperienceState {
    fn remember(&mut self, page: String) {
        if self.trail.get(self.cursor) == Some(&page) {
            return;
        }
        if !self.trail.is_empty() {
            self.trail.truncate(self.cursor + 1);
        }
        self.trail.push(page);
        if self.trail.len() > 64 {
            self.trail.remove(0);
        }
        self.cursor = self.trail.len().saturating_sub(1);
    }
}

fn knowledge_quick_rank(title: &str, detail: &str, query: &str) -> Option<u8> {
    let query = query.trim().to_lowercase();
    let title = title.to_lowercase();
    let detail = detail.to_lowercase();
    if query.is_empty() || title.starts_with(&query) {
        Some(0)
    } else if title.contains(&query) {
        Some(1)
    } else if query
        .split_whitespace()
        .all(|part| title.contains(part) || detail.contains(part))
    {
        Some(2)
    } else {
        None
    }
}

fn knowledge_quiet_button(ui: &mut egui::Ui, text: &str, tip: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(text).size(13.0))
            .frame(false)
            .min_size(egui::vec2(
                (text.chars().count() as f32 * 13.0 + 16.0).max(28.0),
                28.0,
            )),
    )
    .on_hover_text(tip)
}

impl TimerWindowsClient {
    fn observe_knowledge_navigation(&mut self) {
        let workspace = self.background_job_workspace_fingerprint();
        if self.desktop_ui.experience.workspace != workspace {
            self.desktop_ui.experience = KnowledgeExperienceState {
                workspace,
                ..Default::default()
            };
        }
        if self.workspace_edit_locked() || self.knowledge_trash_mode {
            return;
        }
        let page = if self.desktop_ui.navigation.editor_visible {
            self.selected_note_id.clone()
        } else {
            String::new()
        };
        self.desktop_ui.experience.remember(page);
        if self.desktop_ui.experience.record_page != self.selected_note_id {
            self.desktop_ui.experience.record_page = self.selected_note_id.clone();
            self.desktop_ui.experience.record_query.clear();
        }
    }

    fn knowledge_trail_target(&self, direction: isize) -> Option<(usize, String)> {
        let state = &self.desktop_ui.experience;
        let mut cursor = state.cursor as isize + direction;
        while cursor >= 0 && (cursor as usize) < state.trail.len() {
            let page = &state.trail[cursor as usize];
            if page.is_empty()
                || self.data.notes.iter().any(|note| {
                    note.id == *page
                        && note.deleted_at_epoch_millis.is_none()
                        && desktop_note_kind(note) == DesktopNoteKind::Document
                })
            {
                return Some((cursor as usize, page.clone()));
            }
            cursor += direction;
        }
        None
    }

    fn navigate_knowledge_trail(&mut self, direction: isize) -> bool {
        if self.workspace_edit_locked() || self.flush_note_draft().is_err() {
            return false;
        }
        let Some((cursor, page)) = self.knowledge_trail_target(direction) else {
            return false;
        };
        if page.is_empty() {
            if !self.return_to_document_list() {
                return false;
            }
        } else {
            self.select_note_by_id(&page);
            if self.selected_note_id != page || !self.desktop_ui.navigation.editor_visible {
                return false;
            }
        }
        self.desktop_ui.experience.cursor = cursor;
        true
    }

    fn open_knowledge_quick_switch(&mut self) {
        self.observe_knowledge_navigation();
        self.desktop_ui.experience.palette_open = true;
        self.desktop_ui.experience.palette_focus = true;
        self.desktop_ui.experience.palette_query.clear();
        self.desktop_ui.experience.palette_selected = 0;
    }

    fn handle_knowledge_experience_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        if self.tab != AppTab::Knowledge {
            return false;
        }
        self.observe_knowledge_navigation();
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::K)) {
            if self.desktop_ui.experience.palette_open {
                self.desktop_ui.experience.palette_open = false;
            } else {
                self.open_knowledge_quick_switch();
            }
            return true;
        }
        if self.desktop_ui.experience.palette_open {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.desktop_ui.experience.palette_open = false;
            }
            return true;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F8)) {
            self.desktop_ui.experience.focus_mode = !self.desktop_ui.experience.focus_mode;
            return true;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Backslash)) {
            self.toggle_knowledge_navigator(ctx.screen_rect().width());
            return true;
        }
        let alt = egui::Modifiers {
            alt: true,
            ..Default::default()
        };
        for (key, direction) in [(egui::Key::ArrowLeft, -1), (egui::Key::ArrowRight, 1)] {
            if ctx.input_mut(|i| i.consume_key(alt, key)) {
                self.navigate_knowledge_trail(direction);
                return true;
            }
        }
        if (self.desktop_ui.experience.inspector.is_some()
            || self.desktop_ui.experience.focus_mode
            || self.desktop_ui.experience.navigator_drawer_open)
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            if self.desktop_ui.experience.navigator_drawer_open {
                self.desktop_ui.experience.navigator_drawer_open = false;
            } else if self.desktop_ui.experience.inspector.is_some() {
                self.desktop_ui.experience.inspector = None;
            } else {
                self.desktop_ui.experience.focus_mode = false;
            }
            return true;
        }
        false
    }

    fn knowledge_quick_choices(&self) -> Arc<Vec<(KnowledgeQuickChoice, String, String)>> {
        let query = self
            .desktop_ui
            .experience
            .palette_query
            .trim()
            .to_lowercase();
        let trail = &self.desktop_ui.experience.trail;
        {
            let mut cache = self.desktop_ui.knowledge.read_cache.borrow_mut();
            cache.at_version(self.notes_cache_version());
            if let Some(previous) = &cache.quick_choices {
                if previous.query == query && &previous.trail == trail {
                    return Arc::clone(&previous.choices);
                }
            }
        }
        let navigation = self.knowledge_navigation();
        let choices = Arc::new(build_knowledge_quick_choices(&navigation, &query, trail));
        self.desktop_ui
            .knowledge
            .read_cache
            .borrow_mut()
            .quick_choices = Some(KnowledgeQuickChoiceCache {
            query,
            trail: trail.clone(),
            choices: Arc::clone(&choices),
        });
        choices
    }

    fn execute_knowledge_quick_choice(&mut self, choice: KnowledgeQuickChoice) -> bool {
        if self.workspace_edit_locked() {
            return false;
        }
        let completed = match choice {
            KnowledgeQuickChoice::Page(id) => {
                if self.flush_note_draft().is_err() {
                    return false;
                }
                self.select_note_by_id(&id);
                self.selected_note_id == id && self.desktop_ui.navigation.editor_visible
            }
            KnowledgeQuickChoice::NewPage => self.create_knowledge_page(None, false),
            KnowledgeQuickChoice::NewDatabase => self.create_knowledge_page(None, true),
            KnowledgeQuickChoice::Home => self.return_to_document_list(),
            KnowledgeQuickChoice::Focus => {
                self.desktop_ui.experience.focus_mode = !self.desktop_ui.experience.focus_mode;
                true
            }
            KnowledgeQuickChoice::Navigator => {
                self.toggle_knowledge_navigator(self.desktop_ui.experience.window_width);
                true
            }
        };
        if completed {
            self.desktop_ui.experience.palette_open = false;
            self.observe_knowledge_navigation();
        }
        completed
    }

    fn ui_knowledge_quick_switch(&mut self, ctx: &egui::Context) {
        if !self.desktop_ui.experience.palette_open || self.tab != AppTab::Knowledge {
            return;
        }
        let mut open = true;
        let mut picked = None;
        let width = (ctx.screen_rect().width() - 80.0).clamp(260.0, 560.0);
        egui::Window::new("快速跳转")
            .id(egui::Id::new("knowledge_quick_switch"))
            .open(&mut open)
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(palette().panel)
                    .rounding(12.0)
                    .inner_margin(14.0),
            )
            .anchor(egui::Align2::CENTER_TOP, [0.0, 82.0])
            .fixed_size([width, 420.0])
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("快速跳转").size(16.0).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if knowledge_quiet_button(ui, "×", "关闭 · Esc").clicked() {
                            self.desktop_ui.experience.palette_open = false;
                        }
                    });
                });
                let down =
                    ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
                let up = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp));
                let enter =
                    ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
                let input = ui.add(
                    egui::TextEdit::singleline(&mut self.desktop_ui.experience.palette_query)
                        .id(egui::Id::new("knowledge_quick_query"))
                        .desired_width(f32::INFINITY)
                        .font(egui::FontId::proportional(19.0))
                        .hint_text("搜索页面，或输入 > 查找操作"),
                );
                if self.desktop_ui.experience.palette_focus {
                    input.request_focus();
                    self.desktop_ui.experience.palette_focus = false;
                }
                if input.changed() {
                    self.desktop_ui.experience.palette_selected = 0;
                }
                let choices = self.knowledge_quick_choices();
                let selected = &mut self.desktop_ui.experience.palette_selected;
                *selected = (*selected).min(choices.len().saturating_sub(1));
                if down && !choices.is_empty() {
                    *selected = (*selected + 1) % choices.len();
                }
                if up && !choices.is_empty() {
                    *selected = (*selected + choices.len() - 1) % choices.len();
                }
                if enter {
                    picked = choices.get(*selected).map(|c| c.0.clone());
                }
                let selected = *selected;
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_source("knowledge_quick_results")
                    .max_height(310.0)
                    .show(ui, |ui| {
                        if choices.is_empty() {
                            ui.label("没有匹配的页面");
                        }
                        for (index, (choice, title, detail)) in choices.iter().enumerate() {
                            let (rect, row) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 52.0),
                                egui::Sense::click(),
                            );
                            if index == selected || row.hovered() {
                                ui.painter().rect_filled(
                                    rect,
                                    6.0,
                                    if index == selected {
                                        palette().selected
                                    } else {
                                        palette().panel_alt
                                    },
                                );
                            }
                            let painter = ui.painter().with_clip_rect(rect.shrink(8.0));
                            painter.text(
                                rect.min + egui::vec2(14.0, 8.0),
                                egui::Align2::LEFT_TOP,
                                title,
                                egui::FontId::proportional(15.0),
                                palette().text,
                            );
                            painter.text(
                                rect.min + egui::vec2(14.0, 31.0),
                                egui::Align2::LEFT_TOP,
                                detail,
                                egui::FontId::proportional(11.0),
                                palette().muted,
                            );
                            row.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::WidgetType::SelectableLabel,
                                    index == selected,
                                    title,
                                )
                            });
                            if row.clicked() {
                                picked = Some(choice.clone());
                            }
                            if index == selected && (up || down) {
                                row.scroll_to_me(Some(egui::Align::Center));
                            }
                        }
                    });
                ui.separator();
                ui.label(
                    egui::RichText::new("↑ ↓ 选择     Enter 打开     Esc 关闭")
                        .size(11.0)
                        .color(palette().muted),
                );
                if self.note_save_state == DesktopDocumentSaveState::Failed {
                    ui.colored_label(palette().danger, "当前页面未保存成功，修复保存后再跳转");
                }
            });
        if !open {
            self.desktop_ui.experience.palette_open = false;
        }
        if let Some(choice) = picked {
            self.execute_knowledge_quick_choice(choice);
        }
    }

    fn ui_knowledge_breadcrumbs(&mut self, ui: &mut egui::Ui) {
        if knowledge_quiet_button(ui, "知识库", "返回资料列表").clicked() {
            self.return_to_document_list();
        }
        if !self.desktop_ui.navigation.editor_visible || self.selected_note_id.is_empty() {
            return;
        }
        let navigation = self.knowledge_navigation();
        let ancestors = navigation.ancestors(&self.selected_note_id);
        for page in ancestors
            .iter()
            .skip(ancestors.len().saturating_sub(2))
            .filter(|page| !page.encrypted)
        {
            ui.label(egui::RichText::new("/").color(palette().muted));
            let title = page.title.chars().take(16).collect::<String>();
            if knowledge_quiet_button(ui, &title, &page.title).clicked() {
                self.select_note_by_id(&page.id);
                break;
            }
        }
    }

    fn ui_knowledge_context_bar(&mut self, ui: &mut egui::Ui) {
        self.observe_knowledge_navigation();
        self.desktop_ui.experience.window_width = ui.ctx().screen_rect().width();
        let p = palette();
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        ui.horizontal(|ui| {
            for (symbol, direction, tip) in [("‹", -1, "后退 · Alt+←"), ("›", 1, "前进 · Alt+→")]
            {
                let response = ui
                    .add_enabled(
                        self.knowledge_trail_target(direction).is_some(),
                        egui::Button::new(egui::RichText::new(symbol).size(23.0))
                            .frame(false)
                            .min_size(egui::vec2(26.0, 28.0)),
                    )
                    .on_hover_text(tip);
                if response.clicked() {
                    self.navigate_knowledge_trail(direction);
                }
            }
            if knowledge_quiet_button(ui, "☰", "显示或收起页面树 · Ctrl+\\").clicked() {
                self.toggle_knowledge_navigator(ui.ctx().screen_rect().width());
            }
            let width = ui.available_width();
            ui.allocate_ui_with_layout(
                egui::vec2((width - 420.0).max(70.0), 28.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    if width > 650.0 {
                        self.ui_knowledge_breadcrumbs(ui);
                    } else if knowledge_quiet_button(ui, "知识库", "返回资料列表").clicked()
                    {
                        self.return_to_document_list();
                    }
                },
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button("•••", |ui| {
                    self.ui_knowledge_workspace_tools(ui);
                    ui.separator();
                    if ui.button("页面历史").clicked() {
                        self.desktop_ui.experience.inspector = Some(KnowledgeInspector::History);
                        ui.close_menu();
                    }
                    if ui.button("附件与加密").clicked() {
                        self.desktop_ui.experience.inspector = Some(KnowledgeInspector::Tools);
                        ui.close_menu();
                    }
                    if ui.button("引用与批注").clicked() {
                        self.desktop_ui.experience.inspector = Some(KnowledgeInspector::Discussion);
                        ui.close_menu();
                    }
                    if ui.button("知识问答").clicked() {
                        self.knowledge_ai_panel_open = !self.knowledge_ai_panel_open;
                        ui.close_menu();
                    }
                    if ui.button("管理文件夹").clicked() {
                        self.knowledge_folder_manager_open = !self.knowledge_folder_manager_open;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(
                            !self.selected_note_id.is_empty(),
                            egui::Button::new("移入回收站"),
                        )
                        .clicked()
                    {
                        self.desktop_ui.pending_knowledge_delete = Some((
                            KnowledgeDeleteIntent::Trash(self.selected_note_id.clone()),
                            self.data_version,
                        ));
                        ui.close_menu();
                    }
                    if ui
                        .button(if self.knowledge_trash_mode {
                            "返回文档"
                        } else {
                            "回收站"
                        })
                        .clicked()
                    {
                        self.toggle_knowledge_trash_mode();
                        ui.close_menu();
                    }
                });
                self.ui_knowledge_save_control(ui);
                if knowledge_quiet_button(
                    ui,
                    if self.desktop_ui.experience.focus_mode {
                        "退出专注"
                    } else {
                        "专注"
                    },
                    "专注模式 · F8，Esc 返回",
                )
                .clicked()
                {
                    self.desktop_ui.experience.focus_mode = !self.desktop_ui.experience.focus_mode;
                }
                if knowledge_quiet_button(ui, "查找", "快速跳转 · Ctrl+K").clicked() {
                    self.open_knowledge_quick_switch();
                }
                if self.desktop_ui.navigation.editor_visible {
                    let properties = knowledge_quiet_button(ui, "页面", "页面属性与工具");
                    #[cfg(test)]
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(
                            egui::Id::new("knowledge_properties_toggle"),
                            properties.rect,
                        )
                    });
                    if properties.clicked() {
                        self.desktop_ui.experience.inspector =
                            if self.desktop_ui.experience.inspector
                                == Some(KnowledgeInspector::Properties)
                            {
                                None
                            } else {
                                Some(KnowledgeInspector::Properties)
                            };
                    }
                    if knowledge_quiet_button(ui, "目录", "查看大纲").clicked() {
                        self.desktop_ui.experience.inspector =
                            if self.desktop_ui.experience.inspector
                                == Some(KnowledgeInspector::Outline)
                            {
                                None
                            } else {
                                Some(KnowledgeInspector::Outline)
                            };
                    }
                    if !self.selected_note_is_locked()
                        && knowledge_quiet_button(ui, "阅读", "打开阅读视图").clicked()
                    {
                        self.open_knowledge_reading_view(None);
                    }
                }
                let running = self
                    .data
                    .slots
                    .iter()
                    .filter(|s| s.running_since_epoch_millis.is_some())
                    .count();
                if running > 0
                    && width > 700.0
                    && ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(format!("● {running}"))
                                    .size(12.0)
                                    .color(p.good),
                            )
                            .frame(false),
                        )
                        .on_hover_text("正在运行的本机计时 · 点击查看")
                        .clicked()
                {
                    self.switch_tab(AppTab::Board);
                }
            });
        });
    }
}

impl TimerWindowsClient {
    fn toggle_knowledge_navigator(&mut self, width: f32) {
        if width < 970.0 {
            self.desktop_ui.experience.navigator_drawer_open =
                !self.desktop_ui.experience.navigator_drawer_open;
        } else {
            self.desktop_ui.experience.navigator_hidden =
                !self.desktop_ui.experience.navigator_hidden;
        }
        self.desktop_ui.experience.focus_mode = false;
    }

    fn ui_knowledge_create_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button("空白页面").clicked() {
            self.create_knowledge_page(None, false);
            ui.close_menu();
        }
        if ui.button("数据库").clicked() {
            self.create_knowledge_page(None, true);
            ui.close_menu();
        }
        ui.separator();
        for (label, template) in [
            ("项目计划", KnowledgeTemplate::Project),
            ("会议纪要", KnowledgeTemplate::Meeting),
            ("复盘", KnowledgeTemplate::Review),
            ("阅读笔记", KnowledgeTemplate::Reading),
            ("任务管理", KnowledgeTemplate::Tasks),
        ] {
            if ui.button(label).clicked() {
                self.create_knowledge_template(template);
                ui.close_menu();
            }
        }
    }

    fn ui_knowledge_save_control(&mut self, ui: &mut egui::Ui) {
        let enabled = !self.selected_note_id.is_empty()
            && !self.selected_note_is_locked()
            && self.workspace_persistence_ready
            && (self.note_dirty || self.persistence.pending());
        let save = ui
            .add_enabled(
                enabled,
                egui::Button::new(
                    egui::RichText::new(if enabled {
                        "保存"
                    } else {
                        self.note_save_state.label()
                    })
                    .size(12.0)
                    .color(
                        if self.note_save_state == DesktopDocumentSaveState::Failed {
                            palette().danger
                        } else {
                            palette().muted
                        },
                    ),
                )
                .frame(false),
            )
            .on_hover_text("保存当前页面 · Ctrl+S");
        #[cfg(test)]
        ui.ctx().data_mut(|d| {
            d.insert_temp(egui::Id::new("knowledge_save"), (save.rect, save.enabled()))
        });
        if save.clicked() {
            self.save_note();
        }
    }

    fn ui_knowledge_library(&mut self, ui: &mut egui::Ui) {
        let p = palette();
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new("知识库")
                        .size(32.0)
                        .strong()
                        .color(p.text),
                );
                let count = self.view_cache.active_note_count;
                ui.label(
                    egui::RichText::new(format!("{count} 篇资料"))
                        .size(12.0)
                        .color(p.muted),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button("＋ 新建", |ui| self.ui_knowledge_create_menu(ui));
            });
        });
        ui.add_space(22.0);
        let query_active = !self.note_search_draft.trim().is_empty()
            || self.knowledge_quick_filter != KnowledgeQuickFilter::All
            || self.data.note_preferences.selected_folder_id.is_some();
        if !query_active {
            let navigation = self.knowledge_navigation();
            let mut seen = HashSet::new();
            let recent = self
                .desktop_ui
                .experience
                .trail
                .iter()
                .rev()
                .filter(|id| !id.is_empty())
                .take(12)
                .filter_map(|id| {
                    navigation
                        .by_id
                        .get(id)
                        .map(|index| &navigation.pages[*index])
                })
                .filter(|page| !page.encrypted)
                .filter(|p| seen.insert(p.id.clone()))
                .take(3)
                .map(|page| (page.id.clone(), page.title.clone(), page.database))
                .collect::<Vec<_>>();
            if !recent.is_empty() {
                ui.label(egui::RichText::new("继续阅读").size(12.0).color(p.muted));
                ui.add_space(8.0);
                ui.columns(
                    if ui.available_width() < 500.0 { 1 } else { 3 },
                    |columns| {
                        for (column, (page_id, page_title, database)) in
                            columns.iter_mut().zip(recent.iter())
                        {
                            let title = if page_title.trim().is_empty() {
                                "未命名"
                            } else {
                                page_title
                            };
                            let (rect, response) = column.allocate_exact_size(
                                egui::vec2(column.available_width(), 84.0),
                                egui::Sense::click(),
                            );
                            column.painter().rect_filled(
                                rect,
                                8.0,
                                if response.hovered() {
                                    p.selected
                                } else {
                                    p.panel_alt
                                },
                            );
                            let painter = column.painter().with_clip_rect(rect.shrink(14.0));
                            painter.text(
                                rect.min + egui::vec2(16.0, 17.0),
                                egui::Align2::LEFT_TOP,
                                if *database { "数据库" } else { "页面" },
                                egui::FontId::proportional(11.0),
                                p.muted,
                            );
                            painter.text(
                                rect.min + egui::vec2(16.0, 44.0),
                                egui::Align2::LEFT_TOP,
                                title,
                                egui::FontId::proportional(16.0),
                                p.text,
                            );
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(egui::WidgetType::Button, title)
                            });
                            if response.clicked() {
                                self.select_note_by_id(page_id);
                            }
                        }
                    },
                );
                ui.add_space(26.0);
            }
        }
        self.ui_knowledge_collection_controls(ui);
        ui.add_space(16.0);
        if self.knowledge_view != KnowledgeCollectionView::Recent {
            self.ui_knowledge_collection_list(
                ui,
                (ui.clip_rect().bottom() - ui.cursor().top() - 20.0).max(180.0),
            );
            return;
        }
        let pages = self.view_cache.notes.clone();
        if pages.is_empty() {
            ui.add_space(36.0);
            ui.label(
                egui::RichText::new(if query_active {
                    "没有匹配的资料"
                } else {
                    "从一页开始"
                })
                .size(22.0)
                .strong(),
            );
            ui.add_space(8.0);
            if query_active {
                if knowledge_quiet_button(ui, "清除筛选", "显示全部资料").clicked() {
                    self.note_search_draft.clear();
                    self.knowledge_quick_filter = KnowledgeQuickFilter::All;
                    self.set_knowledge_folder_filter(None);
                    self.view_cache.data_version = u64::MAX;
                    self.rebuild_view_cache();
                }
            } else {
                ui.label(egui::RichText::new("写下想法，或用数据库整理一个项目。").color(p.muted));
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    if action_button(ui, "创建页面", ButtonTone::Primary).clicked() {
                        self.create_knowledge_page(None, false);
                    }
                    if knowledge_quiet_button(ui, "创建数据库", "以表格和看板整理记录").clicked()
                    {
                        self.create_knowledge_page(None, true);
                    }
                });
            }
            return;
        }
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("资料").small().color(p.muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new("最近修改").small().color(p.muted));
            });
        });
        ui.add_space(4.0);
        // Keep one scroll surface for the library. Allocate the list's full
        // height, then create responses and lay out text only for visible rows.
        // Large libraries no longer visit every page on each repaint.
        let row_height = 54.0;
        let row_stride = row_height + ui.spacing().item_spacing.y;
        let total_height = row_stride * pages.len() as f32 - ui.spacing().item_spacing.y;
        let (list_rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), total_height.max(0.0)),
            egui::Sense::hover(),
        );
        let clip = ui.clip_rect();
        let first = ((clip.top() - list_rect.top()) / row_stride)
            .floor()
            .max(0.0) as usize;
        let end = ((clip.bottom() - list_rect.top()) / row_stride)
            .ceil()
            .max(0.0) as usize;
        for (index, page) in pages
            .iter()
            .enumerate()
            .skip(first)
            .take(end.min(pages.len()).saturating_sub(first))
        {
            let encrypted = page.encrypted;
            let title = if encrypted {
                "加密页面"
            } else if page.title.trim().is_empty() {
                "未命名"
            } else {
                &page.title
            };
            let icon = if encrypted {
                "🔒"
            } else if page.is_database {
                "▦"
            } else {
                "▤"
            };
            let rect = egui::Rect::from_min_size(
                list_rect.min + egui::vec2(0.0, index as f32 * row_stride),
                egui::vec2(list_rect.width(), row_height),
            );
            #[cfg(test)]
            ui.ctx().data_mut(|data| {
                data.insert_temp(egui::Id::new(("knowledge_home_row", &page.id)), rect);
            });
            let response = ui.interact(
                rect,
                ui.id().with(("knowledge_recent_page", &page.id)),
                egui::Sense::click(),
            );
            if response.hovered() {
                ui.painter().rect_filled(rect, 6.0, p.panel_alt);
            }
            ui.painter().text(
                rect.left_center() + egui::vec2(12.0, 0.0),
                egui::Align2::LEFT_CENTER,
                icon,
                egui::FontId::proportional(17.0),
                p.muted,
            );
            let date = desktop_local_timestamp(page.updated_at_epoch_millis);
            let date = date.chars().take(16).collect::<String>();
            let date_width = if rect.width() > 520.0 { 132.0 } else { 0.0 };
            let galley = ui.painter().layout(
                title.to_string(),
                egui::FontId::proportional(15.0),
                p.text,
                (rect.width() - 62.0 - date_width).max(80.0),
            );
            ui.painter()
                .with_clip_rect(egui::Rect::from_min_max(
                    rect.min + egui::vec2(42.0, 8.0),
                    rect.max - egui::vec2(date_width + 10.0, 8.0),
                ))
                .galley(rect.min + egui::vec2(42.0, 17.0), galley, p.text);
            if date_width > 0.0 {
                ui.painter().text(
                    rect.right_center() - egui::vec2(12.0, 0.0),
                    egui::Align2::RIGHT_CENTER,
                    date,
                    egui::FontId::proportional(11.0),
                    p.muted,
                );
            }
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, title));
            if response.on_hover_text(title).clicked() {
                self.select_note_by_id(&page.id);
            }
            ui.painter().hline(
                rect.x_range(),
                rect.bottom(),
                egui::Stroke::new(0.5, p.line),
            );
        }
    }

    fn ui_knowledge_inspector(&mut self, ui: &mut egui::Ui) {
        let Some(active) = self.desktop_ui.experience.inspector else {
            return;
        };
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 10.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("页面面板").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if knowledge_quiet_button(ui, "×", "收起面板 · Esc").clicked() {
                    self.desktop_ui.experience.inspector = None;
                }
            });
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().button_padding.x = 6.0;
            for (kind, title) in [
                (KnowledgeInspector::Outline, "目录"),
                (KnowledgeInspector::Properties, "属性"),
                (KnowledgeInspector::Discussion, "引用"),
                (KnowledgeInspector::History, "历史"),
                (KnowledgeInspector::Tools, "工具"),
            ] {
                if ui.selectable_label(active == kind, title).clicked() {
                    self.desktop_ui.experience.inspector = Some(kind);
                }
            }
        });
        ui.separator();
        if self.selected_note_is_locked() {
            ui.label("解密后可查看页面信息");
            return;
        }
        egui::ScrollArea::vertical()
            .id_source(("knowledge_inspector", self.selected_note_id.clone()))
            .auto_shrink([false, false])
            .show(ui, |ui| match active {
                KnowledgeInspector::Properties => self.ui_knowledge_page_properties(ui),
                KnowledgeInspector::Discussion => self.ui_knowledge_references_and_comments(ui),
                KnowledgeInspector::History => self.ui_note_history(ui),
                KnowledgeInspector::Tools => {
                    self.ui_note_attachments(ui, self.knowledge_page_locked());
                    self.ui_note_export_tools(ui);
                    self.ui_note_crypto(ui);
                }
                KnowledgeInspector::Outline => {
                    let headings = self
                        .note_blocks_draft
                        .iter()
                        .filter_map(|b| {
                            let level = b
                                .knowledge
                                .as_ref()
                                .and_then(|m| match m.kind {
                                    knowledge::BlockKind::Heading1 => Some(0),
                                    knowledge::BlockKind::Heading2 => Some(1),
                                    knowledge::BlockKind::Heading3 => Some(2),
                                    _ => None,
                                })
                                .or_else(|| {
                                    let hashes = b.text.chars().take_while(|c| *c == '#').count();
                                    (hashes > 0 && hashes <= 3).then_some(hashes.saturating_sub(1))
                                })?;
                            Some((
                                b.id.clone(),
                                b.text.trim_start_matches('#').trim().to_string(),
                                level,
                            ))
                        })
                        .collect::<Vec<_>>();
                    if headings.is_empty() {
                        ui.label(
                            egui::RichText::new("添加标题块后，目录会出现在这里。")
                                .color(palette().muted),
                        );
                    }
                    for (id, title, level) in headings {
                        ui.horizontal(|ui| {
                            ui.add_space(level as f32 * 12.0);
                            if ui
                                .add_sized(
                                    [ui.available_width(), 32.0],
                                    egui::SelectableLabel::new(
                                        self.note_active_block_id == id,
                                        title,
                                    ),
                                )
                                .clicked()
                            {
                                self.desktop_ui.parity.outline_target = Some(id);
                            }
                        });
                    }
                }
            });
    }
}

fn knowledge_filter_rows(
    rows: &[String],
    records: &[knowledge::PageRecord],
    query: &str,
) -> Vec<String> {
    let query = query.trim().to_lowercase();
    rows.iter()
        .filter(|id| {
            query.is_empty()
                || records.iter().any(|p| {
                    p.id == **id
                        && !p.encrypted
                        && !p.deleted
                        && p.title.to_lowercase().contains(&query)
                })
        })
        .cloned()
        .collect()
}

struct KnowledgeQuickChoiceCache {
    query: String,
    trail: Vec<String>,
    choices: Arc<Vec<(KnowledgeQuickChoice, String, String)>>,
}

fn build_knowledge_quick_choices(
    navigation: &KnowledgeNavigation,
    query: &str,
    trail: &[String],
) -> Vec<(KnowledgeQuickChoice, String, String)> {
    let commands = [
        (KnowledgeQuickChoice::NewPage, "新建页面", "Ctrl+N"),
        (
            KnowledgeQuickChoice::NewDatabase,
            "新建数据库",
            "表格、看板与日历",
        ),
        (KnowledgeQuickChoice::Home, "返回知识库", "查看全部资料"),
        (KnowledgeQuickChoice::Focus, "切换专注模式", "F8"),
        (
            KnowledgeQuickChoice::Navigator,
            "显示或收起页面树",
            "Ctrl+\\",
        ),
    ];
    if let Some(query) = query.strip_prefix('>') {
        return commands
            .into_iter()
            .filter(|(_, title, detail)| knowledge_quick_rank(title, detail, query).is_some())
            .map(|(choice, title, detail)| (choice, title.into(), detail.into()))
            .collect();
    }
    let recent_positions = trail
        .iter()
        .enumerate()
        .map(|(index, id)| (id.as_str(), index + 1))
        .collect::<std::collections::HashMap<_, _>>();
    let mut pages = navigation
        .pages
        .iter()
        .filter_map(|n| {
            let encrypted = n.encrypted;
            let title = if encrypted {
                "加密页面"
            } else if n.title.trim().is_empty() {
                "未命名"
            } else {
                &n.title
            };
            let parent = n
                .parent_id
                .as_deref()
                .and_then(|id| navigation.by_id.get(id))
                .map(|index| &navigation.pages[*index])
                .filter(|page| !page.encrypted)
                .map_or("知识库", |page| page.title.as_str());
            let detail = if encrypted {
                "已加密".to_string()
            } else {
                format!(
                    "{} · {}",
                    parent,
                    if n.database { "数据库" } else { "页面" }
                )
            };
            let rank = knowledge_quick_rank(title, &detail, query)?;
            let recent = recent_positions.get(n.id.as_str()).copied().unwrap_or(0);
            Some((
                rank,
                std::cmp::Reverse(recent),
                std::cmp::Reverse(n.updated_at_epoch_millis),
                KnowledgeQuickChoice::Page(n.id.clone()),
                title.to_string(),
                detail,
            ))
        })
        .collect::<Vec<_>>();
    pages.sort_by(|a, b| (&a.0, &a.1, &a.2, &a.4).cmp(&(&b.0, &b.1, &b.2, &b.4)));
    let mut choices = pages
        .into_iter()
        .take(80)
        .map(|p| (p.3, p.4, p.5))
        .collect::<Vec<_>>();
    if query.is_empty() {
        choices.extend(
            commands
                .into_iter()
                .take(2)
                .map(|(c, t, d)| (c, t.into(), d.into())),
        );
    }
    choices
}
