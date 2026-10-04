// v2.22.54 - Borrow shared records while choosing templates.
// v2.22.52 - Import preview, portable export and reusable page subtrees.

impl TimerWindowsClient {
    fn knowledge_subtree_pages(&self, root: Option<&str>) -> Vec<Value> {
        let records = self.knowledge_records();
        let selected = root.map(|id| {
            let mut selected = HashSet::from([id.to_string()]);
            let mut queue = vec![id.to_string()];
            while let Some(parent) = queue.pop() {
                for p in records.iter().filter(|p| {
                    p.meta.parent_id.as_deref() == Some(&parent) && !p.deleted && !p.encrypted
                }) {
                    if selected.insert(p.id.clone()) {
                        queue.push(p.id.clone());
                    }
                }
            }
            selected
        });
        self.data
            .notes
            .iter()
            .filter(|p| {
                desktop_note_kind(p) == DesktopNoteKind::Document
                    && p.deleted_at_epoch_millis.is_none()
                    && p.encryption.is_none()
                    && selected.as_ref().map_or(true, |ids| ids.contains(&p.id))
            })
            .filter_map(|p| serde_json::to_value(p).ok())
            .collect()
    }

    fn duplicate_knowledge_page(&mut self, id: &str) -> bool {
        if self.flush_note_draft().is_err() {
            return false;
        }
        let pages = self.knowledge_subtree_pages(Some(id));
        let copied = match knowledge::remap_pages(&pages, random_desktop_identifier) {
            Ok(p) => p,
            Err(error) => {
                self.status = error;
                return false;
            }
        };
        let root = copied
            .iter()
            .find(|p| p["document"]["knowledge"]["parentId"].is_null())
            .and_then(|p| p["id"].as_str())
            .unwrap_or_default()
            .to_string();
        if self.replace_state(
            app_data::upsert_knowledge_pages_app_data_json(
                &self.state_json,
                &serde_json::to_string(&copied).unwrap_or_default(),
                now_millis(),
            ),
            "页面与子页面已复制",
        ) {
            self.select_note_by_id(&root);
            true
        } else {
            false
        }
    }

    fn prepare_knowledge_import(&mut self, extension: &str) {
        match desktop_transfer_file_dialog(false, "导入资料", "", extension, extension) {
            Ok(Some(path)) => {
                let mut media = Vec::new();
                let prepared = (|| -> Result<Vec<Value>, String> {
                    let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
                    if metadata.len() > 64 * 1024 * 1024 {
                        return Err("导入文件不能超过 64 MB".into());
                    }
                    let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
                    let title = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("导入文档");
                    match extension {
                        "csv" => knowledge::csv_to_pages(&raw, title, random_desktop_identifier),
                        "md" => {
                            let source = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
                            let (title, body) = source
                                .strip_prefix("# ")
                                .and_then(|s| s.split_once('\n'))
                                .map_or((title, source), |(heading, rest)| (heading.trim(), rest));
                            let blocks =
                                knowledge::markdown_blocks(body, new_desktop_note_block_id);
                            Ok(vec![
                                json!({"id":random_desktop_identifier("page"),"kind":"DOCUMENT","title":title,"document":{"blocks":blocks,"knowledge":knowledge::KnowledgePage::default()}}),
                            ])
                        }
                        "json" => {
                            let value: Value =
                                serde_json::from_str(&raw).map_err(|e| e.to_string())?;
                            let (pages, attachments) = prepare_knowledge_transfer(&value)?;
                            media = attachments;
                            Ok(pages)
                        }
                        _ => Err("不支持此文件格式".into()),
                    }
                })();
                match prepared {
                    Ok(pages) => {
                        let raw = serde_json::to_string(&pages).unwrap_or_default();
                        if app_data::upsert_knowledge_pages_app_data_json(
                            &self.state_json,
                            &raw,
                            now_millis(),
                        )
                        .is_none()
                        {
                            self.status = "导入内容未通过完整校验，原数据未改变".into();
                            return;
                        }
                        self.desktop_ui.knowledge.import_workspace =
                            self.background_job_workspace_fingerprint();
                        self.desktop_ui.knowledge.import_path = path.display().to_string();
                        self.desktop_ui.knowledge.import_pages = Some(pages);
                        self.desktop_ui.knowledge.import_media = media;
                        self.desktop_ui.knowledge.error.clear();
                    }
                    Err(error) => self.status = format!("导入失败：{error}"),
                }
            }
            Ok(None) => {}
            Err(error) => self.status = error.to_string(),
        }
    }

    fn ui_knowledge_import_preview(&mut self, ui: &mut egui::Ui) {
        if self.desktop_ui.knowledge.import_workspace != self.background_job_workspace_fingerprint()
        {
            self.desktop_ui.knowledge.import_pages = None;
            self.desktop_ui.knowledge.import_media.clear();
            return;
        }
        let Some(pages) = self.desktop_ui.knowledge.import_pages.clone() else {
            return;
        };
        card_frame().show(ui, |ui| {
            ui.strong(format!("将导入 {} 个页面", pages.len()));
            ui.label(&self.desktop_ui.knowledge.import_path);
            egui::ScrollArea::vertical()
                .id_source("knowledge_import_preview")
                .max_height(150.0)
                .show(ui, |ui| {
                    for page in pages.iter().take(30) {
                        ui.label(page["title"].as_str().unwrap_or("未命名"));
                    }
                });
            ui.horizontal(|ui| {
                if ui.button("导入").clicked() {
                    if self.flush_note_draft().is_ok() {
                        let media = self.desktop_ui.knowledge.import_media.clone();
                        if self.commit_knowledge_import(&pages, &media) {
                            self.desktop_ui.knowledge.import_media.clear();
                            self.desktop_ui.knowledge.import_pages = None;
                            let root = pages
                                .iter()
                                .find(|p| p["document"]["knowledge"]["parentId"].is_null())
                                .and_then(|p| p["id"].as_str())
                                .unwrap_or_default();
                            self.select_note_by_id(root);
                        }
                    }
                }
                if ui.button("取消").clicked() {
                    self.desktop_ui.knowledge.import_media.clear();
                    self.desktop_ui.knowledge.import_pages = None;
                }
            });
        });
    }

    fn export_knowledge(&mut self, extension: &str, all: bool) {
        if extension == "html" {
            self.export_selected_note_html();
            return;
        }
        if self.flush_note_draft().is_err() {
            return;
        }
        let selected = self.selected_note();
        let content = (|| -> Result<String, String> {
            if extension == "json" {
                let pages = self.knowledge_subtree_pages(if all {
                    None
                } else {
                    Some(&self.selected_note_id)
                });
                if pages.is_empty() {
                    return Err("没有可导出的未加密页面".into());
                }
                return self.knowledge_transfer_json(pages);
            }
            let note = selected.as_ref().ok_or("请先选择页面")?;
            if self.selected_note_is_locked() {
                return Err("请先解锁页面".into());
            }
            if extension == "csv" {
                let db = note
                    .document
                    .knowledge
                    .as_ref()
                    .and_then(|p| p.database.as_ref())
                    .ok_or("请先打开数据库")?;
                let records = self.knowledge_records();
                let mut engine = knowledge::QueryEngine::new(&records, knowledge_today());
                let view = db
                    .views
                    .iter()
                    .find(|v| v.id == self.desktop_ui.knowledge.active_view)
                    .or_else(|| db.views.iter().find(|v| v.id == db.default_view_id))
                    .ok_or("数据库缺少视图")?;
                let ids = engine.rows(&note.id, view);
                let fields = db.fields.iter().filter(|f| !f.deleted).collect::<Vec<_>>();
                let mut rows = vec![std::iter::once("标题".to_string())
                    .chain(fields.iter().map(|f| f.name.clone()))
                    .collect::<Vec<_>>()];
                for id in ids {
                    let mut row = vec![engine.value(&id, "title")?.text()];
                    for f in &fields {
                        row.push(engine.value(&id, &f.id)?.text());
                    }
                    rows.push(row);
                }
                return Ok(knowledge::write_csv(&rows));
            }
            let blocks =
                serde_json::to_value(note_canvas_blocks(note)).map_err(|e| e.to_string())?;
            Ok(format!(
                "# {}\n\n{}\n",
                note.title,
                knowledge::blocks_markdown(blocks.as_array().ok_or("正文无效")?)
            ))
        })();
        let content = match content {
            Ok(value) => value,
            Err(error) => {
                self.status = format!("导出失败：{error}");
                return;
            }
        };
        let name = format!("knowledge_export_{}.{}", now_millis(), extension);
        match desktop_transfer_file_dialog(true, "导出知识资料", &name, extension, extension)
        {
            Ok(Some(path)) => match write_transfer_text_verified(&path, &content) {
                Ok(()) => {
                    self.desktop_ui.knowledge.export_path = path.display().to_string();
                    self.transfers.last_note_export = Some(path.clone());
                    self.status = format!("已导出：{}", path.display());
                }
                Err(error) => self.status = format!("导出失败：{error}"),
            },
            Ok(None) => {}
            Err(error) => self.status = error.to_string(),
        }
    }

    fn ui_knowledge_workspace_tools(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("导入", |ui| {
            for (extension, label) in [
                ("md", "Markdown"),
                ("csv", "CSV 数据表"),
                ("json", "知识工作区"),
            ] {
                if ui.button(label).clicked() {
                    self.prepare_knowledge_import(extension);
                    ui.close_menu();
                }
            }
        });
        ui.menu_button("导出", |ui| {
            for (extension, label) in [
                ("md", "Markdown"),
                ("html", "完整 HTML"),
                ("csv", "数据库 CSV"),
                ("json", "当前页面与子页面"),
            ] {
                if ui
                    .add_enabled(
                        !self.selected_note_id.is_empty() && !self.selected_note_is_locked(),
                        egui::Button::new(label),
                    )
                    .clicked()
                {
                    self.export_knowledge(extension, false);
                    ui.close_menu();
                }
            }
            if ui.button("整个知识工作区 JSON").clicked() {
                self.export_knowledge("json", true);
                ui.close_menu();
            }
        });
        ui.menu_button("复用", |ui| {
            if ui
                .add_enabled(
                    !self.selected_note_id.is_empty() && !self.selected_note_is_locked(),
                    egui::Button::new("复制当前页面与子页面"),
                )
                .clicked()
            {
                self.duplicate_knowledge_page(&self.selected_note_id.clone());
                ui.close_menu();
            }
            let records = self.knowledge_records();
            let templates = records
                .iter()
                .filter(|p| p.meta.template && !p.encrypted && !p.deleted)
                .collect::<Vec<_>>();
            if !templates.is_empty() {
                ui.separator();
                for p in templates {
                    if ui.button(&p.title).clicked() {
                        self.duplicate_knowledge_page(&p.id);
                        ui.close_menu();
                    }
                }
            }
        });
    }
}
