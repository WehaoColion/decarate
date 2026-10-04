// v1.1.0.7 Windows - Separate direct AI questions from source-only knowledge and show send scope.
// v1.0.3.12 Windows - Index library folder names once per view instead of scanning per note.
// v1.0.3.3 Windows - Add a dedicated canvas view to the knowledge workspace.
// v2.22.53 - Focused knowledge workspace, contextual controls and safe navigation.
// v2.22.52 - Preserve structured knowledge pages through editing and persistence.
// v2.22.46 - Align workspace controls and responsive pane spacing.
// v2.22.41 - Focused knowledge browsing, search and folder management.

fn knowledge_library_folder_counts<'a>(
    notes: &[DesktopNoteListItem],
    folders: &'a [DesktopNoteFolder],
) -> BTreeMap<&'a str, usize> {
    let mut folder_names = std::collections::HashMap::with_capacity(folders.len());
    for folder in folders {
        // Preserve the first match used by the previous linear lookup if an
        // imported workspace contains more than one folder with the same ID.
        folder_names
            .entry(folder.id.as_str())
            .or_insert(folder.name.as_str());
    }
    let mut counts = BTreeMap::new();
    for note in notes {
        let name = note
            .folder_id
            .as_deref()
            .and_then(|id| folder_names.get(id).copied())
            .unwrap_or("未入库");
        *counts.entry(name).or_insert(0) += 1;
    }
    counts
}

impl TimerWindowsClient {
    fn ui_knowledge(&mut self, ui: &mut egui::Ui) {
        self.observe_knowledge_navigation();
        let p = palette();
        let width = ui.available_width();
        let editing = self.desktop_ui.navigation.editor_visible && !self.knowledge_trash_mode;
        let canvas_visible = !editing
            && !self.knowledge_trash_mode
            && self.knowledge_view == KnowledgeCollectionView::Canvas;
        let navigator = !self.desktop_ui.experience.navigator_hidden
            && !self.desktop_ui.experience.focus_mode
            && width >= 880.0;
        let inspector = editing && self.desktop_ui.experience.inspector.is_some();
        let docked = inspector
            && width
                - if navigator {
                    self.desktop_ui.experience.navigator_width
                } else {
                    0.0
                }
                >= 870.0;
        if navigator {
            let panel = egui::SidePanel::left("knowledge_navigator")
                .default_width(self.desktop_ui.experience.navigator_width)
                .width_range(200.0..=300.0)
                .resizable(true)
                .frame(
                    egui::Frame::none()
                        .fill(p.bg)
                        .inner_margin(egui::Margin::symmetric(14.0, 16.0)),
                )
                .show_inside(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                    ui.horizontal(|ui| {
                        if knowledge_quiet_button(ui, "知识库", "全部资料").clicked() {
                            self.return_to_document_list();
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.menu_button("＋", |ui| {
                                self.ui_knowledge_create_menu(ui);
                            });
                        });
                    });
                    ui.add_space(12.0);
                    if editing {
                        self.ui_knowledge_search(ui);
                    } else if knowledge_quiet_button(ui, "查找页面   Ctrl+K", "快速跳转").clicked()
                    {
                        self.open_knowledge_quick_switch();
                    }
                    ui.add_space(16.0);
                    self.ui_knowledge_tree(ui, ui.available_height().max(80.0));
                });
            self.desktop_ui.experience.navigator_width = panel.response.rect.width();
        }
        if docked {
            egui::SidePanel::right("knowledge_inspector_dock")
                .exact_width(286.0)
                .resizable(false)
                .frame(egui::Frame::none().fill(p.bg))
                .show_inside(ui, |ui| {
                    egui::Frame::none()
                        .inner_margin(14.0)
                        .show(ui, |ui| self.ui_knowledge_inspector(ui));
                });
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(p.panel)
                    .inner_margin(egui::Margin::symmetric(
                        if width < 700.0 { 16.0 } else { 28.0 },
                        20.0,
                    )),
            )
            .show_inside(ui, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
                if canvas_visible {
                    self.ui_knowledge_canvas(ui);
                } else {
                    egui::ScrollArea::vertical()
                        .id_source((
                            "knowledge_content_scroll",
                            self.selected_note_id.clone(),
                            editing,
                            self.knowledge_trash_mode,
                        ))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let full = self
                                .desktop_ui
                                .knowledge
                                .page
                                .as_ref()
                                .is_some_and(|m| m.full_width || m.database.is_some());
                            let max_width = if editing && !full { 820.0 } else { 1100.0 };
                            let available = ui.available_width();
                            let content_width = available.min(max_width);
                            ui.horizontal_top(|ui| {
                                ui.add_space(((available - content_width) / 2.0).max(0.0));
                                ui.vertical(|ui| {
                                    ui.set_width(content_width);
                                    self.ui_knowledge_import_preview(ui);
                                    if self.knowledge_folder_manager_open
                                        && !self.knowledge_trash_mode
                                    {
                                        self.ui_knowledge_folder_manager(ui);
                                        ui.add_space(16.0);
                                    }
                                    if self.knowledge_ai_panel_open && !self.knowledge_trash_mode {
                                        self.ui_knowledge_ai_panel(ui);
                                        ui.add_space(16.0);
                                    }
                                    if self.knowledge_trash_mode {
                                        ui.horizontal(|ui| {
                                            ui.heading("回收站");
                                            if ui
                                                .add_enabled(
                                                    self.view_cache.trash_note_count > 0,
                                                    egui::Button::new("清空回收站"),
                                                )
                                                .clicked()
                                            {
                                                self.desktop_ui.pending_knowledge_delete = Some((
                                                    KnowledgeDeleteIntent::Empty,
                                                    self.data_version,
                                                ));
                                            }
                                        });
                                        self.ui_knowledge_search(ui);
                                        self.ui_knowledge_trash(ui);
                                    } else if self.desktop_ui.navigation.editor_visible {
                                        self.ui_note_editor(ui);
                                    } else {
                                        self.ui_knowledge_library(ui);
                                    }
                                    ui.add_space(100.0);
                                });
                            });
                        });
                }
            });
        let overlay_enabled = ui.is_enabled();
        if !navigator && self.desktop_ui.experience.navigator_drawer_open {
            let mut open = true;
            let selected = self.selected_note_id.clone();
            egui::Window::new("页面树")
                .id(egui::Id::new("knowledge_navigator_drawer"))
                .open(&mut open)
                .default_width(300.0)
                .default_height(400.0)
                .collapsible(false)
                .anchor(egui::Align2::LEFT_TOP, [70.0, 62.0])
                .show(ui.ctx(), |ui| {
                    ui.set_enabled(overlay_enabled);
                    self.ui_knowledge_search(ui);
                    self.ui_knowledge_tree(ui, ui.available_height().max(220.0));
                });
            if !open || selected != self.selected_note_id {
                self.desktop_ui.experience.navigator_drawer_open = false;
            }
        }
        if inspector && !docked {
            let mut open = true;
            egui::Window::new("页面面板")
                .id(egui::Id::new("knowledge_inspector_window"))
                .open(&mut open)
                .title_bar(false)
                .collapsible(false)
                .resizable(true)
                .default_width(320.0)
                .default_height(430.0)
                .anchor(egui::Align2::RIGHT_TOP, [-20.0, 62.0])
                .show(ui.ctx(), |ui| {
                    ui.set_enabled(overlay_enabled);
                    self.ui_knowledge_inspector(ui);
                });
            if !open {
                self.desktop_ui.experience.inspector = None;
            }
        }
    }
    fn ui_knowledge_collection_controls(&mut self, ui: &mut egui::Ui) {
        card_frame().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for view in [
                    KnowledgeCollectionView::Recent,
                    KnowledgeCollectionView::Board,
                    KnowledgeCollectionView::Library,
                    KnowledgeCollectionView::Canvas,
                ] {
                    desktop_segment(ui, &mut self.knowledge_view, view, view.label());
                }
                if self.knowledge_view != KnowledgeCollectionView::Canvas {
                    self.desktop_ui.knowledge_canvas.connect_mode = false;
                    self.desktop_ui.knowledge_canvas.connect_from = None;
                }
                ui.separator();
                let mut filter = self.knowledge_quick_filter;
                egui::ComboBox::from_id_source("knowledge_quick_filter")
                    .selected_text(filter.label())
                    .show_ui(ui, |ui| {
                        for value in KnowledgeQuickFilter::ALL {
                            ui.selectable_value(&mut filter, value, value.label());
                        }
                    });
                if filter != self.knowledge_quick_filter {
                    self.knowledge_quick_filter = filter;
                    self.view_cache.data_version = u64::MAX;
                    self.rebuild_view_cache();
                }

                let current_folder = self.data.note_preferences.selected_folder_id.clone();
                let mut folder = current_folder.clone();
                let folders = self.data.note_folders.clone();
                let folder_label = folder
                    .as_ref()
                    .and_then(|id| folders.iter().find(|item| &item.id == id))
                    .map(|item| item.name.as_str())
                    .unwrap_or("全部文件夹");
                egui::ComboBox::from_id_source("knowledge_folder_filter")
                    .selected_text(folder_label)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut folder, None, "全部文件夹");
                        for item in &folders {
                            ui.selectable_value(&mut folder, Some(item.id.clone()), &item.name);
                        }
                    });
                if folder != current_folder {
                    self.set_knowledge_folder_filter(folder);
                }

                let current_sort = normalize_note_sort_mode(&self.data.note_preferences.sort_mode);
                let mut sort = current_sort;
                egui::ComboBox::from_id_source("knowledge_sort")
                    .selected_text(match sort {
                        1 => "创建时间（新到旧）",
                        2 => "创建时间（旧到新）",
                        3 => "标题 A-Z",
                        _ => "最近更新",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut sort, 0, "最近更新");
                        ui.selectable_value(&mut sort, 1, "创建时间（新到旧）");
                        ui.selectable_value(&mut sort, 2, "创建时间（旧到新）");
                        ui.selectable_value(&mut sort, 3, "标题 A-Z");
                    });
                if sort != current_sort {
                    self.set_knowledge_sort_mode(sort);
                }
            });
            ui.add_space(8.0);
            self.ui_knowledge_search(ui);
            if self.knowledge_quick_filter != KnowledgeQuickFilter::All
                || self.data.note_preferences.selected_folder_id.is_some()
            {
                if ui.small_button("清除筛选").clicked() {
                    self.knowledge_quick_filter = KnowledgeQuickFilter::All;
                    self.set_knowledge_folder_filter(None);
                    self.rebuild_view_cache();
                }
            }
        });
    }

    fn ui_knowledge_search(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        ui.horizontal(|ui| {
            let clear_width = if self.note_search_draft.is_empty() {
                0.0
            } else {
                48.0
            };
            changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut self.note_search_draft)
                        .id(egui::Id::new("notes_search"))
                        .desired_width((ui.available_width() - clear_width).max(80.0))
                        .hint_text(if self.knowledge_trash_mode {
                            "搜索已删除文档"
                        } else {
                            "搜索标题、正文、附件或文件夹"
                        }),
                )
                .changed();
            if !self.note_search_draft.is_empty() && ui.small_button("清除").clicked() {
                self.note_search_draft.clear();
                changed = true;
            }
        });
        if changed {
            self.view_cache.data_version = u64::MAX;
            self.rebuild_view_cache();
        }
    }

    fn ui_knowledge_collection_list(&mut self, ui: &mut egui::Ui, max_height: f32) {
        if self.knowledge_view == KnowledgeCollectionView::Recent {
            self.ui_knowledge_tree(ui, max_height);
            return;
        }
        let notes = self.view_cache.notes.clone();
        let mut clicked = None;
        let mut convert_sticky = None;
        egui::ScrollArea::vertical()
            .id_source("knowledge_collection_scroll")
            .max_height(max_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                match self.knowledge_view {
                    KnowledgeCollectionView::Recent => {
                        for note in notes.iter() {
                            if note_row(ui, note, self.selected_note_id == note.id).clicked() {
                                clicked = Some(note.id.clone());
                            }
                            ui.add_space(8.0);
                        }
                        let sticky_notes = self
                            .data
                            .notes
                            .iter()
                            .filter(|note| {
                                desktop_note_kind(note) == DesktopNoteKind::Sticky
                                    && note.deleted_at_epoch_millis.is_none()
                            })
                            .take(12)
                            .cloned()
                            .collect::<Vec<_>>();
                        if !sticky_notes.is_empty() {
                            ui.separator();
                            ui.label(egui::RichText::new("从便签加入资料库").strong());
                            for note in &sticky_notes {
                                ui.horizontal(|ui| {
                                    ui.label(if note.title.trim().is_empty() {
                                        "未命名便签"
                                    } else {
                                        &note.title
                                    });
                                    if ui.button("转为文档").clicked() {
                                        convert_sticky = Some(note.id.clone());
                                    }
                                });
                            }
                        }
                    }
                    KnowledgeCollectionView::Board => {
                        let folder_names = self
                            .data
                            .note_folders
                            .iter()
                            .map(|folder| (folder.id.clone(), folder.name.clone()))
                            .collect::<BTreeMap<_, _>>();
                        let mut groups = BTreeMap::<String, Vec<&DesktopNoteListItem>>::new();
                        for note in notes.iter() {
                            let name = note
                                .folder_id
                                .as_ref()
                                .and_then(|id| folder_names.get(id))
                                .cloned()
                                .unwrap_or_else(|| "未入库".to_string());
                            groups.entry(name).or_default().push(note);
                        }
                        for (name, group) in groups {
                            ui.label(egui::RichText::new(name).strong().color(palette().text));
                            ui.add_space(4.0);
                            for note in &group {
                                if note_row(ui, note, self.selected_note_id == note.id).clicked() {
                                    clicked = Some(note.id.clone());
                                }
                                ui.add_space(6.0);
                            }
                            ui.add_space(8.0);
                        }
                    }
                    KnowledgeCollectionView::Library => {
                        let counts = knowledge_library_folder_counts(
                            notes.as_ref(),
                            &self.data.note_folders,
                        );
                        for (name, count) in counts {
                            pill(
                                ui,
                                &format!("{name} · {count}"),
                                palette().panel_alt,
                                palette().muted,
                            );
                            ui.add_space(6.0);
                        }
                        ui.separator();
                        for note in notes.iter() {
                            if note_row(ui, note, self.selected_note_id == note.id).clicked() {
                                clicked = Some(note.id.clone());
                            }
                            ui.add_space(8.0);
                        }
                    }
                    KnowledgeCollectionView::Canvas => {}
                }
                if notes.is_empty() {
                    empty_state(
                        ui,
                        if self.view_cache.active_note_count == 0 {
                            "还没有知识文档"
                        } else {
                            "没有匹配内容"
                        },
                    );
                }
            });
        if let Some(note_id) = convert_sticky {
            self.convert_sticky_to_document(&note_id);
        } else if let Some(note_id) = clicked {
            self.select_note_by_id(&note_id);
        }
    }

    fn ui_knowledge_trash(&mut self, ui: &mut egui::Ui) {
        let notes = self.view_cache.notes.clone();
        if notes.is_empty() {
            empty_state(ui, "知识回收站为空");
            return;
        }
        let mut restore = None;
        let mut delete = None;
        for note in notes.iter() {
            card_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new(if note.title.trim().is_empty() {
                                "未命名文档"
                            } else {
                                &note.title
                            })
                            .strong(),
                        );
                        ui.label(
                            egui::RichText::new(note.preview("未命名"))
                                .size(12.0)
                                .color(palette().muted),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if action_button(ui, "彻底删除", ButtonTone::Danger).clicked() {
                            delete = Some(note.id.clone());
                        }
                        if action_button(ui, "恢复", ButtonTone::Quiet).clicked() {
                            restore = Some(note.id.clone());
                        }
                    });
                });
            });
            ui.add_space(8.0);
        }
        if let Some(note_id) = restore {
            self.restore_knowledge_note(&note_id);
        } else if let Some(note_id) = delete {
            self.desktop_ui.pending_knowledge_delete =
                Some((KnowledgeDeleteIntent::Permanent(note_id), self.data_version));
        }
    }

    fn ui_knowledge_folder_manager(&mut self, ui: &mut egui::Ui) {
        card_frame().show(ui, |ui| {
            section_heading(ui, "文件夹管理");
            ui.add_space(8.0);
            let folders = self.data.note_folders.clone();
            ui.horizontal_wrapped(|ui| {
                for folder in &folders {
                    let selected =
                        self.knowledge_folder_edit_id.as_deref() == Some(folder.id.as_str());
                    if ui.selectable_label(selected, &folder.name).clicked() {
                        self.knowledge_folder_edit_id = Some(folder.id.clone());
                        self.knowledge_folder_name_draft = folder.name.clone();
                    }
                }
                if ui.button("＋ 新文件夹").clicked() {
                    self.knowledge_folder_edit_id = None;
                    self.knowledge_folder_name_draft.clear();
                }
            });
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.knowledge_folder_name_draft)
                        .desired_width(ui.available_width().min(260.0))
                        .hint_text("文件夹名称"),
                );
                let save = ui.add_enabled(
                    !self.knowledge_folder_name_draft.trim().is_empty(),
                    egui::Button::new("保存文件夹"),
                );
                #[cfg(test)]
                ui.ctx().data_mut(|data| {
                    data.insert_temp(
                        egui::Id::new("knowledge_folder_save"),
                        (save.rect, save.enabled()),
                    )
                });
                if save.clicked() {
                    self.save_knowledge_folder();
                }
                if self.knowledge_folder_edit_id.is_some()
                    && action_button(ui, "删除", ButtonTone::Danger).clicked()
                {
                    self.desktop_ui.pending_knowledge_delete = Some((
                        KnowledgeDeleteIntent::Folder(
                            self.knowledge_folder_edit_id.clone().unwrap(),
                        ),
                        self.data_version,
                    ));
                }
            });
        });
    }

    fn ui_knowledge_ai_panel(&mut self, ui: &mut egui::Ui) {
        card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                section_heading(ui, "AI 问答");
                if ui.button("关闭").clicked() {
                    self.cancel_desktop_ai_query();
                    self.knowledge_ai_panel_open = false;
                }
            });
            let recipient = desktop_ai_configured_recipient(&self.sync);
            ui.label(match &recipient {
                Ok(host) => format!("{} · {}", self.sync.ai_model, host),
                Err(error) => error.clone(),
            });
            if ui.button("更改 AI 设置").clicked() {
                self.desktop_ai.preview = None;
                self.switch_tab(AppTab::My);
                self.ai_settings_expanded = true;
            }
            let old_mode = self.desktop_ai.mode;
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.desktop_ai.mode, QueryMode::Direct, "直接问 AI");
                ui.radio_value(&mut self.desktop_ai.mode, QueryMode::Knowledge, "问知识库");
            });
            if self.desktop_ai.mode != old_mode {
                self.desktop_ai.preview = None;
            }
            if self.desktop_ai.mode == QueryMode::Knowledge {
                let old_scope = self.knowledge_ai_scope_all;
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.knowledge_ai_scope_all, true, "全部知识页");
                    ui.add_enabled_ui(
                        self.data.note_preferences.selected_folder_id.is_some(),
                        |ui| {
                            ui.radio_value(&mut self.knowledge_ai_scope_all, false, "当前文件夹");
                        },
                    );
                });
                if old_scope != self.knowledge_ai_scope_all {
                    self.desktop_ai.preview = None;
                }
                ui.label("发送相关知识页节选；答案依据本次来源。加密和已删除的内容不纳入。");
            } else {
                ui.label("使用模型的通用知识与推理，仅发送你的问题。");
            }
            ui.label(format!(
                "你的问题 · {}/1000",
                self.knowledge_ai_question_draft.chars().count()
            ));
            let changed = ui
                .add(
                    egui::TextEdit::multiline(&mut self.knowledge_ai_question_draft)
                        .desired_width(f32::INFINITY)
                        .desired_rows(3)
                        .hint_text("输入问题，例如解释复利并给出公式"),
                )
                .changed();
            if changed {
                self.desktop_ai.preview = None;
            }
            let valid_question = !self.knowledge_ai_question_draft.trim().is_empty()
                && self.knowledge_ai_question_draft.chars().count() <= 1000;
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !self.knowledge_ai_pending
                            && valid_question
                            && recipient.is_ok()
                            && self.workspace_persistence_ready
                            && !self.workspace_edit_locked()
                            && self.background_work_is_allowed(),
                        egui::Button::new("预览并发送问题"),
                    )
                    .clicked()
                {
                    self.launch_knowledge_ai();
                }
                if self.knowledge_ai_pending && ui.button("取消本次请求").clicked() {
                    self.cancel_desktop_ai_query();
                }
            });
            if !self.desktop_ai.message.is_empty() {
                ui.label(&self.desktop_ai.message);
            }
            if let Some(answer) = self.knowledge_ai_answer.clone() {
                ui.separator();
                ui.label(format!(
                    "回答来自 {} · {} · {}",
                    answer.recipient_host,
                    answer.model,
                    format_relative_time(answer.received_at_epoch_millis)
                ));
                ui.horizontal(|ui| {
                    if ui.button("查看排版与公式").clicked() {
                        self.open_desktop_ai_answer_reader();
                    }
                    if ui.button("复制回答原文").clicked() {
                        ui.output_mut(|output| output.copied_text = answer.content.clone());
                    }
                    if ui.button("答案另存为知识页").clicked() {
                        self.save_knowledge_answer_as_document();
                    }
                });
                egui::CollapsingHeader::new("回答原文")
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&answer.content).color(palette().text),
                            )
                            .wrap(true),
                        );
                    });
                if !answer.source_ids.is_empty() {
                    ui.label(egui::RichText::new("本次知识来源").strong());
                    let mut open_source = None;
                    for ((id, title), folder) in answer
                        .source_ids
                        .iter()
                        .zip(answer.source_titles.iter())
                        .zip(answer.source_folders.iter())
                    {
                        if ui.button(format!("{title} · {folder}")).clicked() {
                            open_source = Some(id.clone());
                        }
                    }
                    if let Some(note_id) = open_source {
                        self.open_knowledge_source(&note_id);
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod knowledge_library_count_tests {
    use super::*;

    fn note_in(folder_id: Option<&str>) -> DesktopNoteListItem {
        DesktopNoteListItem {
            id: String::new(),
            title: String::new(),
            row_title: String::new(),
            folder_id: folder_id.map(str::to_owned),
            body_preview: String::new(),
            metadata_prefix: String::new(),
            search_snippet: String::new(),
            encrypted: false,
            is_database: false,
            updated_at_epoch_millis: 0,
        }
    }

    #[test]
    fn library_counts_keep_first_folder_and_place_missing_ids_in_unfiled() {
        let folders = [
            DesktopNoteFolder {
                id: "known".into(),
                name: "资料".into(),
                ..Default::default()
            },
            DesktopNoteFolder {
                id: "known".into(),
                name: "重复目录".into(),
                ..Default::default()
            },
        ];
        let notes = [
            note_in(Some("known")),
            note_in(Some("known")),
            note_in(Some("missing")),
            note_in(None),
        ];
        let counts = knowledge_library_folder_counts(&notes, &folders);
        assert_eq!(counts.get("资料"), Some(&2));
        assert_eq!(counts.get("未入库"), Some(&2));
        assert!(!counts.contains_key("重复目录"));
    }

    #[test]
    #[ignore = "Explicit isolated library count benchmark"]
    fn library_folder_count_benchmark() {
        let folders = (0..512)
            .map(|index| DesktopNoteFolder {
                id: format!("folder-{index}"),
                name: format!("目录 {index}"),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let notes = (0..12_000)
            .map(|index| note_in(Some(&folders[index % folders.len()].id)))
            .collect::<Vec<_>>();
        let legacy = || {
            let mut counts = BTreeMap::<String, usize>::new();
            for note in &notes {
                let name = note
                    .folder_id
                    .as_ref()
                    .and_then(|id| folders.iter().find(|folder| &folder.id == id))
                    .map(|folder| folder.name.clone())
                    .unwrap_or_else(|| "未入库".to_string());
                *counts.entry(name).or_default() += 1;
            }
            counts
        };
        let old_start = Instant::now();
        let expected = std::hint::black_box(legacy());
        let old_elapsed = old_start.elapsed();
        let new_start = Instant::now();
        let actual = std::hint::black_box(knowledge_library_folder_counts(&notes, &folders));
        let new_elapsed = new_start.elapsed();
        assert_eq!(
            expected,
            actual
                .iter()
                .map(|(name, count)| ((*name).to_owned(), *count))
                .collect()
        );
        println!(
            "LIBRARY_FOLDER_COUNT_BENCHMARK notes={} folders={} legacy_ms={:.3} indexed_ms={:.3}",
            notes.len(),
            folders.len(),
            old_elapsed.as_secs_f64() * 1000.0,
            new_elapsed.as_secs_f64() * 1000.0,
        );
    }
}
