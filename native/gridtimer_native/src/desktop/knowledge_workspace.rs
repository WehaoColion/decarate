// Windows - Share immutable database query IDs without per-frame string copies.
// v1.0.2.1 Windows - Render only visible knowledge navigation rows.
// v2.22.54 - Reuse lightweight navigation and immutable query records between data changes.
// v2.22.53 - Focused knowledge workspace, contextual controls and safe navigation.
// v2.22.52 - Page metadata and workspace actions use the existing atomic draft path.
use gridtimer_native::knowledge;
include!("knowledge_read_cache.rs");

#[derive(Default)]
struct KnowledgeWorkspaceState {
    page: Option<knowledge::KnowledgePage>,
    collapsed: std::collections::HashSet<String>,
    active_view: String,
    options_open: bool,
    field_editor: Option<knowledge::DatabaseField>,
    cell_inputs: BTreeMap<String, KnowledgeCellInput>,
    form_title: String,
    form_values: BTreeMap<String, knowledge::CellValue>,
    comment: String,
    version_name: String,
    compare_revision: String,
    comment_block: String,
    import_path: String,
    import_workspace: String,
    import_pages: Option<Vec<Value>>,
    import_media: Vec<KnowledgeTransferMedia>,
    export_path: String,
    external_url: Option<String>,
    extract_attachment: Option<String>,
    calendar_offset: i64,
    error: String,
    read_cache: std::cell::RefCell<KnowledgeReadCache>,
    query_key: Option<KnowledgeQueryKey>,
    query_rows: Arc<Vec<String>>,
    query_cells: BTreeMap<String, BTreeMap<String, Result<knowledge::CellValue, String>>>,
}

impl KnowledgeWorkspaceState {
    fn clear_draft(&mut self) {
        self.page = None;
        self.external_url = None;
        self.extract_attachment = None;
        self.query_key = None;
        self.read_cache.get_mut().rich_preview = None;
        self.query_rows = Arc::default();
        self.query_cells.clear();
        self.active_view.clear();
        self.field_editor = None;
        self.cell_inputs.clear();
        self.form_title.clear();
        self.form_values.clear();
        self.comment.clear();
        self.version_name.clear();
        self.compare_revision.clear();
        self.comment_block.clear();
        self.error.clear();
    }
}

fn apply_knowledge_to_note_value(value: &mut Value, page: Option<&knowledge::KnowledgePage>) {
    if let Some(page) = page {
        value["document"]["knowledge"] = serde_json::to_value(page).unwrap_or(Value::Null);
    }
}

fn knowledge_record(note: &DesktopNote) -> knowledge::PageRecord {
    knowledge::PageRecord {
        id: note.id.clone(),
        title: note.title.clone(),
        content: note_body_text(note),
        meta: note.document.knowledge.clone().unwrap_or_default(),
        created_at: note.created_at_epoch_millis,
        updated_at: note.updated_at_epoch_millis,
        deleted: note.deleted_at_epoch_millis.is_some(),
        encrypted: note.encryption.is_some(),
    }
}

impl TimerWindowsClient {
    fn knowledge_records(&self) -> Arc<Vec<knowledge::PageRecord>> {
        let mut cache = self.desktop_ui.knowledge.read_cache.borrow_mut();
        cache.at_version(self.notes_cache_version());
        cache
            .records
            .get_or_insert_with(|| {
                Arc::new(
                    self.data
                        .notes
                        .iter()
                        .filter(|note| desktop_note_kind(note) == DesktopNoteKind::Document)
                        .map(knowledge_record)
                        .collect(),
                )
            })
            .clone()
    }

    fn knowledge_page_locked(&self) -> bool {
        self.selected_note_is_locked()
            || self
                .desktop_ui
                .knowledge
                .page
                .as_ref()
                .is_some_and(|p| p.locked)
    }

    fn update_knowledge_page(&mut self, page: knowledge::KnowledgePage) -> bool {
        if let Err(error) = page.validate() {
            self.status = error;
            return false;
        }
        if self.selected_note_is_locked() {
            return false;
        }
        let before = self.current_note_draft_snapshot();
        self.desktop_ui.knowledge.page = Some(page);
        self.finish_note_structural_change(before);
        true
    }

    fn create_knowledge_page(&mut self, parent: Option<String>, database: bool) -> bool {
        if let Some(id) = parent.as_deref() {
            if !self.data.notes.iter().any(|p| {
                p.id == id
                    && p.encryption.is_none()
                    && p.deleted_at_epoch_millis.is_none()
                    && !p.document.knowledge.as_ref().is_some_and(|m| m.locked)
            }) {
                self.status = "父页面不可编辑".into();
                return false;
            }
        }
        if !self.new_note_with_kind(DesktopNoteKind::Document) {
            return false;
        }
        self.desktop_ui.knowledge.page = Some(knowledge::KnowledgePage {
            parent_id: parent,
            database: database.then(knowledge::KnowledgeDatabase::task_database),
            icon: if database { "▦" } else { "" }.into(),
            ..Default::default()
        });
        self.note_title_draft = if database {
            "新数据库"
        } else {
            "新页面"
        }
        .into();
        self.mark_note_dirty();
        self.save_note()
    }

    fn move_knowledge_page(&mut self, id: &str, parent: Option<&str>, order: i64) -> bool {
        if self.flush_note_draft().is_err() {
            return false;
        }
        if !knowledge::can_move_page(&self.knowledge_records(), id, parent) {
            self.status = "无法移动到此页面：存在循环、锁定或无效父页面".into();
            return false;
        }
        let Some(mut note) = self.data.notes.iter().find(|p| p.id == id).cloned() else {
            return false;
        };
        let meta = note.document.knowledge.get_or_insert_with(Default::default);
        meta.parent_id = parent.map(str::to_string);
        meta.order = order;
        self.replace_state(
            app_data::upsert_note_app_data_json(
                &self.state_json,
                &serde_json::to_string(&note).unwrap_or_default(),
                now_millis(),
            ),
            "页面已移动",
        )
    }

    fn place_knowledge_page_edge(&mut self, id: &str, last: bool) -> bool {
        if self.flush_note_draft().is_err() {
            return false;
        }
        let Some(page) = self.data.notes.iter().find(|p| p.id == id).cloned() else {
            return false;
        };
        let parent = page
            .document
            .knowledge
            .as_ref()
            .and_then(|m| m.parent_id.clone());
        let orders = self
            .data
            .notes
            .iter()
            .filter(|p| {
                p.id != id
                    && p.deleted_at_epoch_millis.is_none()
                    && p.document
                        .knowledge
                        .as_ref()
                        .and_then(|m| m.parent_id.as_ref())
                        == parent.as_ref()
            })
            .map(|p| p.document.knowledge.as_ref().map_or(0, |m| m.order));
        let order = if last {
            orders.max().unwrap_or(0).checked_add(1)
        } else {
            orders.min().unwrap_or(0).checked_sub(1)
        };
        let Some(order) = order else {
            self.status = "页面排序已到边界".into();
            return false;
        };
        self.move_knowledge_page(id, parent.as_deref(), order)
    }

    fn ui_knowledge_tree(&mut self, ui: &mut egui::Ui, max_height: f32) {
        let notes = self.view_cache.notes.clone();
        if !self.note_search_draft.trim().is_empty()
            || self.knowledge_quick_filter != KnowledgeQuickFilter::All
            || self.data.note_preferences.selected_folder_id.is_some()
        {
            egui::ScrollArea::vertical()
                .id_source("knowledge_filtered_tree")
                .max_height(max_height)
                .show(ui, |ui| {
                    for note in notes.iter() {
                        if note_row(ui, note, self.selected_note_id == note.id).clicked() {
                            self.select_note_by_id(&note.id);
                        }
                        if !note.search_snippet.is_empty() {
                            ui.label(
                                egui::RichText::new(&note.search_snippet)
                                    .small()
                                    .color(palette().muted),
                            );
                        }
                        ui.add_space(8.0);
                    }
                    if notes.is_empty() {
                        empty_state(ui, "没有匹配的页面");
                    }
                });
            return;
        }
        let all = self.knowledge_navigation();
        let rows = self.knowledge_tree_rows(&all);
        let dragging = egui::DragAndDrop::has_payload_of_type::<KnowledgePageDrag>(ui.ctx());
        egui::ScrollArea::vertical()
            .id_source("knowledge_page_tree")
            .max_height(max_height)
            .auto_shrink([false, false])
            .show_rows(
                ui,
                32.0,
                rows.len() + usize::from(dragging),
                |ui, visible| {
                    for row_index in visible {
                        let row_id = match rows.get(row_index) {
                            Some(KnowledgeTreeRow::PinnedHeading) => {
                                egui::Id::new("pinned_heading")
                            }
                            Some(KnowledgeTreeRow::Pinned(index)) => {
                                egui::Id::new(("pinned_page", &all.pages[*index].id))
                            }
                            Some(KnowledgeTreeRow::PagesHeading) => egui::Id::new("pages_heading"),
                            Some(KnowledgeTreeRow::Page { index, .. }) => {
                                egui::Id::new(("tree_page", &all.pages[*index].id))
                            }
                            None => egui::Id::new("root_drop_target"),
                        };
                        ui.push_id(row_id, |ui| match rows.get(row_index) {
                            Some(KnowledgeTreeRow::PinnedHeading) => {
                                ui.add_sized(
                                    [ui.available_width(), 32.0],
                                    egui::Label::new(
                                        egui::RichText::new("收藏").small().color(palette().muted),
                                    ),
                                );
                            }
                            Some(KnowledgeTreeRow::Pinned(index)) => {
                                let p = &all.pages[*index];
                                let title = if p.encrypted {
                                    "加密页面"
                                } else if p.title.is_empty() {
                                    "未命名"
                                } else {
                                    &p.title
                                };
                                let (rect, response) = ui.allocate_exact_size(
                                    egui::vec2(ui.available_width(), 32.0),
                                    egui::Sense::click(),
                                );
                                let selected = self.selected_note_id == p.id;
                                if selected || response.hovered() {
                                    ui.painter().rect_filled(
                                        rect,
                                        5.0,
                                        if selected {
                                            palette().selected
                                        } else {
                                            palette().panel_alt
                                        },
                                    );
                                }
                                ui.painter()
                                    .with_clip_rect(rect.shrink2(egui::vec2(8.0, 0.0)))
                                    .text(
                                        rect.left_center() + egui::vec2(8.0, 0.0),
                                        egui::Align2::LEFT_CENTER,
                                        title,
                                        egui::FontId::proportional(13.0),
                                        palette().text,
                                    );
                                response.widget_info(|| {
                                    egui::WidgetInfo::selected(
                                        egui::WidgetType::SelectableLabel,
                                        selected,
                                        title,
                                    )
                                });
                                if response.on_hover_text(title).clicked() {
                                    self.select_note_by_id(&p.id);
                                }
                            }
                            Some(KnowledgeTreeRow::PagesHeading) => {
                                ui.horizontal(|ui| {
                                    ui.set_height(32.0);
                                    ui.label(
                                        egui::RichText::new("页面").small().color(palette().muted),
                                    );
                                    if ui
                                        .small_button("＋")
                                        .on_hover_text("新建顶层页面")
                                        .clicked()
                                    {
                                        self.create_knowledge_page(None, false);
                                    }
                                    if ui.small_button("＋ 数据库").clicked() {
                                        self.create_knowledge_page(None, true);
                                    }
                                });
                            }
                            Some(KnowledgeTreeRow::Page { index, depth }) => {
                                self.ui_knowledge_tree_item(ui, &all, &all.pages[*index], *depth);
                            }
                            None => {
                                let root = ui.add_sized(
                                    [ui.available_width(), 32.0],
                                    egui::Label::new(
                                        egui::RichText::new("移至顶层")
                                            .small()
                                            .color(palette().muted),
                                    )
                                    .sense(egui::Sense::hover()),
                                );
                                if let Some(payload) =
                                    root.dnd_release_payload::<KnowledgePageDrag>()
                                {
                                    self.move_knowledge_page(&payload.0, None, now_millis());
                                }
                            }
                        });
                    }
                },
            );
        if all.pages.is_empty() {
            empty_state(ui, "创建页面，开始整理资料");
        }
    }

    fn ui_knowledge_tree_item(
        &mut self,
        ui: &mut egui::Ui,
        all: &KnowledgeNavigation,
        page: &KnowledgeNavigationPage,
        depth: usize,
    ) {
        let children = !all.children_of(Some(&page.id)).is_empty();
        let collapsed = self.desktop_ui.knowledge.collapsed.contains(&page.id);
        ui.horizontal(|ui| {
            ui.set_height(32.0);
            ui.add_space((depth.min(12) * 14) as f32);
            if children {
                if ui.small_button(if collapsed { "▸" } else { "▾" }).clicked() {
                    if collapsed {
                        self.desktop_ui.knowledge.collapsed.remove(&page.id);
                    } else {
                        self.desktop_ui.knowledge.collapsed.insert(page.id.clone());
                    }
                    ui.ctx().request_repaint();
                }
            } else {
                ui.add_space(18.0);
            }
            let label = if page.encrypted {
                "🔒 加密页面".into()
            } else {
                format!(
                    "{} {}",
                    if page.icon.is_empty() {
                        if page.database {
                            "▦"
                        } else {
                            "▤"
                        }
                    } else {
                        &page.icon
                    },
                    if page.title.is_empty() {
                        "未命名"
                    } else {
                        &page.title
                    }
                )
            };
            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), 32.0),
                egui::Sense::click_and_drag(),
            );
            if !ui.is_rect_visible(rect) {
                return;
            }
            #[cfg(test)]
            ui.ctx().data_mut(|data| {
                data.insert_temp(egui::Id::new(("knowledge_tree_row", &page.id)), rect);
            });
            let selected =
                self.selected_note_id == page.id && self.desktop_ui.navigation.editor_visible;
            if selected || response.hovered() {
                ui.painter().rect_filled(
                    rect,
                    5.0,
                    if selected {
                        palette().selected
                    } else {
                        palette().panel_alt
                    },
                );
            }
            let painter = ui
                .painter()
                .with_clip_rect(rect.shrink2(egui::vec2(8.0, 0.0)));
            painter.text(
                rect.left_center() + egui::vec2(8.0, 0.0),
                egui::Align2::LEFT_CENTER,
                &label,
                egui::FontId::proportional(13.0),
                palette().text,
            );
            response.widget_info(|| {
                egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, selected, &label)
            });
            let response = response.on_hover_text(&label);
            response.dnd_set_drag_payload(KnowledgePageDrag(page.id.clone()));
            if response.clicked() {
                self.select_note_by_id(&page.id);
            }
            if let Some(payload) = response.dnd_release_payload::<KnowledgePageDrag>() {
                self.move_knowledge_page(&payload.0, Some(&page.id), now_millis());
            }
            response.context_menu(|ui| {
                if ui
                    .add_enabled(
                        !page.encrypted && !page.locked,
                        egui::Button::new("新建子页面"),
                    )
                    .clicked()
                {
                    self.create_knowledge_page(Some(page.id.clone()), false);
                    ui.close_menu();
                }
                if ui
                    .add_enabled(
                        !page.encrypted && !page.locked,
                        egui::Button::new("新建子数据库"),
                    )
                    .clicked()
                {
                    self.create_knowledge_page(Some(page.id.clone()), true);
                    ui.close_menu();
                }
                if ui
                    .button(if page.pinned {
                        "取消收藏"
                    } else {
                        "收藏"
                    })
                    .clicked()
                {
                    self.select_note_by_id(&page.id);
                    let before = self.current_note_draft_snapshot();
                    self.note_pinned_draft = !page.pinned;
                    self.finish_note_structural_change(before);
                    self.save_note();
                    ui.close_menu();
                }
                for (label, last) in [("移到同级最前", false), ("移到同级最后", true)] {
                    if ui
                        .add_enabled(!page.encrypted && !page.locked, egui::Button::new(label))
                        .clicked()
                    {
                        self.place_knowledge_page_edge(&page.id, last);
                        ui.close_menu();
                    }
                }
                if ui
                    .add_enabled(!page.locked, egui::Button::new("移至顶层"))
                    .clicked()
                {
                    self.move_knowledge_page(&page.id, None, now_millis());
                    ui.close_menu();
                }
            });
        });
    }

    fn ui_knowledge_page_header(&mut self, ui: &mut egui::Ui) {
        let meta = self.desktop_ui.knowledge.page.clone().unwrap_or_default();
        let p = palette();
        if !meta.cover.is_empty() {
            let color = match meta.cover.as_str() {
                "砂岩" => egui::Color32::from_rgb(204, 172, 132),
                "海盐" => egui::Color32::from_rgb(123, 173, 181),
                "森林" => egui::Color32::from_rgb(124, 159, 127),
                "暮色" => egui::Color32::from_rgb(161, 147, 186),
                _ => p.accent,
            };
            let (rect, _) = ui
                .allocate_exact_size(egui::vec2(ui.available_width(), 74.0), egui::Sense::hover());
            ui.painter().rect_filled(
                rect,
                8.0,
                if p.is_dark {
                    color.gamma_multiply(0.35)
                } else {
                    color.gamma_multiply(0.3)
                },
            );
            let painter = ui.painter().with_clip_rect(rect.shrink(1.0));
            for (offset, radius) in [(40.0, 120.0), (140.0, 85.0), (250.0, 110.0)] {
                painter.circle_filled(
                    rect.right_center() - egui::vec2(offset, -44.0),
                    radius,
                    color.gamma_multiply(0.12),
                );
            }
            ui.add_space(18.0);
        } else {
            ui.add_space(18.0);
        }
        if !meta.icon.is_empty() {
            ui.label(egui::RichText::new(&meta.icon).size(30.0));
            ui.add_space(6.0);
        }
        let mut title = self.note_title_draft.clone();
        let response = ui.add_enabled(
            !meta.locked,
            egui::TextEdit::singleline(&mut title)
                .id(egui::Id::new("note_title_editor"))
                .frame(false)
                .font(egui::FontId::proportional(
                    if ui.available_width() < 480.0 {
                        28.0
                    } else {
                        34.0
                    },
                ))
                .desired_width(f32::INFINITY)
                .hint_text("未命名"),
        );
        self.desktop_ui
            .navigation
            .track_document_text_edit(&response);
        if response.changed() {
            let before = self.begin_note_text_edit("title");
            self.note_title_draft = title.replace(['\n', '\r'], " ").chars().take(240).collect();
            self.finish_note_text_edit(before, "title");
        }
        if self.desktop_ui.note_editor_focus && !meta.locked {
            response.request_focus();
            self.desktop_ui.note_editor_focus = false;
        }
        ui.horizontal_wrapped(|ui| {
            let kind = if meta.database.is_some() {
                "数据库"
            } else {
                "页面"
            };
            ui.label(egui::RichText::new(kind).size(11.0).color(p.muted));
            if meta.locked {
                ui.label(egui::RichText::new("· 已锁定").size(11.0).color(p.muted));
            }
            if self.note_pinned_draft {
                ui.label(egui::RichText::new("· 已收藏").size(11.0).color(p.muted));
            }
            for tag in meta.tags.iter().take(6) {
                ui.label(
                    egui::RichText::new(format!("#{tag}"))
                        .size(11.0)
                        .color(p.accent),
                );
            }
        });
        ui.add_space(16.0);
        // Record fields are part of the record, and remain adjacent to its title.
        self.ui_knowledge_record_properties(ui);
    }

    fn ui_knowledge_page_properties(&mut self, ui: &mut egui::Ui) {
        let editing_id = self.selected_note_id.clone();
        let records = self.knowledge_records();
        let mut meta = self.desktop_ui.knowledge.page.clone().unwrap_or_default();
        let original = meta.clone();
        ui.set_max_width(ui.available_width());
        if ui
            .add_sized(
                [ui.available_width(), 32.0],
                egui::Button::new(if meta.locked {
                    "解锁页面"
                } else {
                    "锁定页面"
                }),
            )
            .clicked()
            && self.flush_note_draft().is_ok()
        {
            meta = self.desktop_ui.knowledge.page.clone().unwrap_or_default();
            meta.locked = !meta.locked;
        }
        ui.add_enabled_ui(!meta.locked, |ui| {
            ui.label(egui::RichText::new("外观").small().color(palette().muted));
            ui.horizontal(|ui| {
                ui.label("图标");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut meta.icon)
                        .desired_width(48.0)
                        .char_limit(8),
                );
                self.desktop_ui
                    .navigation
                    .track_document_text_edit(&response);
            });
            egui::ComboBox::from_id_source("knowledge_cover")
                .width(ui.available_width() - 12.0)
                .selected_text(if meta.cover.is_empty() {
                    "无封面"
                } else {
                    &meta.cover
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut meta.cover, String::new(), "无封面");
                    for name in ["砂岩", "海盐", "森林", "暮色"] {
                        ui.selectable_value(&mut meta.cover, name.into(), name);
                    }
                });
            ui.horizontal(|ui| {
                ui.checkbox(&mut meta.full_width, "通栏");
                ui.checkbox(&mut meta.small_text, "小字号");
            });
            ui.separator();
            ui.label(egui::RichText::new("归类").small().color(palette().muted));
            let mut tags = meta.tags.join(", ");
            let tags_response = ui.add(
                egui::TextEdit::singleline(&mut tags)
                    .desired_width(f32::INFINITY)
                    .hint_text("标签，以逗号分隔"),
            );
            self.desktop_ui
                .navigation
                .track_document_text_edit(&tags_response);
            if tags_response.changed() {
                meta.tags = tags
                    .split([',', '，'])
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect();
            }
            ui.label("上级页面");
            let mut parent = meta.parent_id.clone();
            egui::ComboBox::from_id_source("knowledge_page_parent")
                .width(ui.available_width() - 12.0)
                .selected_text(
                    parent
                        .as_ref()
                        .and_then(|id| records.iter().find(|p| &p.id == id))
                        .map_or("顶层", |p| p.title.as_str()),
                )
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut parent, None, "顶层");
                    for p in records.iter() {
                        if knowledge::can_move_page(&records, &self.selected_note_id, Some(&p.id)) {
                            ui.selectable_value(&mut parent, Some(p.id.clone()), &p.title);
                        }
                    }
                });
            meta.parent_id = parent;
            let pin = ui.checkbox(&mut self.note_pinned_draft, "收藏页面");
            #[cfg(test)]
            ui.ctx()
                .data_mut(|d| d.insert_temp(egui::Id::new("knowledge_pin"), pin.rect));
            if pin.changed() {
                let current = self.note_pinned_draft;
                self.note_pinned_draft = !current;
                let before = self.current_note_draft_snapshot();
                self.note_pinned_draft = current;
                self.finish_note_structural_change(before);
            }
            ui.checkbox(&mut meta.template, "作为模板");
        });
        if meta != original {
            self.update_knowledge_page(meta.clone());
        }
        if editing_id != self.selected_note_id {
            return;
        }
        ui.separator();
        if ui
            .add_enabled(!meta.locked, egui::Button::new("＋ 子页面"))
            .clicked()
        {
            self.create_knowledge_page(Some(editing_id), false);
        }
    }
}

#[derive(Clone)]
struct KnowledgePageDrag(String);

#[derive(Default)]
struct KnowledgeCellInput {
    source: String,
    text: String,
}
