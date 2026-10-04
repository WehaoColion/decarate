// Windows - Cache database record indices and visible row search until source or query changes.
// v1.0.3 Windows - Reuse bounded gallery Markdown previews across frames.
// v1.0.3.10 Windows - Validate pending form inputs and preserve edits across remote refreshes.
// v1.0.3.9 Windows - Render only visible database table rows while preserving focused edits.
// v1.0.2.2 Windows - Reuse database filtering and index page lookups within each frame.
// v2.22.54 - Reuse query inputs and compute draft overlays only when a query changes.
// v2.22.53 - Focused knowledge workspace, contextual controls and safe navigation.
// v2.22.52 - Seven saved views over independently stored record pages.

struct KnowledgeRecordLookup<'a> {
    records: &'a [knowledge::PageRecord],
    by_id: Arc<std::collections::HashMap<String, usize>>,
}

fn knowledge_gallery_preview_text(content: &str) -> String {
    let source = content.chars().take(2048).collect::<String>();
    pulldown_cmark::Parser::new_ext(&source, pulldown_cmark::Options::ENABLE_TASKLISTS)
        .filter_map(|event| match event {
            pulldown_cmark::Event::Text(text) | pulldown_cmark::Event::Code(text) => {
                Some(text.into_string())
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(160)
        .collect()
}

impl<'a> KnowledgeRecordLookup<'a> {
    fn new(records: &'a [knowledge::PageRecord]) -> Self {
        let mut by_id = std::collections::HashMap::with_capacity(records.len());
        for (index, page) in records.iter().enumerate() {
            // Match the previous first-match lookup even for invalid duplicate IDs.
            by_id.entry(page.id.clone()).or_insert(index);
        }
        Self {
            records,
            by_id: Arc::new(by_id),
        }
    }

    fn get(&self, id: &str) -> Option<&'a knowledge::PageRecord> {
        self.by_id
            .get(id)
            .and_then(|index| self.records.get(*index))
    }

    fn filter_rows<'rows>(
        &self,
        rows: &'rows [String],
        query: &str,
    ) -> std::borrow::Cow<'rows, [String]> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return std::borrow::Cow::Borrowed(rows);
        }
        std::borrow::Cow::Owned(
            rows.iter()
                .filter(|id| {
                    self.get(id).is_some_and(|page| {
                        !page.encrypted
                            && !page.deleted
                            && page.title.to_lowercase().contains(&query)
                    })
                })
                .cloned()
                .collect(),
        )
    }
}

fn knowledge_field_label<'a>(db: &'a knowledge::KnowledgeDatabase, id: &str) -> &'a str {
    if id == "title" {
        "标题"
    } else {
        db.fields
            .iter()
            .find(|f| f.id == id)
            .map_or("已移除属性", |f| f.name.as_str())
    }
}

fn knowledge_column_width(view: &knowledge::DatabaseView, field: &knowledge::DatabaseField) -> f32 {
    view.column_widths
        .get(&field.id)
        .copied()
        .unwrap_or(match field.kind {
            knowledge::FieldKind::Date => 210.0,
            knowledge::FieldKind::Select
            | knowledge::FieldKind::Status
            | knowledge::FieldKind::Checkbox => 120.0,
            _ => 140.0,
        })
}

fn knowledge_cell_editor(
    ui: &mut egui::Ui,
    key: &str,
    field: &knowledge::DatabaseField,
    value: &knowledge::CellValue,
    records: &[knowledge::PageRecord],
    inputs: &mut BTreeMap<String, KnowledgeCellInput>,
    navigation: &mut DesktopNavigationState,
) -> Option<Result<knowledge::CellValue, String>> {
    use knowledge::{CellValue as C, FieldKind as F};
    if field.kind.computed() {
        ui.label(value.text());
        return None;
    }
    let mut next = value.clone();
    let mut changed = false;
    match field.kind {
        F::Checkbox => {
            let mut checked = matches!(value, C::Checkbox(true));
            if ui.checkbox(&mut checked, "").changed() {
                next = C::Checkbox(checked);
                changed = true;
            }
        }
        F::Select | F::Status => {
            egui::ComboBox::from_id_source(("cell", key))
                .selected_text(if value.empty() {
                    "选择".into()
                } else {
                    value.text()
                })
                .width(110.0)
                .show_ui(ui, |ui| {
                    if !field.required {
                        changed |= ui.selectable_value(&mut next, C::Empty, "清空").changed();
                    }
                    for option in &field.options {
                        changed |= ui
                            .selectable_value(&mut next, C::Select(option.clone()), option)
                            .changed();
                    }
                });
        }
        F::MultiSelect | F::Relation => {
            let selected = match value {
                C::MultiSelect(v) | C::Relation(v) => v.clone(),
                _ => Vec::new(),
            };
            let mut selected = selected;
            let label = if field.kind == F::Relation {
                selected
                    .iter()
                    .map(|id| {
                        records
                            .iter()
                            .find(|p| &p.id == id && !p.encrypted && !p.deleted)
                            .map_or("不可用页面", |p| p.title.as_str())
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                selected.join(", ")
            };
            ui.menu_button(if label.is_empty() { "选择" } else { &label }, |ui| {
                if field.kind == F::Relation {
                    for p in records.iter().filter(|p| {
                        !p.deleted
                            && !p.encrypted
                            && field
                                .relation_database_id
                                .as_ref()
                                .map_or(true, |db| p.meta.parent_id.as_ref() == Some(db))
                    }) {
                        let mut checked = selected.contains(&p.id);
                        if ui.checkbox(&mut checked, &p.title).changed() {
                            selected.retain(|id| id != &p.id);
                            if checked {
                                selected.push(p.id.clone());
                            }
                            changed = true;
                        }
                    }
                } else {
                    for option in &field.options {
                        let mut checked = selected.contains(option);
                        if ui.checkbox(&mut checked, option).changed() {
                            selected.retain(|s| s != option);
                            if checked {
                                selected.push(option.clone());
                            }
                            changed = true;
                        }
                    }
                }
            });
            next = if field.kind == F::Relation {
                C::Relation(selected)
            } else {
                C::MultiSelect(selected)
            };
        }
        _ => {
            let id = egui::Id::new(("property_input", key));
            let current = value.text();
            let input = inputs
                .entry(key.into())
                .or_insert_with(|| KnowledgeCellInput {
                    source: current.clone(),
                    text: current.clone(),
                });
            if input.source != current
                && !ui.memory(|m| m.has_focus(id))
                && (input.text == input.source
                    || knowledge::parse_cell(field, &input.text).as_ref() == Ok(value))
            {
                input.source = current.clone();
                input.text = current;
            }
            let text = &mut input.text;
            let response = ui.add(
                egui::TextEdit::singleline(text)
                    .id(egui::Id::new(("property_input", key)))
                    .desired_width(ui.available_width().min(230.0))
                    .hint_text(match field.kind {
                        F::Date => "YYYY-MM-DD 或 起日..止日",
                        F::Number => "数字",
                        F::Url => "https://",
                        _ => "",
                    }),
            );
            navigation.track_document_text_edit(&response);
            if response.lost_focus() && *text != value.text()
                || response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
            {
                return Some(knowledge::parse_cell(field, text));
            }
        }
    }
    changed.then(|| knowledge::validate_cell(field, &next).map(|_| next))
}

impl TimerWindowsClient {
    fn flush_knowledge_property_inputs(&mut self) -> io::Result<()> {
        let pending = self
            .desktop_ui
            .knowledge
            .cell_inputs
            .iter()
            .filter(|(key, input)| {
                (key.starts_with("page:") || key.starts_with("table:"))
                    && input.text != input.source
            })
            .map(|(key, input)| (key.clone(), input.text.clone()))
            .collect::<Vec<_>>();
        if pending.is_empty() {
            return Ok(());
        }
        self.draft_write_barrier()?;
        let prepared = (|| -> Result<_, String> {
            let mut changed = BTreeMap::<String, DesktopNote>::new();
            let mut accepted = Vec::new();
            for (key, text) in pending {
                // Match complete keys rather than splitting user supplied IDs.
                // Inputs belonging to deliberately removed fields are obsolete.
                let target = self.data.notes.iter().find_map(|note| {
                    if note.encryption.is_some()
                        || note.deleted_at_epoch_millis.is_some()
                        || (!key.starts_with(&format!("page:{}:", note.id))
                            && !key.starts_with(&format!("table:{}:", note.id)))
                    {
                        return None;
                    }
                    let meta = note.document.knowledge.as_ref()?;
                    let parent = self
                        .data
                        .notes
                        .iter()
                        .find(|parent| {
                            Some(&parent.id) == meta.parent_id.as_ref()
                                && parent.encryption.is_none()
                                && parent.deleted_at_epoch_millis.is_none()
                        })?
                        .document
                        .knowledge
                        .as_ref()?;
                    let field = parent.database.as_ref()?.fields.iter().find(|field| {
                        !field.deleted
                            && !field.kind.computed()
                            && (key == format!("page:{}:{}", note.id, field.id)
                                || key == format!("table:{}:{}", note.id, field.id))
                    })?;
                    Some((note, meta, parent, field))
                });
                let Some((note, meta, parent, field)) = target else {
                    continue;
                };
                let value = knowledge::parse_cell(field, &text)?;
                if meta
                    .properties
                    .get(&field.id)
                    .unwrap_or(&knowledge::CellValue::Empty)
                    != &value
                {
                    if meta.locked || parent.locked {
                        return Err("页面或数据库已锁定，属性修改尚未保存".into());
                    }
                    if !self.knowledge_relation_targets_are_current(field, &value) {
                        return Err("关联目标已变化，请重新选择".into());
                    }
                    changed
                        .entry(note.id.clone())
                        .or_insert_with(|| note.clone())
                        .document
                        .knowledge
                        .as_mut()
                        .unwrap()
                        .properties
                        .insert(field.id.clone(), value.clone());
                }
                accepted.push((key, value.text()));
            }
            let mut next = self.state_json.clone();
            let now = now_millis();
            let has_changes = !changed.is_empty();
            let selected_changed = changed.contains_key(&self.selected_note_id);
            for note in changed.values() {
                next = app_data::upsert_note_app_data_json(
                    &next,
                    &serde_json::to_string(note).map_err(|error| error.to_string())?,
                    now,
                )
                .ok_or_else(|| "属性修改未通过保存校验".to_string())?;
            }
            Ok((has_changes.then_some(next), accepted, selected_changed))
        })();
        let (next, accepted, selected_changed) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                self.status = format!("属性尚未保存：{error}");
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    self.status.clone(),
                ));
            }
        };
        if let Some(next) = next {
            if !self.replace_state(Some(next), "属性已保存") {
                return Err(io::Error::new(io::ErrorKind::Other, self.status.clone()));
            }
        }
        if selected_changed {
            self.desktop_ui.knowledge.page = self
                .data
                .notes
                .iter()
                .find(|note| note.id == self.selected_note_id)
                .and_then(|note| note.document.knowledge.clone());
        }
        for (key, text) in accepted {
            if let Some(input) = self.desktop_ui.knowledge.cell_inputs.get_mut(&key) {
                input.source = text.clone();
                input.text = text;
            }
        }
        Ok(())
    }

    fn knowledge_relation_targets_are_current(
        &self,
        field: &knowledge::DatabaseField,
        value: &knowledge::CellValue,
    ) -> bool {
        let knowledge::CellValue::Relation(ids) = value else {
            return true;
        };
        ids.iter().all(|id| {
            self.data.notes.iter().any(|note| {
                &note.id == id
                    && desktop_note_kind(note) == DesktopNoteKind::Document
                    && note.encryption.is_none()
                    && note.deleted_at_epoch_millis.is_none()
                    && field
                        .relation_database_id
                        .as_ref()
                        .map_or(true, |database| {
                            note.document
                                .knowledge
                                .as_ref()
                                .and_then(|page| page.parent_id.as_ref())
                                == Some(database)
                        })
            })
        })
    }

    fn set_knowledge_cell(
        &mut self,
        id: &str,
        field_id: &str,
        value: knowledge::CellValue,
    ) -> bool {
        if self.flush_note_body_draft().is_err() {
            return false;
        }
        let Some(mut note) = self
            .data
            .notes
            .iter()
            .find(|n| n.id == id && n.deleted_at_epoch_millis.is_none() && n.encryption.is_none())
            .cloned()
        else {
            return false;
        };
        let meta = note.document.knowledge.get_or_insert_with(Default::default);
        if meta.locked {
            self.status = "页面已锁定".into();
            return false;
        }
        let database = meta
            .parent_id
            .as_ref()
            .and_then(|id| {
                self.data.notes.iter().find(|p| {
                    &p.id == id && p.encryption.is_none() && p.deleted_at_epoch_millis.is_none()
                })
            })
            .and_then(|n| n.document.knowledge.as_ref());
        if database.is_some_and(|p| p.locked) {
            self.status = "数据库已锁定".into();
            return false;
        }
        let Some(db) = database.and_then(|p| p.database.as_ref()) else {
            return false;
        };
        let Some(field) = db.fields.iter().find(|f| f.id == field_id && !f.deleted) else {
            return false;
        };
        if let Err(error) = knowledge::validate_cell(field, &value) {
            self.status = error;
            return false;
        }
        if !self.knowledge_relation_targets_are_current(field, &value) {
            self.status = "关联目标已变化，请重新选择".into();
            return false;
        }
        meta.properties.insert(field_id.into(), value);
        self.replace_state(
            app_data::upsert_note_app_data_json(
                &self.state_json,
                &serde_json::to_string(&note).unwrap_or_default(),
                now_millis(),
            ),
            "属性已保存",
        )
    }

    fn ui_knowledge_record_properties(&mut self, ui: &mut egui::Ui) {
        let Some(parent) = self
            .desktop_ui
            .knowledge
            .page
            .as_ref()
            .and_then(|p| p.parent_id.clone())
        else {
            return;
        };
        let Some((db, database_locked)) = self
            .data
            .notes
            .iter()
            .find(|n| {
                n.id == parent && n.encryption.is_none() && n.deleted_at_epoch_millis.is_none()
            })
            .and_then(|n| n.document.knowledge.as_ref())
            .and_then(|meta| meta.database.as_ref().map(|db| (db.clone(), meta.locked)))
        else {
            return;
        };
        let mut meta = self.desktop_ui.knowledge.page.clone().unwrap_or_default();
        let previous = meta.clone();
        egui::CollapsingHeader::new("数据库属性")
            .id_source(("record_properties", &self.selected_note_id))
            .default_open(true)
            .show(ui, |ui| {
                // Ordinary input fields need neither the workspace record copy
                // nor the query index; collapsed properties need neither either.
                let records = db
                    .fields
                    .iter()
                    .any(|field| {
                        !field.deleted
                            && (field.kind.computed()
                                || field.kind == knowledge::FieldKind::Relation)
                    })
                    .then(|| self.knowledge_records());
                let records = records.as_deref().map(Vec::as_slice).unwrap_or_default();
                let mut engine = None;
                egui::Grid::new("record_properties_grid")
                    .num_columns(2)
                    .spacing([20.0, 8.0])
                    .show(ui, |ui| {
                        for field in db.fields.iter().filter(|f| !f.deleted) {
                            ui.label(format!(
                                "{}{}",
                                field.name,
                                if field.required { " *" } else { "" }
                            ));
                            if field.kind.computed() {
                                match engine
                                    .get_or_insert_with(|| {
                                        knowledge::QueryEngine::new(records, knowledge_today())
                                    })
                                    .value(&self.selected_note_id, &field.id)
                                {
                                    Ok(v) => {
                                        ui.label(v.text());
                                    }
                                    Err(error) => {
                                        ui.colored_label(palette().danger, error);
                                    }
                                }
                            } else {
                                ui.add_enabled_ui(!meta.locked && !database_locked, |ui| {
                                    let value =
                                        meta.properties.get(&field.id).cloned().unwrap_or_default();
                                    if let Some(result) = knowledge_cell_editor(
                                        ui,
                                        &format!("page:{}:{}", self.selected_note_id, field.id),
                                        field,
                                        &value,
                                        records,
                                        &mut self.desktop_ui.knowledge.cell_inputs,
                                        &mut self.desktop_ui.navigation,
                                    ) {
                                        match result {
                                            Ok(value) => {
                                                meta.properties.insert(field.id.clone(), value);
                                            }
                                            Err(error) => self.desktop_ui.knowledge.error = error,
                                        }
                                    }
                                });
                            }
                            ui.end_row();
                        }
                    });
            });
        if meta != previous {
            self.update_knowledge_page(meta);
        }
    }

    fn refresh_knowledge_query(
        &mut self,
        db: &knowledge::KnowledgeDatabase,
        view: &knowledge::DatabaseView,
        records: &[knowledge::PageRecord],
    ) {
        let today = knowledge_today();
        let meta = &self.desktop_ui.knowledge.page;
        if self
            .desktop_ui
            .knowledge
            .query_key
            .as_ref()
            .is_some_and(|key| {
                key.version == self.notes_cache_version()
                    && key.page_id == self.selected_note_id
                    && key.today == today
                    && &key.database == db
                    && &key.view == view
                    && &key.meta == meta
            })
        {
            return;
        }
        let key = KnowledgeQueryKey {
            version: self.notes_cache_version(),
            page_id: self.selected_note_id.clone(),
            today,
            database: db.clone(),
            view: view.clone(),
            meta: meta.clone(),
        };
        // Only a changed query needs the draft overlay. Warm frames borrow the shared records.
        let mut draft_records = std::borrow::Cow::Borrowed(records);
        if let Some(meta) = meta.as_ref() {
            if let Some(index) = records
                .iter()
                .position(|p| p.id == self.selected_note_id && &p.meta != meta)
            {
                draft_records.to_mut()[index].meta = meta.clone();
            }
        }
        let mut engine = knowledge::QueryEngine::new(&draft_records, today);
        let rows = engine.rows(&self.selected_note_id, view);
        let mut cells = BTreeMap::new();
        for id in &rows {
            let mut row = BTreeMap::new();
            row.insert("title".into(), engine.value(id, "title"));
            for field in db.fields.iter().filter(|f| !f.deleted) {
                row.insert(field.id.clone(), engine.value(id, &field.id));
            }
            cells.insert(id.clone(), row);
        }
        self.desktop_ui.knowledge.query_key = Some(key);
        self.desktop_ui.knowledge.query_rows = Arc::new(rows);
        self.desktop_ui.knowledge.query_cells = cells;
    }

    fn ui_knowledge_database(&mut self, ui: &mut egui::Ui) {
        let Some(mut meta) = self.desktop_ui.knowledge.page.clone() else {
            return;
        };
        let Some(mut db) = meta.database.clone() else {
            return;
        };
        let previous = db.clone();
        let active = self.desktop_ui.knowledge.active_view.clone();
        let index = db
            .views
            .iter()
            .position(|v| v.id == active)
            .or_else(|| db.views.iter().position(|v| v.id == db.default_view_id))
            .unwrap_or(0);
        egui::ScrollArea::horizontal()
            .id_source("knowledge_view_tabs")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for view in &db.views {
                        let selected = view.id == db.views[index].id;
                        let tab = knowledge_quiet_button(ui, &view.name, view.kind.label());
                        if selected {
                            ui.painter().hline(
                                tab.rect.x_range(),
                                tab.rect.bottom() + 2.0,
                                egui::Stroke::new(2.0, palette().accent),
                            );
                        }
                        if tab.clicked() {
                            self.desktop_ui.knowledge.active_view = view.id.clone();
                        }
                    }
                    ui.menu_button("＋ 视图", |ui| {
                        for kind in knowledge::ViewKind::ALL {
                            if ui
                                .add_enabled(!meta.locked, egui::Button::new(kind.label()))
                                .clicked()
                            {
                                let mut view = db.views[index].clone();
                                view.id = random_desktop_identifier("view");
                                view.name = kind.label().into();
                                view.kind = kind;
                                self.desktop_ui.knowledge.active_view = view.id.clone();
                                db.views.push(view);
                                ui.close_menu();
                            }
                        }
                    });
                    if ui.button("视图设置").clicked() {
                        self.desktop_ui.knowledge.options_open =
                            !self.desktop_ui.knowledge.options_open;
                    }
                });
            });
        let index = db
            .views
            .iter()
            .position(|v| v.id == self.desktop_ui.knowledge.active_view)
            .unwrap_or(index);
        if self.desktop_ui.knowledge.options_open {
            let mut open = true;
            let enabled = ui.is_enabled() && !meta.locked;
            egui::Window::new("视图设置")
                .id(egui::Id::new("knowledge_view_options"))
                .open(&mut open)
                .default_width(520.0)
                .default_height(460.0)
                .collapsible(false)
                .show(ui.ctx(), |ui| {
                    ui.set_enabled(enabled);
                    egui::ScrollArea::vertical()
                        .max_height(520.0)
                        .show(ui, |ui| {
                            self.ui_knowledge_database_options(ui, &mut db, index)
                        });
                });
            if !open {
                self.desktop_ui.knowledge.options_open = false;
            }
        }
        let view = db.views[index.min(db.views.len() - 1)].clone();
        if db != previous {
            meta.database = Some(db.clone());
            self.update_knowledge_page(meta.clone());
        }
        let records = self.knowledge_records();
        self.refresh_knowledge_query(&db, &view, &records);
        let all_rows = self.desktop_ui.knowledge.query_rows.clone();
        let record_lookup = self.knowledge_record_lookup(&records);
        let mut rows = Arc::clone(&all_rows);
        ui.horizontal(|ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.desktop_ui.experience.record_query)
                    .id_source("knowledge_record_query")
                    .desired_width((ui.available_width() * 0.4).clamp(110.0, 240.0))
                    .hint_text("查找记录"),
            );
            self.desktop_ui
                .navigation
                .track_document_text_edit(&response);
            if !self.desktop_ui.experience.record_query.is_empty()
                && knowledge_quiet_button(ui, "×", "清除查找").clicked()
            {
                self.desktop_ui.experience.record_query.clear();
            }
            rows = self.cached_knowledge_database_rows(
                &all_rows,
                &records,
                &record_lookup,
                &self.desktop_ui.experience.record_query,
            );
            ui.label(
                egui::RichText::new(format!("{} / {} 条", rows.len(), all_rows.len()))
                    .small()
                    .color(palette().muted),
            );
            if view.kind != knowledge::ViewKind::Form
                && ui
                    .add_enabled(!meta.locked, egui::Button::new("＋ 新记录"))
                    .clicked()
            {
                self.desktop_ui.knowledge.active_view = db
                    .views
                    .iter()
                    .find(|v| v.kind == knowledge::ViewKind::Form)
                    .map(|v| v.id.clone())
                    .unwrap_or_default();
                if self.desktop_ui.knowledge.active_view.is_empty() {
                    let mut next = meta.clone();
                    let db = next.database.as_mut().unwrap();
                    let form = knowledge::DatabaseView {
                        id: random_desktop_identifier("view"),
                        name: "新记录".into(),
                        kind: knowledge::ViewKind::Form,
                        visible_fields: db
                            .fields
                            .iter()
                            .filter(|f| !f.deleted)
                            .map(|f| f.id.clone())
                            .collect(),
                        ..Default::default()
                    };
                    self.desktop_ui.knowledge.active_view = form.id.clone();
                    db.views.push(form);
                    self.update_knowledge_page(next);
                }
            }
        });
        ui.add_space(8.0);
        match view.kind {
            knowledge::ViewKind::Table => {
                self.ui_knowledge_table(ui, &db, &view, &rows, &record_lookup, meta.locked)
            }
            knowledge::ViewKind::Board => {
                self.ui_knowledge_board(ui, &db, &view, &rows, &record_lookup, meta.locked)
            }
            knowledge::ViewKind::List | knowledge::ViewKind::Gallery => {
                self.ui_knowledge_cards(ui, &db, &view, &rows, &record_lookup)
            }
            knowledge::ViewKind::Calendar => {
                self.ui_knowledge_calendar(ui, &db, &view, &rows, &record_lookup, meta.locked)
            }
            knowledge::ViewKind::Timeline => {
                self.ui_knowledge_timeline(ui, &db, &view, &rows, &record_lookup)
            }
            knowledge::ViewKind::Form => self.ui_knowledge_form(ui, &db, &records, meta.locked),
        }
        if !self.desktop_ui.knowledge.error.is_empty() {
            ui.colored_label(palette().danger, &self.desktop_ui.knowledge.error);
            if ui.small_button("关闭提示").clicked() {
                self.desktop_ui.knowledge.error.clear();
            }
        }
    }

    fn ui_knowledge_database_options(
        &mut self,
        ui: &mut egui::Ui,
        db: &mut knowledge::KnowledgeDatabase,
        index: usize,
    ) {
        let snapshot = db.clone();
        let view = &mut db.views[index];
        egui::CollapsingHeader::new("视图与列")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label("名称");
                    let response =
                        ui.add(egui::TextEdit::singleline(&mut view.name).desired_width(140.0));
                    self.desktop_ui
                        .navigation
                        .track_document_text_edit(&response);
                    egui::ComboBox::from_id_source("database_view_kind")
                        .selected_text(view.kind.label())
                        .show_ui(ui, |ui| {
                            for kind in knowledge::ViewKind::ALL {
                                ui.selectable_value(&mut view.kind, kind, kind.label());
                            }
                        });
                    if ui.button("设为默认").clicked() {
                        db.default_view_id = view.id.clone();
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("分组");
                    egui::ComboBox::from_id_source("db_group_by")
                        .selected_text(knowledge_field_label(&snapshot, &view.group_by))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut view.group_by, String::new(), "不分组");
                            for f in snapshot
                                .fields
                                .iter()
                                .filter(|f| !f.deleted && !f.kind.computed())
                            {
                                ui.selectable_value(&mut view.group_by, f.id.clone(), &f.name);
                            }
                        });
                    ui.label("日期");
                    egui::ComboBox::from_id_source("db_date_field")
                        .selected_text(knowledge_field_label(&snapshot, &view.date_field))
                        .show_ui(ui, |ui| {
                            for f in snapshot
                                .fields
                                .iter()
                                .filter(|f| !f.deleted && f.kind == knowledge::FieldKind::Date)
                            {
                                ui.selectable_value(&mut view.date_field, f.id.clone(), &f.name);
                            }
                        });
                });
                for f in snapshot.fields.iter().filter(|f| !f.deleted) {
                    ui.horizontal(|ui| {
                        let mut visible = view.visible_fields.contains(&f.id);
                        if ui.checkbox(&mut visible, &f.name).changed() {
                            view.visible_fields.retain(|id| id != &f.id);
                            if visible {
                                view.visible_fields.push(f.id.clone());
                            }
                        }
                        if visible {
                            let mut width = knowledge_column_width(view, f);
                            if ui
                                .add(
                                    egui::DragValue::new(&mut width)
                                        .clamp_range(70.0..=600.0)
                                        .suffix(" px"),
                                )
                                .changed()
                            {
                                view.column_widths.insert(f.id.clone(), width);
                            }
                            let mut aggregate = view.aggregates.get(&f.id).copied();
                            egui::ComboBox::from_id_source(("aggregate", &view.id, &f.id))
                                .selected_text(aggregate.map_or("无统计", |a| a.label()))
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut aggregate, None, "无统计");
                                    for kind in knowledge::AggregateKind::ALL {
                                        ui.selectable_value(
                                            &mut aggregate,
                                            Some(kind),
                                            kind.label(),
                                        );
                                    }
                                });
                            if let Some(a) = aggregate {
                                view.aggregates.insert(f.id.clone(), a);
                            } else {
                                view.aggregates.remove(&f.id);
                            }
                        }
                        if ui.small_button("编辑属性").clicked() {
                            self.desktop_ui.knowledge.field_editor = Some(f.clone());
                        }
                    });
                }
            });
        egui::CollapsingHeader::new("筛选与排序").show(ui, |ui| {
            let mut filter = view.filter.clone().unwrap_or(knowledge::Filter::All {
                filters: Vec::new(),
            });
            let original_filter = filter.clone();
            knowledge_filter_editor(
                ui,
                &snapshot,
                &mut filter,
                0,
                "filter",
                &mut self.desktop_ui.navigation,
            );
            if filter != original_filter {
                view.filter = Some(filter);
            }
            let mut remove = None;
            for (i, sort) in view.sorts.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_source(("sort", i))
                        .selected_text(knowledge_field_label(&snapshot, &sort.field))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut sort.field, "title".into(), "标题");
                            for f in snapshot.fields.iter().filter(|f| !f.deleted) {
                                ui.selectable_value(&mut sort.field, f.id.clone(), &f.name);
                            }
                        });
                    ui.checkbox(&mut sort.descending, "降序");
                    if ui.small_button("删除").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                view.sorts.remove(i);
            }
            if view.sorts.len() < 16 && ui.small_button("＋ 排序").clicked() {
                view.sorts.push(knowledge::SortRule {
                    field: "title".into(),
                    descending: false,
                });
            }
        });
        if ui.button("＋ 新属性").clicked() {
            self.desktop_ui.knowledge.field_editor = Some(knowledge::DatabaseField {
                id: random_desktop_identifier("property"),
                name: "新属性".into(),
                ..Default::default()
            });
        }
        if let Some(mut field) = self.desktop_ui.knowledge.field_editor.clone() {
            ui.separator();
            ui.label("属性设置");
            ui.horizontal_wrapped(|ui| {
                let response =
                    ui.add(egui::TextEdit::singleline(&mut field.name).desired_width(160.0));
                self.desktop_ui
                    .navigation
                    .track_document_text_edit(&response);
                egui::ComboBox::from_id_source("new_field_kind")
                    .selected_text(field.kind.label())
                    .show_ui(ui, |ui| {
                        for kind in knowledge::FieldKind::ALL {
                            ui.selectable_value(&mut field.kind, kind, kind.label());
                        }
                    });
                if !field.kind.computed() {
                    ui.checkbox(&mut field.required, "必填");
                }
            });
            match field.kind {
                knowledge::FieldKind::Select
                | knowledge::FieldKind::MultiSelect
                | knowledge::FieldKind::Status => {
                    let mut text = field.options.join("\n");
                    ui.label("选项，每行一个");
                    let response = ui.text_edit_multiline(&mut text);
                    self.desktop_ui
                        .navigation
                        .track_document_text_edit(&response);
                    if response.changed() {
                        field.options = text
                            .lines()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                            .collect();
                    }
                }
                knowledge::FieldKind::Formula => {
                    ui.label("使用 [字段名] 引用属性，例如 if([成本] > 0, [收入] / [成本], 0)");
                    let response = ui.add(
                        egui::TextEdit::multiline(&mut field.formula)
                            .code_editor()
                            .desired_rows(3)
                            .desired_width(f32::INFINITY),
                    );
                    self.desktop_ui
                        .navigation
                        .track_document_text_edit(&response);
                }
                knowledge::FieldKind::Relation => {
                    let records = self.knowledge_records();
                    egui::ComboBox::from_id_source("relation_target")
                        .selected_text(
                            field
                                .relation_database_id
                                .as_ref()
                                .and_then(|id| records.iter().find(|p| &p.id == id))
                                .map_or("任意页面", |p| p.title.as_str()),
                        )
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut field.relation_database_id, None, "任意页面");
                            for p in records
                                .iter()
                                .filter(|p| !p.deleted && !p.encrypted && p.meta.database.is_some())
                            {
                                ui.selectable_value(
                                    &mut field.relation_database_id,
                                    Some(p.id.clone()),
                                    &p.title,
                                );
                            }
                        });
                }
                knowledge::FieldKind::Rollup => {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("关联属性");
                        egui::ComboBox::from_id_source("rollup_relation")
                            .selected_text(knowledge_field_label(
                                &snapshot,
                                &field.rollup_relation_field,
                            ))
                            .show_ui(ui, |ui| {
                                for f in snapshot.fields.iter().filter(|f| {
                                    !f.deleted && f.kind == knowledge::FieldKind::Relation
                                }) {
                                    ui.selectable_value(
                                        &mut field.rollup_relation_field,
                                        f.id.clone(),
                                        &f.name,
                                    );
                                }
                            });
                        ui.label("目标属性名或 ID");
                        let response = ui.text_edit_singleline(&mut field.rollup_target_field);
                        self.desktop_ui
                            .navigation
                            .track_document_text_edit(&response);
                        egui::ComboBox::from_id_source("rollup_aggregate")
                            .selected_text(field.aggregate.label())
                            .show_ui(ui, |ui| {
                                for a in knowledge::AggregateKind::ALL {
                                    ui.selectable_value(&mut field.aggregate, a, a.label());
                                }
                            });
                    });
                }
                _ => {}
            }
            let mut finish = false;
            ui.horizontal(|ui| {
                if ui.button("保存属性").clicked() {
                    let mut candidate = db.clone();
                    if let Some(f) = candidate.fields.iter_mut().find(|f| f.id == field.id) {
                        *f = field.clone();
                    } else {
                        candidate.fields.push(field.clone());
                        for view in &mut candidate.views {
                            view.visible_fields.push(field.id.clone());
                        }
                    }
                    let validation = candidate.validate().and_then(|_| {
                        for p in self.knowledge_records().iter().filter(|p| {
                            p.meta.parent_id.as_deref() == Some(&self.selected_note_id)
                                && !p.deleted
                                && !p.encrypted
                        }) {
                            knowledge::validate_record(&candidate, &p.meta.properties)?;
                        }
                        Ok(())
                    });
                    match validation {
                        Ok(()) => {
                            *db = candidate;
                            finish = true;
                        }
                        Err(error) => {
                            self.desktop_ui.knowledge.error = format!("属性未修改：{error}")
                        }
                    }
                }
                if ui.button("取消").clicked() {
                    finish = true;
                }
                if db.fields.iter().any(|f| f.id == field.id) && ui.button("隐藏属性").clicked()
                {
                    if let Some(f) = db.fields.iter_mut().find(|f| f.id == field.id) {
                        f.deleted = true;
                    }
                    for v in &mut db.views {
                        v.visible_fields.retain(|id| id != &field.id);
                    }
                    finish = true;
                }
            });
            self.desktop_ui.knowledge.field_editor = if finish { None } else { Some(field) };
        }
        if db.fields.iter().any(|f| f.deleted) {
            ui.menu_button("恢复隐藏属性", |ui| {
                for f in db.fields.iter_mut().filter(|f| f.deleted) {
                    if ui.button(&f.name).clicked() {
                        f.deleted = false;
                        ui.close_menu();
                    }
                }
            });
        }
    }

    fn knowledge_cached_value(
        &self,
        id: &str,
        field: &str,
    ) -> Result<knowledge::CellValue, String> {
        self.desktop_ui
            .knowledge
            .query_cells
            .get(id)
            .and_then(|row| row.get(field))
            .cloned()
            .unwrap_or(Ok(knowledge::CellValue::Empty))
    }

    fn ui_knowledge_table(
        &mut self,
        ui: &mut egui::Ui,
        db: &knowledge::KnowledgeDatabase,
        view: &knowledge::DatabaseView,
        rows: &[String],
        records: &KnowledgeRecordLookup<'_>,
        locked: bool,
    ) {
        let fields = view
            .visible_fields
            .iter()
            .filter_map(|id| db.fields.iter().find(|f| &f.id == id && !f.deleted))
            .collect::<Vec<_>>();
        let title_width = if ui.available_width() < 680.0 {
            150.0
        } else {
            180.0
        };
        let mut updates = Vec::new();
        let mut open = None;
        let table_width = title_width
            + fields
                .iter()
                .map(|field| knowledge_column_width(view, field))
                .sum::<f32>()
            + fields.len() as f32 * 16.0;
        // Include the header and aggregate footer in the virtual row count.
        let total_rows = rows.len() + 2;
        // A focused editor stays mounted until it reports lost_focus, so scrolling
        // cannot discard a draft before the cell commits it.
        let editing = ui.memory(|memory| {
            self.desktop_ui
                .knowledge
                .cell_inputs
                .keys()
                .any(|key| memory.has_focus(egui::Id::new(("property_input", key))))
        });
        let mut draw_rows = |ui: &mut egui::Ui, visible: std::ops::Range<usize>| {
            ui.set_min_width(table_width);
            egui::Grid::new(("knowledge_database_table", &view.id))
                .striped(true)
                .min_row_height(36.0)
                .spacing([16.0, 4.0])
                .show(ui, |ui| {
                    if visible.contains(&0) {
                        ui.add_sized(
                            [title_width, 28.0],
                            egui::Label::new(egui::RichText::new("标题").strong()),
                        );
                        for f in &fields {
                            ui.add_sized(
                                [knowledge_column_width(view, f), 28.0],
                                egui::Label::new(egui::RichText::new(&f.name).strong()),
                            );
                        }
                        ui.end_row();
                    }
                    for row_index in visible.clone() {
                        if row_index == 0 || row_index > rows.len() {
                            continue;
                        }
                        let id = &rows[row_index - 1];
                        let page = records.get(id);
                        ui.vertical(|ui| {
                            ui.set_width(title_width);
                            if ui
                                .link(page.map_or("未命名", |p| {
                                    if p.title.is_empty() {
                                        "未命名"
                                    } else {
                                        &p.title
                                    }
                                }))
                                .clicked()
                            {
                                open = Some(id.clone());
                            }
                        });
                        for f in &fields {
                            ui.vertical(|ui| {
                                ui.set_width(knowledge_column_width(view, f));
                                match self.knowledge_cached_value(id, &f.id) {
                                    Ok(value) => {
                                        ui.add_enabled_ui(
                                            !locked && !page.is_some_and(|p| p.meta.locked),
                                            |ui| {
                                                if let Some(result) = knowledge_cell_editor(
                                                    ui,
                                                    &format!("table:{id}:{}", f.id),
                                                    f,
                                                    &value,
                                                    records.records,
                                                    &mut self.desktop_ui.knowledge.cell_inputs,
                                                    &mut self.desktop_ui.navigation,
                                                ) {
                                                    updates.push((
                                                        id.clone(),
                                                        f.id.clone(),
                                                        result,
                                                    ));
                                                }
                                            },
                                        );
                                    }
                                    Err(error) => {
                                        ui.colored_label(palette().danger, "计算错误")
                                            .on_hover_text(error);
                                    }
                                }
                            });
                        }
                        ui.end_row();
                    }
                    if visible.contains(&(rows.len() + 1)) {
                        ui.label(format!("{} 条", rows.len()));
                        for f in &fields {
                            if let Some(kind) = view.aggregates.get(&f.id) {
                                let values = rows
                                    .iter()
                                    .filter_map(|id| self.knowledge_cached_value(id, &f.id).ok())
                                    .collect::<Vec<_>>();
                                ui.label(format!(
                                    "{} {}",
                                    kind.label(),
                                    knowledge::aggregate(&values, *kind).text()
                                ));
                            } else {
                                ui.label("");
                            }
                        }
                        ui.end_row();
                    }
                });
        };
        let scroll = egui::ScrollArea::both()
            .id_source("knowledge_database_table_scroll")
            .max_height(560.0);
        if editing {
            scroll.show(ui, |ui| draw_rows(ui, 0..total_rows));
        } else {
            // Grid rows are at least 36 px high with 4 px vertical spacing.
            // show_rows adds the surrounding Ui spacing itself.
            let row_height = (40.0 - ui.spacing().item_spacing.y).max(1.0);
            scroll.show_rows(ui, row_height, total_rows, |ui, visible| {
                draw_rows(ui, visible)
            });
        }
        for (id, field, result) in updates {
            match result {
                Ok(value) => {
                    if !self.set_knowledge_cell(&id, &field, value) {
                        self.desktop_ui.knowledge.error = self.status.clone();
                    }
                }
                Err(error) => self.desktop_ui.knowledge.error = error,
            }
        }
        if let Some(id) = open {
            self.select_note_by_id(&id);
        }
    }

    fn ui_knowledge_board(
        &mut self,
        ui: &mut egui::Ui,
        db: &knowledge::KnowledgeDatabase,
        view: &knowledge::DatabaseView,
        rows: &[String],
        records: &KnowledgeRecordLookup<'_>,
        locked: bool,
    ) {
        let field = db
            .fields
            .iter()
            .find(|f| f.id == view.group_by && !f.deleted);
        let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
        if let Some(f) = field {
            for option in &f.options {
                groups.insert(option.clone(), Vec::new());
            }
        }
        for id in rows {
            let value = self
                .knowledge_cached_value(id, &view.group_by)
                .unwrap_or_default();
            groups
                .entry(if value.empty() {
                    "未分组".into()
                } else {
                    value.text()
                })
                .or_default()
                .push(id.clone());
        }
        let mut groups = groups.into_iter().collect::<Vec<_>>();
        groups.sort_by_key(|(label, _)| {
            field
                .and_then(|f| f.options.iter().position(|o| o == label))
                .unwrap_or(usize::MAX)
        });
        let mut moved = None;
        let count = groups.len().max(1) as f32;
        let column_width = ((ui.available_width() - (count - 1.0) * ui.spacing().item_spacing.x)
            / count)
            .clamp(190.0, 280.0);
        egui::ScrollArea::horizontal()
            .id_source("knowledge_board_scroll")
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    for (label, ids) in groups {
                        let response = ui
                            .vertical(|ui| {
                                ui.set_width(column_width);
                                ui.label(
                                    egui::RichText::new(format!("{label}  {}", ids.len())).strong(),
                                );
                                egui::Frame::none()
                                    .fill(palette().panel_alt)
                                    .rounding(7.0)
                                    .inner_margin(8.0)
                                    .show(ui, |ui| {
                                        ui.set_min_height(150.0);
                                        for id in ids {
                                            if let Some(p) = records.get(&id) {
                                                let response = card_frame()
                                                    .inner_margin(10.0)
                                                    .show(ui, |ui| {
                                                        ui.set_width(
                                                            (column_width - 36.0).max(100.0),
                                                        );
                                                        if ui
                                                            .link(if p.title.is_empty() {
                                                                "未命名"
                                                            } else {
                                                                &p.title
                                                            })
                                                            .clicked()
                                                        {
                                                            self.select_note_by_id(&p.id);
                                                        }
                                                        for f in view
                                                            .visible_fields
                                                            .iter()
                                                            .filter(|id| *id != &view.group_by)
                                                            .take(3)
                                                        {
                                                            if let Ok(value) = self
                                                                .knowledge_cached_value(&p.id, f)
                                                            {
                                                                if !value.empty() {
                                                                    ui.label(
                                                                        egui::RichText::new(
                                                                            value.text(),
                                                                        )
                                                                        .small()
                                                                        .color(palette().muted),
                                                                    );
                                                                }
                                                            }
                                                        }
                                                    })
                                                    .response
                                                    .interact(egui::Sense::drag());
                                                if !locked && !p.meta.locked {
                                                    response.dnd_set_drag_payload(
                                                        KnowledgePageDrag(p.id.clone()),
                                                    );
                                                }
                                                ui.add_space(6.0);
                                            }
                                        }
                                    });
                            })
                            .response;
                        if !locked {
                            if let Some(payload) =
                                response.dnd_release_payload::<KnowledgePageDrag>()
                            {
                                if rows.contains(&payload.0) {
                                    moved = Some((payload.0.clone(), label));
                                }
                            }
                        }
                    }
                })
            });
        if let (Some((id, label)), Some(field)) = (moved, field) {
            let value = if label == "未分组" {
                Ok(knowledge::CellValue::Empty)
            } else {
                knowledge::parse_cell(field, &label)
            };
            match value {
                Ok(value) => {
                    self.set_knowledge_cell(&id, &field.id, value);
                }
                Err(error) => self.desktop_ui.knowledge.error = error,
            }
        }
    }

    fn ui_knowledge_cards(
        &mut self,
        ui: &mut egui::Ui,
        db: &knowledge::KnowledgeDatabase,
        view: &knowledge::DatabaseView,
        rows: &[String],
        records: &KnowledgeRecordLookup<'_>,
    ) {
        let columns = if view.kind == knowledge::ViewKind::Gallery {
            (ui.available_width() / 240.0).floor().clamp(1.0, 4.0) as usize
        } else {
            1
        };
        for chunk in rows.chunks(columns) {
            ui.columns(columns, |uis| {
                for (i, id) in chunk.iter().enumerate() {
                    let ui = &mut uis[i];
                    if let Some(p) = records.get(id) {
                        card_frame().show(ui, |ui| {
                            if view.kind == knowledge::ViewKind::Gallery {
                                let text = self.cached_knowledge_gallery_preview(p);
                                ui.add_sized(
                                    [ui.available_width(), 80.0],
                                    egui::Label::new(
                                        egui::RichText::new(text.as_str()).color(palette().muted),
                                    )
                                    .wrap(true),
                                );
                                ui.separator();
                            }
                            if ui
                                .link(format!(
                                    "{} {}",
                                    p.meta.icon,
                                    if p.title.is_empty() {
                                        "未命名"
                                    } else {
                                        &p.title
                                    }
                                ))
                                .clicked()
                            {
                                self.select_note_by_id(&p.id);
                            }
                            for f in &view.visible_fields {
                                if let Ok(value) = self.knowledge_cached_value(id, f) {
                                    if !value.empty() {
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "{}  {}",
                                                knowledge_field_label(db, f),
                                                value.text()
                                            ))
                                            .small(),
                                        );
                                    }
                                }
                            }
                        });
                    }
                }
            });
            ui.add_space(8.0);
        }
    }

    fn ui_knowledge_calendar(
        &mut self,
        ui: &mut egui::Ui,
        db: &knowledge::KnowledgeDatabase,
        view: &knowledge::DatabaseView,
        rows: &[String],
        records: &KnowledgeRecordLookup<'_>,
        locked: bool,
    ) {
        let base = knowledge::day_date(knowledge_today());
        let year = base[..4].parse::<i64>().unwrap_or(2026);
        let month = base[5..7].parse::<i64>().unwrap_or(1);
        let shifted = (year * 12 + month - 1 + self.desktop_ui.knowledge.calendar_offset)
            .clamp(12, 9999 * 12 + 11);
        self.desktop_ui.knowledge.calendar_offset = shifted - (year * 12 + month - 1);
        let ym = format!(
            "{:04}-{:02}",
            shifted.div_euclid(12),
            shifted.rem_euclid(12) + 1
        );
        let Some(first) = knowledge::date_day(&format!("{ym}-01")) else {
            return;
        };
        ui.horizontal(|ui| {
            if ui.small_button("上月").clicked() {
                self.desktop_ui.knowledge.calendar_offset -= 1;
            }
            ui.strong(&ym);
            if ui.small_button("下月").clicked() {
                self.desktop_ui.knowledge.calendar_offset += 1;
            }
            if ui.small_button("今天").clicked() {
                self.desktop_ui.knowledge.calendar_offset = 0;
            }
            ui.label(format!(
                "按 {}",
                knowledge_field_label(db, &view.date_field)
            ));
        });
        let start = first - (first + 3).rem_euclid(7);
        let mut moved = None;
        let width = ui.available_width().max(560.0);
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
            egui::ScrollArea::horizontal()
                .id_source("calendar_scroll")
                .show(ui, |ui| {
                    ui.set_width(width);
                    ui.columns(7, |days| {
                        for (day, name) in days
                            .iter_mut()
                            .zip(["一", "二", "三", "四", "五", "六", "日"])
                        {
                            day.add_sized([day.available_width(), 20.0], egui::Label::new(name));
                        }
                    });
                    for week in 0..6 {
                        ui.columns(7, |days| {
                            for (dow, ui) in days.iter_mut().enumerate() {
                                let day = start + week * 7 + dow as i64;
                                let date = knowledge::day_date(day);
                                let cell_width = (ui.available_width() - 10.0).max(40.0);
                                let response = egui::Frame::none()
                                    .fill(if day == knowledge_today() {
                                        palette().accent_soft
                                    } else {
                                        palette().panel_alt
                                    })
                                    .inner_margin(5.0)
                                    .rounding(5.0)
                                    .show(ui, |ui| {
                                        ui.set_width(cell_width);
                                        ui.set_min_height(74.0);
                                        ui.label(egui::RichText::new(&date[8..]).color(
                                            if date.starts_with(&ym) {
                                                palette().text
                                            } else {
                                                palette().muted
                                            },
                                        ));
                                        for id in rows {
                                            if let Ok(knowledge::CellValue::Date(range)) =
                                                self.knowledge_cached_value(id, &view.date_field)
                                            {
                                                let end = if range.end.is_empty() {
                                                    &range.start
                                                } else {
                                                    &range.end
                                                };
                                                if range.start.as_str() <= date.as_str()
                                                    && end.as_str() >= date.as_str()
                                                {
                                                    if let Some(p) = records.get(id) {
                                                        let r = ui.add(
                                                            egui::Label::new(
                                                                egui::RichText::new(&p.title)
                                                                    .small(),
                                                            )
                                                            .wrap(true)
                                                            .sense(egui::Sense::click_and_drag()),
                                                        );
                                                        if r.clicked() {
                                                            self.select_note_by_id(id);
                                                        }
                                                        if !locked && !p.meta.locked {
                                                            r.dnd_set_drag_payload(
                                                                KnowledgePageDrag(id.clone()),
                                                            );
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    })
                                    .response;
                                if !locked {
                                    if let Some(payload) =
                                        response.dnd_release_payload::<KnowledgePageDrag>()
                                    {
                                        if rows.contains(&payload.0) {
                                            moved = Some((payload.0.clone(), date));
                                        }
                                    }
                                }
                            }
                        });
                    }
                });
        });
        if let Some((id, date)) = moved {
            let duration = match self.knowledge_cached_value(&id, &view.date_field) {
                Ok(knowledge::CellValue::Date(d)) => knowledge::date_day(&d.end)
                    .zip(knowledge::date_day(&d.start))
                    .map_or(0, |(end, start)| end - start),
                _ => 0,
            };
            let end = knowledge::day_date(knowledge::date_day(&date).unwrap() + duration);
            self.set_knowledge_cell(
                &id,
                &view.date_field,
                knowledge::CellValue::Date(knowledge::DateRange { start: date, end }),
            );
        }
        let undated = rows
            .iter()
            .filter(|id| {
                self.knowledge_cached_value(id, &view.date_field)
                    .map_or(true, |v| v.empty())
            })
            .collect::<Vec<_>>();
        if !undated.is_empty() {
            ui.label(format!("{} 条记录尚未设置日期", undated.len()));
            for id in undated {
                if let Some(p) = records.get(id) {
                    if ui.link(&p.title).clicked() {
                        self.select_note_by_id(id);
                    }
                }
            }
        }
    }

    fn ui_knowledge_timeline(
        &mut self,
        ui: &mut egui::Ui,
        _db: &knowledge::KnowledgeDatabase,
        view: &knowledge::DatabaseView,
        rows: &[String],
        records: &KnowledgeRecordLookup<'_>,
    ) {
        let intervals = rows
            .iter()
            .filter_map(|id| {
                let Ok(knowledge::CellValue::Date(range)) =
                    self.knowledge_cached_value(id, &view.date_field)
                else {
                    return None;
                };
                let start = knowledge::date_day(&range.start)?;
                let end = knowledge::date_day(&range.end).unwrap_or(start);
                Some((id, start, end))
            })
            .collect::<Vec<_>>();
        if intervals.is_empty() {
            empty_state(ui, "设置记录的日期后，会显示在时间轴上");
            return;
        }
        let min = intervals.iter().map(|(_, s, _)| *s).min().unwrap();
        let max = intervals.iter().map(|(_, _, e)| *e).max().unwrap();
        ui.label(
            egui::RichText::new(format!(
                "{} 至 {}",
                knowledge::day_date(min),
                knowledge::day_date(max)
            ))
            .small()
            .color(palette().muted),
        );
        let span = (max - min + 1).max(7);
        let width = (span.min(366) as f32 * 30.0).max(ui.available_width() - 190.0);
        let scale = width / span as f32;
        egui::ScrollArea::both()
            .id_source("knowledge_timeline_scroll")
            .max_height(560.0)
            .show(ui, |ui| {
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(width + 190.0, 40.0 + intervals.len() as f32 * 38.0),
                    egui::Sense::hover(),
                );
                let painter = ui.painter_at(rect);
                let step = ((58.0 / scale).ceil() as i64).max((span / 12).max(1));
                let mut day = min;
                while day <= max + 1 {
                    let x = rect.left() + 190.0 + (day - min) as f32 * scale;
                    painter.line_segment(
                        [
                            egui::pos2(x, rect.top() + 24.0),
                            egui::pos2(x, rect.bottom()),
                        ],
                        egui::Stroke::new(1.0, palette().line),
                    );
                    painter.text(
                        egui::pos2(x, rect.top()),
                        egui::Align2::LEFT_TOP,
                        knowledge::day_date(day)[5..].to_string(),
                        egui::FontId::proportional(11.0),
                        palette().muted,
                    );
                    day += step;
                }
                for (i, (id, start, end)) in intervals.iter().enumerate() {
                    let y = rect.top() + 34.0 + i as f32 * 38.0;
                    let title = records.get(id).map_or("未命名", |p| p.title.as_str());
                    let label = egui::Rect::from_min_size(
                        egui::pos2(rect.left(), y),
                        egui::vec2(180.0, 28.0),
                    );
                    ui.put(label, egui::Label::new(title).wrap(false));
                    let bar = egui::Rect::from_min_size(
                        egui::pos2(rect.left() + 190.0 + (*start - min) as f32 * scale, y),
                        egui::vec2(((*end - *start + 1) as f32 * scale).max(12.0), 26.0),
                    );
                    painter.rect_filled(bar, 4.0, palette().accent);
                    let response = ui.interact(
                        bar,
                        egui::Id::new(("timeline_record", *id)),
                        egui::Sense::click(),
                    );
                    if response.clicked() {
                        self.select_note_by_id(id);
                    }
                    response.on_hover_text(format!(
                        "{title}\n{} 至 {}",
                        knowledge::day_date(*start),
                        knowledge::day_date(*end)
                    ));
                }
            });
    }

    fn knowledge_form_values_with_pending_inputs(
        &self,
        db: &knowledge::KnowledgeDatabase,
    ) -> Result<BTreeMap<String, knowledge::CellValue>, String> {
        let mut values = self.desktop_ui.knowledge.form_values.clone();
        for field in db
            .fields
            .iter()
            .filter(|field| !field.deleted && !field.kind.computed())
        {
            if matches!(
                field.kind,
                knowledge::FieldKind::Text
                    | knowledge::FieldKind::Number
                    | knowledge::FieldKind::Date
                    | knowledge::FieldKind::Url
                    | knowledge::FieldKind::Email
                    | knowledge::FieldKind::Phone
            ) {
                let key = format!("form:{}:{}", self.selected_note_id, field.id);
                if let Some(input) = self.desktop_ui.knowledge.cell_inputs.get(&key) {
                    values.insert(field.id.clone(), knowledge::parse_cell(field, &input.text)?);
                }
            }
            if let Some(value) = values.get(&field.id) {
                if !self.knowledge_relation_targets_are_current(field, value) {
                    return Err(format!("{} 的关联目标已变化，请重新选择", field.name));
                }
            }
        }
        knowledge::validate_record(db, &values)?;
        Ok(values)
    }

    fn ui_knowledge_form(
        &mut self,
        ui: &mut egui::Ui,
        db: &knowledge::KnowledgeDatabase,
        records: &[knowledge::PageRecord],
        locked: bool,
    ) {
        ui.add_enabled_ui(!locked, |ui| {
            ui.label("标题 *");
            let response = ui.add(egui::TextEdit::singleline(&mut self.desktop_ui.knowledge.form_title)
                .desired_width(f32::INFINITY).hint_text("新记录"));
            self.desktop_ui.navigation.track_document_text_edit(&response);
            for field in db.fields.iter().filter(|field| !field.deleted && !field.kind.computed()) {
                ui.add_space(8.0);
                ui.label(format!("{}{}", field.name, if field.required { " *" } else { "" }));
                let value = self.desktop_ui.knowledge.form_values.get(&field.id).cloned().unwrap_or_default();
                if let Some(Ok(value)) = knowledge_cell_editor(ui,
                    &format!("form:{}:{}", self.selected_note_id, field.id), field, &value, records,
                    &mut self.desktop_ui.knowledge.cell_inputs, &mut self.desktop_ui.navigation)
                {
                    self.desktop_ui.knowledge.form_values.insert(field.id.clone(), value);
                }
            }
            // Read the text still held by focused inputs. Invalid optional inputs
            // must not silently fall back to their previous (or empty) value.
            let values = self.knowledge_form_values_with_pending_inputs(db);
            let has_title = !self.desktop_ui.knowledge.form_title.trim().is_empty();
            if has_title {
                if let Err(error) = &values {
                    ui.colored_label(palette().danger, error);
                }
            }
            ui.add_space(12.0);
            let submit = ui.add_enabled(has_title && values.is_ok(), egui::Button::new("创建记录"));
            #[cfg(test)]
            ui.ctx().data_mut(|data| data.insert_temp(egui::Id::new("knowledge_form_submit"), (submit.rect, submit.enabled())));
            if submit.clicked() {
                let Ok(values) = values else { return; };
                let title = self.desktop_ui.knowledge.form_title.trim().to_string();
                let parent = self.selected_note_id.clone();
                if self.flush_note_draft().is_err() {
                    return;
                }
                let note = json!({"id":random_desktop_identifier("page"),"kind":"DOCUMENT","title":title,"document":{"blocks":[],"knowledge":knowledge::KnowledgePage{parent_id:Some(parent),properties:values,..Default::default()}}});
                if self.replace_state(app_data::upsert_note_app_data_json(&self.state_json, &note.to_string(), now_millis()), "记录已创建") {
                    self.desktop_ui.knowledge.form_title.clear();
                    self.desktop_ui.knowledge.form_values.clear();
                    self.desktop_ui.knowledge.cell_inputs.retain(|key, _| !key.starts_with("form:"));
                    self.desktop_ui.knowledge.error.clear();
                }
            }
        });
    }
}

fn knowledge_today() -> i64 {
    let date = desktop_local_timestamp(now_millis());
    knowledge::date_day(date.get(..10).unwrap_or("")).unwrap_or_else(|| now_millis() / 86_400_000)
}

fn knowledge_filter_editor(
    ui: &mut egui::Ui,
    db: &knowledge::KnowledgeDatabase,
    filter: &mut knowledge::Filter,
    depth: usize,
    id: &str,
    navigation: &mut DesktopNavigationState,
) {
    if depth > 6 {
        return;
    }
    ui.push_id(id, |ui| match filter {
        knowledge::Filter::Rule {
            field,
            operator,
            value,
        } => {
            ui.horizontal_wrapped(|ui| {
                egui::ComboBox::from_id_source("filter_field")
                    .selected_text(knowledge_field_label(db, field))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(field, "title".into(), "标题");
                        for f in db.fields.iter().filter(|f| !f.deleted) {
                            ui.selectable_value(field, f.id.clone(), &f.name);
                        }
                    });
                egui::ComboBox::from_id_source("filter_operator")
                    .selected_text(operator.label())
                    .show_ui(ui, |ui| {
                        for op in knowledge::FilterOperator::ALL {
                            ui.selectable_value(operator, op, op.label());
                        }
                    });
                if !matches!(
                    operator,
                    knowledge::FilterOperator::Empty | knowledge::FilterOperator::NotEmpty
                ) {
                    let mut text = value.text();
                    let response =
                        ui.add(egui::TextEdit::singleline(&mut text).desired_width(160.0));
                    navigation.track_document_text_edit(&response);
                    if response.changed() {
                        *value = match db.fields.iter().find(|f| f.id == *field) {
                            Some(f) if f.kind == knowledge::FieldKind::Number => text
                                .parse::<f64>()
                                .ok()
                                .filter(|n| n.is_finite())
                                .map(knowledge::CellValue::Number)
                                .unwrap_or(knowledge::CellValue::Text(text)),
                            _ => knowledge::CellValue::Text(text),
                        };
                    }
                }
            });
        }
        _ => {
            let any = matches!(filter, knowledge::Filter::Any { .. });
            let mut is_any = any;
            ui.horizontal(|ui| {
                ui.selectable_value(&mut is_any, false, "全部满足");
                ui.selectable_value(&mut is_any, true, "任一满足");
            });
            let filters = match filter {
                knowledge::Filter::All { filters } | knowledge::Filter::Any { filters } => filters,
                _ => unreachable!(),
            };
            let mut remove = None;
            for (i, child) in filters.iter_mut().enumerate() {
                ui.horizontal_top(|ui| {
                    ui.indent((id, i), |ui| {
                        knowledge_filter_editor(
                            ui,
                            db,
                            child,
                            depth + 1,
                            &format!("{id}:{i}"),
                            navigation,
                        )
                    });
                    if ui.small_button("×").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                filters.remove(i);
            }
            ui.horizontal(|ui| {
                if filters.len() < 64 && ui.small_button("＋ 条件").clicked() {
                    filters.push(knowledge::Filter::Rule {
                        field: "title".into(),
                        operator: knowledge::FilterOperator::Contains,
                        value: knowledge::CellValue::Empty,
                    });
                }
                if depth < 6 && filters.len() < 64 && ui.small_button("＋ 条件组").clicked() {
                    filters.push(knowledge::Filter::All {
                        filters: Vec::new(),
                    });
                }
            });
            if is_any != any {
                let children = std::mem::take(filters);
                *filter = if is_any {
                    knowledge::Filter::Any { filters: children }
                } else {
                    knowledge::Filter::All { filters: children }
                };
            }
        }
    });
}

impl TimerWindowsClient {
    fn knowledge_record_lookup<'a>(
        &self,
        records: &'a Arc<Vec<knowledge::PageRecord>>,
    ) -> KnowledgeRecordLookup<'a> {
        let mut cache = self.desktop_ui.knowledge.read_cache.borrow_mut();
        cache.at_version(self.notes_cache_version());
        // Only reuse an index paired with this exact immutable record snapshot.
        // Alternate/fixture snapshots keep their own index instead of poisoning it.
        if cache
            .records
            .as_ref()
            .is_some_and(|cached| Arc::ptr_eq(cached, records))
        {
            let by_id = Arc::clone(
                cache
                    .record_indices
                    .get_or_insert_with(|| KnowledgeRecordLookup::new(records).by_id),
            );
            KnowledgeRecordLookup { records, by_id }
        } else {
            KnowledgeRecordLookup::new(records)
        }
    }

    fn cached_knowledge_database_rows(
        &self,
        rows: &Arc<Vec<String>>,
        records: &Arc<Vec<knowledge::PageRecord>>,
        lookup: &KnowledgeRecordLookup<'_>,
        query: &str,
    ) -> Arc<Vec<String>> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return Arc::clone(rows);
        }
        let mut cache = self.desktop_ui.knowledge.read_cache.borrow_mut();
        cache.at_version(self.notes_cache_version());
        let source_is_current = cache
            .records
            .as_ref()
            .is_some_and(|cached| Arc::ptr_eq(cached, records));
        if source_is_current {
            if let Some((previous_rows, previous_query, result)) = &cache.record_search {
                if Arc::ptr_eq(previous_rows, rows) && previous_query == &query {
                    return Arc::clone(result);
                }
            }
        }
        let result = Arc::new(lookup.filter_rows(rows, &query).into_owned());
        if source_is_current {
            cache.record_search = Some((Arc::clone(rows), query, Arc::clone(&result)));
        }
        result
    }
}
