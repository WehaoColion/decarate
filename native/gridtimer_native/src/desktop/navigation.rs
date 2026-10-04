// v1.0.3.2 Windows - Release document text-edit histories at note and session boundaries.
// v2.22.48 - Preserve timer detail origins behind the workspace boundary.
// v2.22.41 - Restore the knowledge collection after a round trip through trash.

#[derive(Clone)]
struct TimerHistoryReturn {
    workspace: String,
    detail_slot: Option<i32>,
}

struct KnowledgeBrowseReturn {
    workspace: String,
    selection: String,
    query: String,
    editor_visible: bool,
}

#[derive(Default)]
struct DesktopNavigationState {
    history_return: Option<TimerHistoryReturn>,
    editor_visible: bool,
    editor_epoch: u64,
    editor_context: Option<egui::Context>,
    editor_text_states: HashSet<egui::Id>,
    sticky_selection: Option<(String, String)>,
    knowledge_selection: Option<(String, String)>,
    knowledge_return: Option<KnowledgeBrowseReturn>,
}

impl DesktopNavigationState {
    fn track_document_text_edit(&mut self, response: &egui::Response) {
        self.editor_context
            .get_or_insert_with(|| response.ctx.clone());
        self.editor_text_states.insert(response.id);
    }

    fn forget_document_text_edits(&mut self) {
        let Some(ctx) = self.editor_context.take() else {
            return;
        };
        // A new widget ID only hides the old undo buffer. Remove its state so
        // the previous document's plaintext and per-widget history are dropped.
        ctx.data_mut(|data| {
            for id in &self.editor_text_states {
                data.remove::<egui::text_edit::TextEditState>(*id);
            }
        });
        ctx.memory_mut(|memory| {
            for id in self.editor_text_states.drain() {
                memory.surrender_focus(id);
            }
        });
    }
}

impl TimerWindowsClient {
    fn open_timer_history(&mut self, slot_id: Option<i32>, archives: bool) -> bool {
        if self.tab != AppTab::Board
            || self.workspace_edit_locked()
            || self.rich_editor.active
            || !matches!(self.shutdown_state, ClientShutdownState::Running)
            || slot_id.is_some_and(|id| !self.data.slots.iter().any(|slot| slot.id == id))
        {
            return false;
        }
        let origin = TimerHistoryReturn {
            workspace: self.background_job_workspace_fingerprint(),
            detail_slot: self
                .desktop_ui
                .slot_editor_open
                .then_some(self.selected_slot_id),
        };
        self.switch_tab(AppTab::History);
        if self.tab != AppTab::History {
            return false;
        }
        self.desktop_ui.navigation.history_return = Some(origin);
        self.desktop_ui.history_slot = slot_id;
        self.desktop_ui.history_archives = archives;
        self.desktop_ui.history_period = 0;
        self.desktop_ui.history_query.clear();
        self.desktop_ui.history_category.clear();
        true
    }

    fn return_from_timer_history(&mut self) {
        if self.tab != AppTab::History {
            return;
        }
        let origin = self.desktop_ui.navigation.history_return.clone();
        self.switch_tab(AppTab::Board);
        if self.tab != AppTab::Board {
            return;
        }
        if let Some(origin) =
            origin.filter(|origin| origin.workspace == self.background_job_workspace_fingerprint())
        {
            if let Some(slot_id) = origin
                .detail_slot
                .filter(|id| self.data.slots.iter().any(|slot| slot.id == *id))
            {
                self.open_slot_editor(slot_id);
            }
        }
    }

    fn remember_document_selection(&mut self) {
        let selection = (!self.selected_note_id.is_empty()).then(|| {
            (
                self.background_job_workspace_fingerprint(),
                self.selected_note_id.clone(),
            )
        });
        match self.tab {
            AppTab::Notes => self.desktop_ui.navigation.sticky_selection = selection,
            AppTab::Knowledge => self.desktop_ui.navigation.knowledge_selection = selection,
            _ => {}
        }
    }

    fn restore_document_selection(&mut self) {
        let selection = match self.tab {
            AppTab::Notes => self.desktop_ui.navigation.sticky_selection.clone(),
            AppTab::Knowledge => self.desktop_ui.navigation.knowledge_selection.clone(),
            _ => None,
        };
        if let Some((workspace, id)) = selection {
            if workspace == self.background_job_workspace_fingerprint()
                && self.data.notes.iter().any(|note| {
                    note.id == id
                        && desktop_note_kind(note) == self.active_note_kind()
                        && note.deleted_at_epoch_millis.is_none()
                })
            {
                self.select_note_by_id(&id);
            }
        }
    }

    fn return_to_document_list(&mut self) -> bool {
        if self.flush_note_draft().is_err() {
            return false;
        }
        self.desktop_ui.navigation.editor_visible = false;
        self.desktop_ui.note_editor_focus = false;
        true
    }

    fn ui_document_back(&mut self, ui: &mut egui::Ui) {
        let response = ui.button(if self.tab == AppTab::Knowledge {
            "返回文档列表"
        } else {
            "返回便签列表"
        });
        #[cfg(test)]
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::new("document_back"), response.rect));
        if response.clicked() {
            self.return_to_document_list();
        }
        ui.add_space(8.0);
    }

    fn ui_notes_toolbar(&mut self, ui: &mut egui::Ui) {
        let compact_editor = compact_split_layout(ui.available_width())
            && self.desktop_ui.navigation.editor_visible
            && !self.note_trash_mode;
        ui.horizontal_wrapped(|ui| {
            if compact_editor {
                let back = ui.button("返回便签列表");
                #[cfg(test)]
                ui.ctx()
                    .data_mut(|data| data.insert_temp(egui::Id::new("document_back"), back.rect));
                if back.clicked() {
                    self.return_to_document_list();
                }
            }
            if !self.note_trash_mode && action_button(ui, "新建便签", ButtonTone::Primary).clicked()
            {
                self.note_search_draft.clear();
                self.view_cache.data_version = u64::MAX;
                self.new_note();
                self.rebuild_view_cache();
            }
            if ui
                .button(if self.note_trash_mode {
                    "返回便签"
                } else {
                    "回收站"
                })
                .clicked()
            {
                let _ = self.toggle_note_trash_mode();
            }
        });
    }
}

fn document_body_rows(ui: &egui::Ui) -> usize {
    // Leave room for save controls while keeping the first lines visible on a short window.
    ((ui.ctx().screen_rect().height() - 350.0) / 20.0).clamp(4.0, 22.0) as usize
}
