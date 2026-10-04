// v1.0.3 Windows - Cache history rows and comparisons without copying recovery snapshots each frame.
// v2.22.52 - Named snapshots and explicit comparisons before restoring one page.

struct KnowledgeHistoryRow {
    id: String,
    label: String,
    timestamp: String,
}

struct KnowledgeHistoryComparisonKey {
    page_id: String,
    revision_id: String,
    draft_revision: u64,
    editor_epoch: u64,
    dirty: bool,
}

struct KnowledgeHistoryComparison {
    page_id: String,
    revision_id: String,
    added: usize,
    removed: usize,
    changed: usize,
    title_change: Option<(String, String)>,
    properties_changed: bool,
    old_markdown: String,
    current_markdown: String,
}

fn knowledge_history_comparison(
    note: &DesktopNote,
    revision: &DesktopNoteRevisionSnapshot,
    title: &str,
    blocks: &[DesktopNoteBlock],
    meta: &Option<knowledge::KnowledgePage>,
) -> KnowledgeHistoryComparison {
    let mut old_blocks = std::collections::HashMap::with_capacity(revision.document.blocks.len());
    for block in &revision.document.blocks {
        old_blocks.entry(block.id.as_str()).or_insert(block);
    }
    let new_ids = blocks
        .iter()
        .map(|block| block.id.as_str())
        .collect::<HashSet<_>>();
    let added = new_ids
        .iter()
        .filter(|id| !old_blocks.contains_key(**id))
        .count();
    let removed = old_blocks
        .keys()
        .filter(|id| !new_ids.contains(**id))
        .count();
    let changed = blocks
        .iter()
        .filter(|block| {
            old_blocks
                .get(block.id.as_str())
                .is_some_and(|old| **old != **block)
        })
        .count();
    let markdown = |blocks: &[DesktopNoteBlock]| {
        knowledge::blocks_markdown(
            &blocks
                .iter()
                .filter_map(|block| serde_json::to_value(block).ok())
                .collect::<Vec<_>>(),
        )
    };
    KnowledgeHistoryComparison {
        page_id: note.id.clone(),
        revision_id: revision.id.clone(),
        added,
        removed,
        changed,
        title_change: (title != revision.title).then(|| (revision.title.clone(), title.to_owned())),
        properties_changed: meta != &revision.document.knowledge,
        old_markdown: markdown(&revision.document.blocks),
        current_markdown: markdown(blocks),
    }
}

impl TimerWindowsClient {
    fn cached_knowledge_history_rows(&self) -> Option<Arc<Vec<KnowledgeHistoryRow>>> {
        let mut cache = self.desktop_ui.knowledge.read_cache.borrow_mut();
        cache.at_version(self.notes_cache_version());
        if self.selected_note_is_locked() {
            cache.history_rows = None;
            cache.history_comparison = None;
            return None;
        }
        if !cache
            .history_rows
            .as_ref()
            .is_some_and(|(id, _)| id == &self.selected_note_id)
        {
            cache.history_rows = None;
            let note = self.selected_note_ref()?;
            let rows = note
                .revisions
                .iter()
                .map(|revision| KnowledgeHistoryRow {
                    id: revision.id.clone(),
                    label: revision.label.clone(),
                    timestamp: desktop_local_timestamp(revision.captured_at_epoch_millis),
                })
                .collect();
            cache.history_rows = Some((note.id.clone(), Arc::new(rows)));
        }
        Some(Arc::clone(&cache.history_rows.as_ref()?.1))
    }

    fn cached_knowledge_history_comparison(&self) -> Option<Arc<KnowledgeHistoryComparison>> {
        let mut cache = self.desktop_ui.knowledge.read_cache.borrow_mut();
        cache.at_version(self.notes_cache_version());
        if self.selected_note_is_locked() {
            cache.history_rows = None;
            cache.history_comparison = None;
            return None;
        }
        let revision_id = &self.desktop_ui.knowledge.compare_revision;
        if let Some((key, comparison)) = &cache.history_comparison {
            if key.page_id == self.selected_note_id
                && key.revision_id == *revision_id
                && key.draft_revision == self.persistence.revisions.note
                && key.editor_epoch == self.desktop_ui.navigation.editor_epoch
                && key.dirty == self.note_dirty
            {
                return Some(Arc::clone(comparison));
            }
        }
        cache.history_comparison = None;
        let note = self.selected_note_ref()?;
        let revision = note
            .revisions
            .iter()
            .find(|revision| revision.id == *revision_id)?;
        let (title, blocks, meta) = if self.note_dirty {
            (
                self.note_title_draft.as_str(),
                self.note_blocks_draft.as_slice(),
                &self.desktop_ui.knowledge.page,
            )
        } else {
            (
                note.title.as_str(),
                note.document.blocks.as_slice(),
                &note.document.knowledge,
            )
        };
        let comparison = Arc::new(knowledge_history_comparison(
            note, revision, title, blocks, meta,
        ));
        cache.history_comparison = Some((
            KnowledgeHistoryComparisonKey {
                page_id: note.id.clone(),
                revision_id: revision_id.clone(),
                draft_revision: self.persistence.revisions.note,
                editor_epoch: self.desktop_ui.navigation.editor_epoch,
                dirty: self.note_dirty,
            },
            Arc::clone(&comparison),
        ));
        Some(comparison)
    }

    fn create_named_knowledge_version(&mut self) -> bool {
        let name = self.desktop_ui.knowledge.version_name.trim().to_string();
        if name.is_empty() || name.chars().count() > 120 || self.selected_note_is_locked() {
            return false;
        }
        if self.flush_note_draft().is_err() {
            return false;
        }
        let Some(note) = self.selected_note() else {
            return false;
        };
        let now = now_millis();
        let mut unlocked = None;
        let next = if note.encryption.is_some() {
            let id = random_desktop_identifier("revision");
            let Some(mut updated) = desktop_note_with_recovery_point(&note, &id, now) else {
                return false;
            };
            let Some(revision) = updated.revisions.iter_mut().find(|r| r.id == id) else {
                return false;
            };
            revision.label = name;
            let encoded = serde_json::to_string(&updated).unwrap_or_default();
            let sealed =
                gridtimer_native::seal_desktop_note_json(&encoded, &self.note_crypto_session_token);
            unlocked = Some(updated);
            sealed
                .and_then(|json| app_data::upsert_note_app_data_json(&self.state_json, &json, now))
        } else {
            app_data::capture_knowledge_version_app_data_json(
                &self.state_json,
                &note.id,
                &name,
                now,
            )
        };
        if self.replace_state(next, "命名版本已保存") {
            self.desktop_ui.knowledge.version_name.clear();
            let updated =
                unlocked.or_else(|| self.data.notes.iter().find(|p| p.id == note.id).cloned());
            if let Some(updated) = updated {
                self.load_note_draft_without_flush(&updated);
            }
            true
        } else {
            false
        }
    }

    fn ui_knowledge_page_history(&mut self, ui: &mut egui::Ui) {
        if self.selected_note_is_locked() || self.selected_note_ref().is_none() {
            return;
        }
        ui.horizontal_wrapped(|ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.desktop_ui.knowledge.version_name)
                    .desired_width(240.0)
                    .char_limit(120)
                    .hint_text("版本名称，如 开题定稿"),
            );
            self.desktop_ui
                .navigation
                .track_document_text_edit(&response);
            if ui
                .add_enabled(
                    !self.selected_note_is_locked()
                        && !self.desktop_ui.knowledge.version_name.trim().is_empty(),
                    egui::Button::new("保存命名版本"),
                )
                .clicked()
            {
                self.create_named_knowledge_version();
            }
            if ui.button("创建恢复点").clicked() {
                self.create_note_recovery_point();
            }
        });
        let Some(rows) = self.cached_knowledge_history_rows() else {
            return;
        };
        egui::ScrollArea::vertical()
            .id_source("knowledge_named_versions")
            .max_height(230.0)
            .show(ui, |ui| {
                for revision in rows.iter() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            egui::RichText::new(if revision.label.is_empty() {
                                "自动恢复点"
                            } else {
                                &revision.label
                            })
                            .strong(),
                        );
                        ui.label(
                            egui::RichText::new(&revision.timestamp)
                                .small()
                                .color(palette().muted),
                        );
                        if ui
                            .selectable_label(
                                self.desktop_ui.knowledge.compare_revision == revision.id,
                                "查看差异",
                            )
                            .clicked()
                        {
                            self.desktop_ui.knowledge.compare_revision = revision.id.clone();
                        }
                    });
                }
                if rows.is_empty() {
                    ui.label(egui::RichText::new("暂无历史版本").color(palette().muted));
                }
            });
        let Some(comparison) = self.cached_knowledge_history_comparison() else {
            return;
        };
        ui.separator();
        ui.label(format!(
            "新增 {} 块 · 删除 {} 块 · 修改 {} 块",
            comparison.added, comparison.removed, comparison.changed,
        ));
        if let Some((old, current)) = &comparison.title_change {
            ui.label(format!("标题：{old} → {current}"));
        }
        if comparison.properties_changed {
            ui.label("页面属性或数据库设置有变化");
        }
        let mut old = comparison.old_markdown.as_str();
        let mut current = comparison.current_markdown.as_str();
        ui.columns(2, |columns| {
            columns[0].strong("历史内容");
            let old_response = columns[0].add(
                egui::TextEdit::multiline(&mut old)
                    .interactive(false)
                    .desired_width(f32::INFINITY)
                    .desired_rows(12),
            );
            self.desktop_ui
                .navigation
                .track_document_text_edit(&old_response);
            columns[1].strong("当前内容");
            let current_response = columns[1].add(
                egui::TextEdit::multiline(&mut current)
                    .interactive(false)
                    .desired_width(f32::INFINITY)
                    .desired_rows(12),
            );
            self.desktop_ui
                .navigation
                .track_document_text_edit(&current_response);
        });
        if ui
            .add_enabled(
                !self.knowledge_page_locked(),
                egui::Button::new("恢复此页面到该版本"),
            )
            .clicked()
        {
            self.restore_note_revision(&comparison.page_id, &comparison.revision_id);
            self.desktop_ui.knowledge.compare_revision.clear();
        }
    }
}
