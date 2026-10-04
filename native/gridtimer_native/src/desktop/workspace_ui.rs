// v1.0.3.17 Windows - Keep timer controls responsive during durable writes.
// v1.0.3.16 Windows - Pace live timer projection and reuse the board drag identity per frame.
// v1.0.3.3 Windows - Scope canvas UI state to the active knowledge workspace.
// v1.0.2.2 Windows - Reuse paused timer projections and borrow timer card read models.
// v2.22.53 - Focused knowledge workspace, contextual controls and safe navigation.
// v2.22.52 - Preserve structured knowledge pages through editing and persistence.
// v2.22.50 - Validate desktop workspace state and release behavior.
// v2.22.49 - Keep five primary destinations with internal timer records and explicit return actions.
// v2.22.47 - Add Android timer details, daily statistics and drag ordering.
// v2.22.44 - Queue timer phase persistence without blocking workspace navigation.

include!("history_ui.rs");

#[derive(Clone)]
enum KnowledgeDeleteIntent {
    Folder(String),
    Trash(String),
    Permanent(String),
    Empty,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingTimerPhaseSettlement {
    workspace: String,
    slot_id: i32,
    run_id: String,
    observed_at_epoch_millis: i64,
}

#[derive(Clone)]
struct PendingTimerReset {
    workspace: String,
    slot_id: i32,
    record: Value,
}

fn timer_reset_snapshot(raw: &str, slot_id: i32) -> Option<Value> {
    serde_json::from_str::<Value>(raw)
        .ok()?
        .get("slots")?
        .as_array()?
        .iter()
        .find(|slot| slot.get("id").and_then(Value::as_i64) == Some(i64::from(slot_id)))
        .cloned()
}

fn timer_board_matches_query(view: &app_data::TimerView, category: &str, query: &str) -> bool {
    timer_board_text_matches_query(&view.title, &view.note, category, view.id, query)
}

fn timer_board_text_matches_query(
    title: &str,
    note: &str,
    category: &str,
    slot_id: i32,
    query: &str,
) -> bool {
    if query.trim().is_empty() {
        return true;
    }
    let haystack =
        format!("{title}\n{note}\n{category}\n格子 {slot_id:02}\n格子{slot_id}\n{slot_id}")
            .to_lowercase();
    query
        .split_whitespace()
        .all(|keyword| haystack.contains(&keyword.to_lowercase()))
}

fn desktop_slot_running_identity(slot: &DesktopSlot) -> Option<&str> {
    slot.running_since_epoch_millis?;
    slot.extra
        .get("activeRunId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
}

fn punctuation_character(key: egui::Key, shift: bool) -> Option<char> {
    Some(match key {
        egui::Key::Comma => {
            if shift {
                '<'
            } else {
                ','
            }
        }
        egui::Key::Period => {
            if shift {
                '>'
            } else {
                '.'
            }
        }
        egui::Key::Semicolon => {
            if shift {
                ':'
            } else {
                ';'
            }
        }
        egui::Key::Colon => ':',
        egui::Key::Slash => {
            if shift {
                '?'
            } else {
                '/'
            }
        }
        egui::Key::Questionmark => '?',
        egui::Key::Backslash => {
            if shift {
                '|'
            } else {
                '\\'
            }
        }
        egui::Key::Pipe => '|',
        egui::Key::OpenBracket => {
            if shift {
                '{'
            } else {
                '['
            }
        }
        egui::Key::CloseBracket => {
            if shift {
                '}'
            } else {
                ']'
            }
        }
        egui::Key::Minus => {
            if shift {
                '_'
            } else {
                '-'
            }
        }
        egui::Key::Equals => {
            if shift {
                '+'
            } else {
                '='
            }
        }
        egui::Key::Plus => '+',
        egui::Key::Backtick => {
            if shift {
                '~'
            } else {
                '`'
            }
        }
        _ => return None,
    })
}

fn punctuation_character_for_key_event(
    key: egui::Key,
    physical_key: Option<egui::Key>,
    shift: bool,
) -> Option<char> {
    punctuation_character(key, shift)
        .or_else(|| physical_key.and_then(|physical| punctuation_character(physical, shift)))
}

fn is_printable_punctuation_text(text: &str) -> bool {
    let mut characters = text.chars();
    let Some(character) = characters.next() else {
        return false;
    };
    characters.next().is_none()
        && matches!(
            character,
            ',' | '.'
                | ';'
                | ':'
                | '/'
                | '?'
                | '!'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | '\\'
                | '|'
                | '-'
                | '_'
                | '='
                | '+'
                | '`'
                | '~'
                | '，'
                | '。'
                | '；'
                | '：'
                | '／'
                | '？'
                | '！'
                | '（'
                | '）'
                | '【'
                | '】'
                | '「'
                | '」'
                | '『'
                | '』'
                | '、'
                | '“'
                | '”'
                | '‘'
                | '’'
        )
}

struct DesktopUiState {
    legal_risk: DesktopLegalRiskState,
    knowledge: KnowledgeWorkspaceState,
    knowledge_canvas: KnowledgeCanvasWorkspaceState,
    experience: KnowledgeExperienceState,
    parity: AndroidParityUi,
    navigation: DesktopNavigationState,
    projector: Option<app_data::TimerProjector>,
    projection: Option<Arc<app_data::TimerProjection>>,
    projection_version: u64,
    slot_editor_open: bool,
    slot_editor_focus: bool,
    note_editor_focus: bool,
    ime_composition_active: bool,
    pending_reset: Option<PendingTimerReset>,
    category_name: String,
    board_query: String,
    board_category: String,
    running_only: bool,
    history_query: String,
    history_category: String,
    history_slot: Option<i32>,
    pending_history_delete: Option<PendingHistoryDelete>,
    history_period: i32,
    history_archives: bool,
    history_index: Option<Arc<desktop_timer::TimerHistoryIndex>>,
    history_version: u64,
    history_summary: Arc<desktop_timer::TimerHistorySummary>,
    history_summary_key: String,
    history_session_rows: Vec<usize>,
    history_archive_rows: Vec<usize>,
    phase_markers: BTreeMap<i32, (String, app_data::TimerViewPhase, i32)>,
    pending_phase_settlement: Option<PendingTimerPhaseSettlement>,
    pending_knowledge_delete: Option<(KnowledgeDeleteIntent, u64)>,
}

impl Default for DesktopUiState {
    fn default() -> Self {
        Self {
            legal_risk: DesktopLegalRiskState::default(),
            knowledge: KnowledgeWorkspaceState::default(),
            knowledge_canvas: KnowledgeCanvasWorkspaceState::default(),
            experience: KnowledgeExperienceState::default(),
            parity: AndroidParityUi::default(),
            navigation: DesktopNavigationState::default(),
            projector: None,
            projection: None,
            projection_version: u64::MAX,
            slot_editor_open: false,
            slot_editor_focus: false,
            note_editor_focus: false,
            ime_composition_active: false,
            pending_reset: None,
            category_name: String::new(),
            board_query: String::new(),
            board_category: String::new(),
            running_only: false,
            history_query: String::new(),
            history_category: String::new(),
            history_slot: None,
            pending_history_delete: None,
            history_period: 0,
            history_archives: false,
            history_index: None,
            history_version: u64::MAX,
            history_summary: Arc::new(desktop_timer::TimerHistorySummary::default()),
            history_summary_key: String::new(),
            history_session_rows: Vec::new(),
            history_archive_rows: Vec::new(),
            phase_markers: BTreeMap::new(),
            pending_phase_settlement: None,
            pending_knowledge_delete: None,
        }
    }
}

impl TimerWindowsClient {
    fn selected_timer_is_visible(&self) -> bool {
        self.data.slots.iter().any(|slot| {
            slot.id == self.selected_slot_id
                && (!self.desktop_ui.running_only || slot.running_since_epoch_millis.is_some())
                && category_matches(&self.desktop_ui.board_category, slot.category_id.as_deref())
                && timer_board_text_matches_query(
                    &slot.title,
                    &slot.note,
                    &category_name(&self.data.categories, slot.category_id.as_deref()),
                    slot.id,
                    &self.desktop_ui.board_query,
                )
        })
    }

    fn workspace_edit_locked(&self) -> bool {
        self.sync_task
            .is_some_and(|task| !matches!(task.kind, SyncTaskKind::Sync | SyncTaskKind::Health))
    }

    fn check_timer_bells(&mut self) {
        if self.desktop_ui.projection_version != self.timers_cache_version() {
            self.refresh_timer_projection(now_millis());
        }
        let mut latest = None;
        let mut settle_slot = None;
        if let Some(projection) = &self.desktop_ui.projection {
            for slot in &projection.slots {
                let marker = (
                    slot.active_run_id.clone(),
                    slot.micro_break_phase,
                    slot.micro_break_cycle_index,
                );
                let due = slot.is_running
                    && self
                        .desktop_ui
                        .phase_markers
                        .get(&slot.id)
                        .is_some_and(|previous| {
                            previous.0 == marker.0
                                && (previous.1 != marker.1 || previous.2 != marker.2)
                                && slot.micro_break_phase_progress_millis <= 3_000
                        });
                if slot.is_running
                    && self
                        .desktop_ui
                        .phase_markers
                        .get(&slot.id)
                        .is_some_and(|previous| {
                            previous.0 == marker.0
                                && (previous.1 != marker.1 || previous.2 != marker.2)
                        })
                {
                    settle_slot = Some((slot.id, slot.active_run_id.clone()));
                }
                self.desktop_ui.phase_markers.insert(slot.id, marker);
                if due && slot.micro_break_phase != app_data::TimerViewPhase::Unknown {
                    latest = Some(
                        if slot.micro_break_phase == app_data::TimerViewPhase::Break {
                            DesktopTimerBellKind::Break
                        } else {
                            DesktopTimerBellKind::Focus
                        },
                    );
                }
            }
        }
        if let Some((slot_id, run_id)) = settle_slot {
            self.desktop_ui.pending_phase_settlement = Some(PendingTimerPhaseSettlement {
                workspace: self.background_job_workspace_fingerprint(),
                slot_id,
                run_id,
                observed_at_epoch_millis: now_millis(),
            });
        }
        self.retry_pending_timer_phase_settlement();
        if self.settings.timer_bell_enabled {
            if let Some(kind) = latest {
                let _ = self.start_timer_bell(kind);
            } else if self.settings.interval_reminder_enabled {
                self.check_interval_timer_bells();
            }
        }
    }

    fn retry_pending_timer_phase_settlement(&mut self) {
        let Some(pending) = self.desktop_ui.pending_phase_settlement.clone() else {
            return;
        };
        let same_running_slot = self.data.slots.iter().any(|slot| {
            slot.id == pending.slot_id
                && desktop_slot_running_identity(slot) == Some(pending.run_id.as_str())
        });
        if pending.workspace != self.background_job_workspace_fingerprint() || !same_running_slot {
            self.desktop_ui.pending_phase_settlement = None;
            return;
        }
        // The ordinary persistence worker commits this event together with any
        // pending drafts. Never run journal validation from the frame callback.
    }

    fn settle_timer_phase(&mut self, slot_id: i32) -> bool {
        let Some(run_id) = self
            .data
            .slots
            .iter()
            .find(|slot| slot.id == slot_id)
            .and_then(desktop_slot_running_identity)
            .map(str::to_owned)
        else {
            return false;
        };
        if self.workspace_edit_locked() || !self.workspace_persistence_ready {
            return false;
        }
        self.desktop_ui.pending_phase_settlement = Some(PendingTimerPhaseSettlement {
            workspace: self.background_job_workspace_fingerprint(),
            slot_id,
            run_id,
            observed_at_epoch_millis: now_millis(),
        });
        true
    }

    fn refresh_timer_projection(&mut self, now: i64) {
        if self.desktop_ui.projection_version != self.timers_cache_version() {
            self.desktop_ui.projector = app_data::TimerProjector::parse(&self.state_json).ok();
            self.desktop_ui.projection_version = self.timers_cache_version();
        } else if let Some(projection) = self.desktop_ui.projection.as_mut() {
            let now = now.max(0);
            if projection.projected_at_epoch_millis == now {
                return;
            }
            if projection.slots.iter().all(|slot| !slot.is_running) {
                // Paused clocks have no time-dependent values. Keep their
                // strings and metadata allocated across mouse/input frames;
                // retain the exact timestamp contract for projection users.
                Arc::make_mut(projection).projected_at_epoch_millis = now;
                return;
            }
        }
        self.desktop_ui.projection = self
            .desktop_ui
            .projector
            .as_ref()
            .map(|p| Arc::new(p.project(now)));
    }

    fn refresh_timer_projection_for_frame(&mut self, now: i64) {
        const LIVE_FRAME_INTERVAL_MILLIS: i64 = 200;
        if self.desktop_ui.projection_version == self.timers_cache_version() {
            if let Some(projection) = self.desktop_ui.projection.as_ref() {
                let previous = projection.projected_at_epoch_millis;
                if now >= previous
                    && (projection.slots.iter().all(|slot| !slot.is_running)
                        || now.saturating_sub(previous) < LIVE_FRAME_INTERVAL_MILLIS)
                {
                    return;
                }
            } else if self.desktop_ui.projector.is_none() {
                return;
            }
        }
        self.refresh_timer_projection(now);
    }

    fn projected_elapsed(&self, slot_id: i32) -> i64 {
        self.desktop_ui
            .projection
            .as_ref()
            .and_then(|p| p.slots.iter().find(|s| s.id == slot_id))
            .map(|s| s.accumulated_millis)
            .unwrap_or(0)
    }

    fn open_slot_editor(&mut self, slot_id: i32) {
        self.select_slot(slot_id);
        if self.selected_slot_id == slot_id {
            self.ensure_selected_slot_draft();
            self.desktop_ui.slot_editor_open = true;
            self.desktop_ui.slot_editor_focus = true;
            self.desktop_ui.category_name.clear();
        }
    }

    fn move_slot_in_order(&mut self, slot_id: i32, delta: i32) {
        if self.flush_slot_draft().is_err() {
            return;
        }
        self.refresh_timer_projection(now_millis());
        let Some(projection) = &self.desktop_ui.projection else {
            return;
        };
        let ids = projection.slots.iter().map(|s| s.id).collect::<Vec<_>>();
        let mut order = desktop_timer::ordered_slot_ids(&ids, &projection.slot_order);
        let Some(index) = order.iter().position(|id| *id == slot_id) else {
            return;
        };
        let target = index as i32 + delta;
        if target < 0 || target as usize >= order.len() {
            return;
        }
        order.swap(index, target as usize);
        self.replace_state(
            app_data::set_slot_order_app_data_json(&self.state_json, &order, now_millis()),
            "格子顺序已保存",
        );
    }

    fn restore_timer_archive(&mut self, archive_id: &str) {
        if self.workspace_edit_locked() || self.flush_all_pending_saves().is_err() {
            return;
        }
        let Some(task) = self
            .data
            .archived_tasks
            .iter()
            .find(|a| a.id == archive_id)
            .cloned()
        else {
            return;
        };
        self.refresh_timer_projection(now_millis());
        let target = self
            .desktop_ui
            .projection
            .as_ref()
            .and_then(|p| desktop_timer::restore_target_slot_id(p, task.original_slot_id));
        let Some(target) = target else {
            self.status = "没有空白格子，请先归档一个已暂停的格子".to_string();
            return;
        };
        let next = app_data::restore_archived_task_app_data_json(
            &self.state_json,
            archive_id,
            now_millis(),
        );
        if !next.as_ref().is_some_and(|raw| {
            !decode_data(raw)
                .archived_tasks
                .iter()
                .any(|a| a.id == archive_id)
        }) {
            self.status = "归档未能恢复，记录仍保留".to_string();
            return;
        }
        if self.replace_state(next, "已恢复到计时") {
            self.switch_tab(AppTab::Board);
            self.desktop_ui.board_query.clear();
            self.desktop_ui.board_category.clear();
            self.desktop_ui.running_only = false;
            self.open_slot_editor(target);
        }
    }

    fn open_timer_archives(&mut self) {
        self.open_timer_history(None, true);
    }

    fn request_timer_reset(&mut self, slot_id: i32) {
        if self.workspace_edit_locked() || self.flush_all_pending_saves().is_err() {
            return;
        }
        let Some(record) = timer_reset_snapshot(&self.state_json, slot_id) else {
            self.status = "格子已不存在".to_string();
            return;
        };
        self.desktop_ui.pending_reset = Some(PendingTimerReset {
            workspace: self.background_job_workspace_fingerprint(),
            slot_id,
            record,
        });
    }

    fn confirm_timer_reset(&mut self) -> bool {
        let Some(pending) = self.desktop_ui.pending_reset.clone() else {
            return false;
        };
        if self.workspace_edit_locked()
            || !matches!(self.shutdown_state, ClientShutdownState::Running)
        {
            self.status = "当前暂不能重置，请稍后重试".to_string();
            return false;
        }
        if pending.workspace != self.background_job_workspace_fingerprint() {
            self.desktop_ui.pending_reset = None;
            self.status = "账户已变化，请重新选择格子".to_string();
            return false;
        }
        if let Err(error) = self.flush_all_pending_saves() {
            self.status = format!("保存失败，累计未重置：{error}");
            return false;
        }
        if timer_reset_snapshot(&self.state_json, pending.slot_id).as_ref() != Some(&pending.record)
        {
            self.desktop_ui.pending_reset = None;
            self.status = "格子内容已变化，请重新确认重置".to_string();
            return false;
        }
        if self.replace_state(
            app_data::reset_slot_app_data_json(&self.state_json, pending.slot_id, now_millis()),
            "累计已重置",
        ) {
            self.desktop_ui.pending_reset = None;
            true
        } else {
            false
        }
    }

    /// Restore printable punctuation when the Windows keyboard pipeline delivers a
    /// punctuation key without its accompanying text event.
    ///
    /// winit normally emits an `Event::Key` followed by an `Event::Text`.  Some
    /// Windows keyboard layouts and IME transitions only deliver the key event,
    /// which leaves egui's text edits unable to insert characters such as `,` and
    /// `.` even though letters continue to work.  Add the missing text event once,
    /// while preserving a full-width/IME text event when one was delivered.
    fn restore_untranslated_punctuation_events(&mut self, ctx: &egui::Context) {
        if self.rich_editor.active {
            self.desktop_ui.ime_composition_active = false;
            return;
        }

        let events = ctx.input(|input| input.events.clone());
        let mut ime_active = self.desktop_ui.ime_composition_active;
        let has_ime_event = events.iter().any(|event| {
            matches!(
                event,
                egui::Event::CompositionStart
                    | egui::Event::CompositionUpdate(_)
                    | egui::Event::CompositionEnd(_)
            )
        });
        let mut fallback = Vec::new();
        for event in &events {
            match event {
                egui::Event::CompositionStart | egui::Event::CompositionUpdate(_) => {
                    ime_active = true;
                    continue;
                }
                egui::Event::CompositionEnd(text) => {
                    // egui 0.27 drops a standalone CompositionEnd when the IME
                    // never emitted CompositionStart. Preserve direct Chinese
                    // punctuation commits as a normal text event. When a real
                    // composition was active, egui will replace its preedit text
                    // itself and adding another event would duplicate the mark.
                    if !ime_active && is_printable_punctuation_text(text) {
                        fallback.push(egui::Event::Text(text.clone()));
                    }
                    ime_active = false;
                    continue;
                }
                _ => {}
            }
            let egui::Event::Key {
                key,
                physical_key,
                pressed: true,
                modifiers,
                ..
            } = event
            else {
                continue;
            };
            if modifiers.ctrl || modifiers.command || modifiers.alt {
                continue;
            }
            if ime_active || has_ime_event {
                continue;
            }
            let Some(character) =
                punctuation_character_for_key_event(*key, *physical_key, modifiers.shift)
            else {
                continue;
            };
            let full_width = match character {
                ',' => '，',
                '.' => '。',
                ';' => '；',
                ':' => '：',
                '?' => '？',
                '!' => '！',
                '(' => '（',
                ')' => '）',
                '[' => '【',
                ']' => '】',
                _ => character,
            };
            let already_has_text = events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Text(text) | egui::Event::CompositionEnd(text)
                        if text.chars().any(|ch| ch == character || ch == full_width)
                )
            });
            if !already_has_text {
                fallback.push(egui::Event::Text(character.to_string()));
            }
        }
        self.desktop_ui.ime_composition_active = ime_active;
        if !fallback.is_empty() {
            ctx.input_mut(|input| input.events.extend(fallback));
        }
    }

    fn handle_workspace_shortcuts(&mut self, ctx: &egui::Context) {
        if self.rich_editor.active {
            return;
        }
        if !matches!(self.shutdown_state, ClientShutdownState::Running) {
            return;
        }
        if self.desktop_ui.pending_history_delete.is_some() {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.desktop_ui.pending_history_delete = None;
            }
            return;
        }
        if self.desktop_ui.pending_knowledge_delete.is_some() {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.desktop_ui.pending_knowledge_delete = None;
            }
            return;
        }
        let command = egui::Modifiers::CTRL;
        if self.desktop_ui.pending_reset.is_some() || self.desktop_ui.slot_editor_open {
            if ctx.input_mut(|i| i.consume_key(command, egui::Key::S)) {
                self.status = match self.flush_all_pending_saves() {
                    Ok(()) => "已保存".to_string(),
                    Err(error) => format!("保存失败：{error}"),
                };
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.desktop_ui.pending_reset = None;
                if self.desktop_ui.slot_editor_open && self.flush_slot_draft().is_ok() {
                    self.desktop_ui.slot_editor_open = false;
                }
            }
            return;
        }
        if self.handle_knowledge_experience_shortcuts(ctx) {
            return;
        }
        if matches!(self.tab, AppTab::Notes | AppTab::Knowledge)
            && ctx.input_mut(|i| {
                i.consume_key(
                    egui::Modifiers {
                        ctrl: true,
                        shift: true,
                        ..egui::Modifiers::NONE
                    },
                    egui::Key::E,
                )
            })
        {
            self.open_rich_editor();
            return;
        }
        for (key, tab) in [
            egui::Key::Num1,
            egui::Key::Num2,
            egui::Key::Num3,
            egui::Key::Num4,
            egui::Key::Num5,
        ]
        .into_iter()
        .zip(AppTab::ALL)
        {
            if ctx.input_mut(|i| i.consume_key(command, key)) {
                self.switch_tab(tab);
            }
        }
        if ctx.input_mut(|i| i.consume_key(command, egui::Key::S)) {
            self.status = match self.flush_all_pending_saves() {
                Ok(()) => "已保存".to_string(),
                Err(e) => format!("保存失败：{e}"),
            };
        }
        if ctx.input_mut(|i| i.consume_key(command, egui::Key::F)) {
            if matches!(self.tab, AppTab::Notes | AppTab::Knowledge)
                && !self.return_to_document_list()
            {
                return;
            }
            let id = match self.tab {
                AppTab::Board => Some("board_search"),
                AppTab::History => Some("history_search"),
                AppTab::Notes | AppTab::Knowledge => Some("notes_search"),
                _ => None,
            };
            if let Some(id) = id {
                ctx.memory_mut(|m| m.request_focus(egui::Id::new(id)));
            }
        }
        if !self.workspace_edit_locked() && self.desktop_ui.pending_reset.is_none() {
            if self.tab == AppTab::Board
                && self.selected_timer_is_visible()
                && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F2))
            {
                self.open_slot_editor(self.selected_slot_id);
            }
            if matches!(self.tab, AppTab::Notes | AppTab::Knowledge)
                && ctx.input_mut(|i| i.consume_key(command, egui::Key::N))
            {
                self.new_note();
            }
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            if self.tab == AppTab::History {
                self.return_from_timer_history();
                return;
            }
            self.desktop_ui.pending_reset = None;
            if self.desktop_ui.slot_editor_open && self.flush_slot_draft().is_ok() {
                self.desktop_ui.slot_editor_open = false;
            }
        }
        if self.tab == AppTab::Board
            && self.selected_timer_is_visible()
            && !self.workspace_edit_locked()
            && !self.desktop_ui.slot_editor_open
            && self.desktop_ui.pending_reset.is_none()
            && !ctx.wants_keyboard_input()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space))
        {
            if let Some(slot) = self.selected_slot() {
                self.toggle_slot(&slot, ctx);
            }
        }
    }

    fn ui_nav(&mut self, ui: &mut egui::Ui) {
        let p = palette();
        let compact = ui.available_width() < 100.0;
        desktop_brand(ui, compact);
        let short = ui.ctx().screen_rect().height() < 620.0;
        ui.add_space(if short { 16.0 } else { 28.0 });
        if !compact {
            ui.label(egui::RichText::new("工作台").size(11.0).color(p.muted));
            ui.add_space(5.0);
        }
        for (index, tab) in AppTab::ALL.into_iter().enumerate() {
            if desktop_nav_button(ui, tab, self.tab.primary_destination() == tab, compact)
                .on_hover_text(format!("{} · Ctrl+{}", tab.title(), index + 1))
                .clicked()
            {
                self.switch_tab(tab);
            }
            ui.add_space(2.0);
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            if !compact {
                ui.label(
                    egui::RichText::new(format!(
                        "v{}",
                        WINDOWS_CLIENT_VERSION
                            .split('-')
                            .next()
                            .unwrap_or(WINDOWS_CLIENT_VERSION)
                    ))
                    .size(11.0)
                    .color(p.muted),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    status_dot(
                        ui,
                        if self.sync.token.is_empty() {
                            p.muted
                        } else {
                            p.good
                        },
                    );
                    ui.label(
                        egui::RichText::new(if self.sync.token.is_empty() {
                            "本地账户"
                        } else {
                            "已登录"
                        })
                        .size(12.0)
                        .color(p.muted),
                    );
                });
            }
        });
    }

    fn ui_header(&mut self, ui: &mut egui::Ui) {
        let p = palette();
        ui.horizontal(|ui| {
            if self.tab == AppTab::History {
                let detail_return = self
                    .desktop_ui
                    .navigation
                    .history_return
                    .as_ref()
                    .is_some_and(|origin| {
                        origin.workspace == self.background_job_workspace_fingerprint()
                            && origin.detail_slot.is_some()
                    });
                let back = ui.button(if detail_return {
                    "返回详情"
                } else {
                    "返回计时"
                });
                #[cfg(test)]
                ui.ctx()
                    .data_mut(|data| data.insert_temp(egui::Id::new("history_back"), back.rect));
                if back.clicked() {
                    self.return_from_timer_history();
                }
                ui.add_space(4.0);
            }
            let title = if self.tab == AppTab::History && self.desktop_ui.history_archives {
                desktop_language_text("归档", "Archive", "アーカイブ")
            } else {
                self.tab.title()
            };
            ui.label(egui::RichText::new(title).size(27.0).strong());
            if self.tab == AppTab::Finance {
                ui.menu_button("备份", |ui| self.ui_finance_backup_actions(ui));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let running = self
                    .data
                    .slots
                    .iter()
                    .filter(|s| s.running_since_epoch_millis.is_some())
                    .count();
                if running > 0 {
                    let editable = !self.workspace_edit_locked()
                        && self.persistence.pending_timer_action.is_none();
                    ui.add_enabled_ui(editable, |ui| {
                        let pause = action_button(ui, "暂停全部", ButtonTone::Quiet);
                        #[cfg(test)]
                        ui.ctx().data_mut(|d| {
                            d.insert_temp(
                                egui::Id::new("header_pause"),
                                (pause.rect, pause.enabled()),
                            )
                        });
                        if pause.clicked() {
                            self.pause_running_slots(ui.ctx());
                        }
                    });
                    pill(ui, &format!("{running} 个运行中"), p.accent_soft, p.accent);
                } else {
                    let (year, month, day) = local_ymd_now();
                    ui.label(
                        egui::RichText::new(format!("{year} 年 {month} 月 {day} 日"))
                            .size(12.0)
                            .color(p.muted),
                    );
                }
            });
        });
    }

    fn ui_board(&mut self, ui: &mut egui::Ui) {
        self.refresh_android_timer_summary();
        if self.desktop_ui.projection_version != self.timers_cache_version() {
            self.refresh_timer_projection(now_millis());
        }
        let Some(projection) = self.desktop_ui.projection.clone() else {
            empty_state(ui, "计时数据暂不可用");
            return;
        };
        let total = desktop_timer::total_accumulated_millis(&projection);
        let active_count = projection
            .slots
            .iter()
            .filter(|s| !desktop_timer::is_blank_timer(s))
            .count();
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            if ui.ctx().screen_rect().height() < 620.0 {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new("累计")
                            .size(12.0)
                            .color(palette().muted),
                    );
                    ui.label(
                        egui::RichText::new(format_duration(total))
                            .size(18.0)
                            .strong(),
                    );
                    ui.separator();
                    ui.label(
                        egui::RichText::new(format!(
                            "已使用 {} / {}",
                            active_count,
                            projection.slots.len()
                        ))
                        .size(12.0)
                        .color(palette().muted),
                    );
                });
                return;
            }
            ui.columns(3, |columns| {
                for (column, (label, value)) in columns.iter_mut().zip([
                    ("累计计时", format_duration(total)),
                    (
                        "已使用格子",
                        format!("{} / {}", active_count, projection.slots.len()),
                    ),
                    ("已归档", format!("{}", self.data.archived_tasks.len())),
                ]) {
                    column.label(egui::RichText::new(label).size(12.0).color(palette().muted));
                    column.label(egui::RichText::new(value).size(24.0).strong());
                }
            });
        });
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.desktop_ui.board_query)
                    .id(egui::Id::new("board_search"))
                    .margin(egui::vec2(8.0, 6.0))
                    .hint_text("搜索名称、备注或编号")
                    .desired_width(200.0),
            );
            category_filter(
                ui,
                "board_category",
                &self.data.categories,
                &mut self.desktop_ui.board_category,
            );
            ui.checkbox(&mut self.desktop_ui.running_only, "仅运行中");
            if (!self.desktop_ui.board_query.is_empty()
                || !self.desktop_ui.board_category.is_empty()
                || self.desktop_ui.running_only)
                && ui.button("清除筛选").clicked()
            {
                self.desktop_ui.board_query.clear();
                self.desktop_ui.board_category.clear();
                self.desktop_ui.running_only = false;
            }
            let records = ui.button("记录");
            #[cfg(test)]
            ui.ctx()
                .data_mut(|data| data.insert_temp(egui::Id::new("board_history"), records.rect));
            if records.clicked() {
                self.open_timer_history(None, false);
            }
            if ui
                .button(format!("归档 {}", self.data.archived_tasks.len()))
                .clicked()
            {
                self.open_timer_archives();
            }
            ui.menu_button("铃声", |ui| {
                let mut enabled = self.settings.timer_bell_enabled;
                if ui.checkbox(&mut enabled, "开启提醒").changed() {
                    self.update_bell_enabled(enabled);
                }
                let mut extra = self.settings.interval_reminder_enabled;
                if ui.checkbox(&mut extra, "额外间隔提醒").changed() {
                    let previous = self.settings.interval_reminder_enabled;
                    self.settings.interval_reminder_enabled = extra;
                    self.rebuild_bell_markers();
                    if let Err(error) = self.persist_settings() {
                        self.settings.interval_reminder_enabled = previous;
                        self.status = format!("提醒设置保存失败：{error}");
                    }
                }
                let mut interval = self.normalized_bell_interval_minutes();
                if ui
                    .add(
                        egui::DragValue::new(&mut interval)
                            .clamp_range(1..=180)
                            .suffix(" 分钟"),
                    )
                    .changed()
                {
                    self.update_bell_interval(interval);
                }
                ui.horizontal(|ui| {
                    if ui.button("试听专注铃").clicked() {
                        let _ = self.start_timer_bell(DesktopTimerBellKind::Focus);
                    }
                    if ui.button("试听休息铃").clicked() {
                        let _ = self.start_timer_bell(DesktopTimerBellKind::Break);
                    }
                });
            });
        });
        ui.add_space(8.0);
        let ids = projection.slots.iter().map(|s| s.id).collect::<Vec<_>>();
        let slots = desktop_timer::ordered_slot_ids(&ids, &projection.slot_order)
            .iter()
            .filter_map(|id| projection.slots.iter().find(|s| s.id == *id))
            .filter(|s| !self.desktop_ui.running_only || s.is_running)
            .filter(|s| category_matches(&self.desktop_ui.board_category, s.category_id.as_deref()))
            .filter(|s| {
                timer_board_matches_query(
                    s,
                    &category_name(&self.data.categories, s.category_id.as_deref()),
                    &self.desktop_ui.board_query,
                )
            })
            .collect::<Vec<_>>();
        let columns = ((ui.available_width() + 14.0) / 282.0)
            .floor()
            .clamp(1.0, 4.0) as usize;
        let width =
            (ui.available_width() - 12.0 * columns.saturating_sub(1) as f32) / columns as f32;
        let workspace_fingerprint = self.background_job_workspace_fingerprint();
        for row in slots.chunks(columns) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                for view in row {
                    ui.push_id(("timer_card", view.id), |ui| {
                        self.ui_timer_card(ui, view, width, &workspace_fingerprint)
                    });
                }
            });
            ui.add_space(12.0);
        }
        if slots.is_empty() {
            empty_state(ui, "没有符合筛选条件的格子");
        }
    }

    fn ui_timer_card(
        &mut self,
        ui: &mut egui::Ui,
        view: &app_data::TimerView,
        width: f32,
        workspace_fingerprint: &str,
    ) {
        let p = palette();
        let timer_action_pending = self.persistence.pending_timer_action.is_some();
        let this_timer_pending = self
            .persistence
            .pending_timer_action
            .as_ref()
            .filter(|action| action.slot_ids.contains(&view.id))
            .map(|action| action.kind);
        let selected = self.selected_slot_id == view.id;
        let phase_color = if view.micro_break_phase == app_data::TimerViewPhase::Break {
            p.good
        } else {
            p.accent
        };
        let (rect, drop_response) =
            ui.allocate_exact_size(egui::vec2(width, 282.0), egui::Sense::hover());
        if let Some(payload) = drop_response.dnd_release_payload::<TimerDragPayload>() {
            self.reorder_timer_from_drop(&payload, view.id);
        }
        if !ui.is_rect_visible(rect) {
            return;
        }
        ui.painter().rect(
            rect,
            12.0,
            p.panel,
            egui::Stroke::new(
                if selected { 1.5 } else { 1.0 },
                if selected { p.accent } else { p.line },
            ),
        );
        if view.is_running {
            ui.painter().rect_filled(
                egui::Rect::from_min_size(rect.min + egui::vec2(16.0, 0.0), egui::vec2(40.0, 3.0)),
                1.5,
                phase_color,
            );
        }
        let inner = rect.shrink2(egui::vec2(17.0, 14.0));
        let row = |top: f32, height: f32| {
            egui::Rect::from_min_size(
                inner.min + egui::vec2(0.0, top),
                egui::vec2(inner.width(), height),
            )
        };
        ui.allocate_ui_at_rect(row(0.0, 24.0), |ui| {
            ui.spacing_mut().interact_size.y = 22.0;
            ui.spacing_mut().button_padding.y = 2.0;
            ui.horizontal(|ui| {
                let (handle, drag) =
                    ui.allocate_exact_size(egui::vec2(14.0, 20.0), egui::Sense::drag());
                for x in [4.0, 10.0] {
                    for y in [5.0, 10.0, 15.0] {
                        ui.painter()
                            .circle_filled(handle.min + egui::vec2(x, y), 1.2, p.muted);
                    }
                }
                let drag = drag.on_hover_text("拖动排序");
                drag.dnd_set_drag_payload(TimerDragPayload {
                    slot_id: view.id,
                    workspace: workspace_fingerprint.to_owned(),
                });
                ui.label(
                    egui::RichText::new(format!("{:02}", view.id))
                        .monospace()
                        .size(12.0)
                        .color(p.muted),
                );
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(category_name(
                            &self.data.categories,
                            view.category_id.as_deref(),
                        ))
                        .size(11.0)
                        .color(p.muted),
                    )
                    .truncate(true),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.menu_button("···", |ui| {
                        if ui.button("计时记录").clicked() {
                            self.open_timer_history(Some(view.id), false);
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("前移").clicked() {
                            self.move_slot_in_order(view.id, -1);
                            ui.close_menu();
                        }
                        if ui.button("后移").clicked() {
                            self.move_slot_in_order(view.id, 1);
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("重置累计").clicked() {
                            self.request_timer_reset(view.id);
                            ui.close_menu();
                        }
                        let can_archive = !view.is_running && !desktop_timer::is_blank_timer(view);
                        if ui
                            .add_enabled(can_archive, egui::Button::new("归档"))
                            .on_hover_text("暂停后可归档，在计时页的归档中恢复")
                            .clicked()
                        {
                            self.archive_slot(view.id);
                            ui.close_menu();
                        }
                    });
                });
            });
        });
        let title = if view.title.trim().is_empty() {
            format!("格子 {:02}", view.id)
        } else {
            view.title.clone()
        };
        ui.allocate_ui_at_rect(row(31.0, 24.0), |ui| {
            if ui
                .add(
                    egui::Label::new(egui::RichText::new(&title).size(17.0).strong())
                        .truncate(true)
                        .sense(egui::Sense::click()),
                )
                .on_hover_text(format!("{title}\n点击编辑名称、分类和备注"))
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                self.open_slot_editor(view.id);
            }
        });
        ui.allocate_ui_at_rect(row(61.0, 48.0), |ui| {
            let clock = ui.add_sized(
                [ui.available_width(), 48.0],
                egui::Button::new(
                    egui::RichText::new(format_duration(view.accumulated_millis))
                        .monospace()
                        .size(32.0)
                        .strong()
                        .color(if view.is_running { phase_color } else { p.text }),
                )
                .fill(egui::Color32::TRANSPARENT)
                .stroke(egui::Stroke::NONE),
            );
            #[cfg(test)]
            ui.ctx()
                .data_mut(|d| d.insert_temp(egui::Id::new(("timer_clock", view.id)), clock.rect));
            if !timer_action_pending
                && clock
                    .on_hover_text(if view.is_running {
                        "点击暂停 · 空格键"
                    } else {
                        "点击开始 · 空格键"
                    })
                    .clicked()
            {
                if !self.slot_dirty {
                    self.select_slot(view.id);
                }
                if let Some(slot) = self
                    .data
                    .slots
                    .iter()
                    .find(|slot| slot.id == view.id)
                    .cloned()
                {
                    self.toggle_slot(&slot, ui.ctx());
                }
            }
        });
        let progress = if view.micro_break_phase_target_millis > 0 {
            (view.micro_break_phase_progress_millis as f64
                / view.micro_break_phase_target_millis as f64)
                .clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let track = row(118.0, 4.0);
        ui.painter().rect_filled(track, 2.0, p.panel_alt);
        if progress > 0.0 {
            ui.painter().rect_filled(
                egui::Rect::from_min_size(track.min, egui::vec2(track.width() * progress, 4.0)),
                2.0,
                phase_color,
            );
        }
        let phase = match view.micro_break_phase {
            app_data::TimerViewPhase::Focus => "专注",
            app_data::TimerViewPhase::Break => "微休息",
            app_data::TimerViewPhase::Unknown => "未知阶段",
        };
        ui.allocate_ui_at_rect(row(130.0, 18.0), |ui| {
            ui.horizontal(|ui| {
                status_dot(
                    ui,
                    if view.is_running {
                        phase_color
                    } else {
                        p.muted
                    },
                );
                ui.label(
                    egui::RichText::new(if view.is_running {
                        phase
                    } else if desktop_timer::is_blank_timer(view) {
                        "待开始"
                    } else {
                        "已暂停"
                    })
                    .size(12.0)
                    .color(p.muted),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let remaining = if view.micro_break_phase_target_millis > 0 {
                        format!(
                            "剩余 {}",
                            format_duration(view.micro_break_phase_remaining_millis)
                        )
                    } else {
                        "阶段不可用".to_string()
                    };
                    ui.label(egui::RichText::new(remaining).size(11.0).color(p.muted));
                });
            });
        });
        if !view.note.trim().is_empty() {
            ui.allocate_ui_at_rect(row(157.0, 18.0), |ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(preview_text(&view.note, ""))
                            .size(12.0)
                            .color(p.muted),
                    )
                    .truncate(true),
                )
                .on_hover_text(&view.note);
            });
        }
        ui.allocate_ui_at_rect(row(184.0, 18.0), |ui| {
            let (today_millis, today_count) = self
                .desktop_ui
                .parity
                .timer_stats
                .get(&view.id)
                .map(|stats| (stats.today_millis, stats.today_count))
                .unwrap_or_default();
            ui.label(
                egui::RichText::new(format!(
                    "今日 {} · {} 次",
                    format_duration(today_millis),
                    today_count
                ))
                .size(11.0)
                .color(p.muted),
            );
        });
        ui.allocate_ui_at_rect(row(220.0, 34.0), |ui| {
            ui.horizontal(|ui| {
                let toggle = ui
                    .add_enabled_ui(!timer_action_pending, |ui| {
                        action_button(
                            ui,
                            match this_timer_pending {
                                Some(TimerActionKind::Start) => "正在开始",
                                Some(TimerActionKind::Pause) => "正在暂停",
                                None if view.is_running => "暂停",
                                None => "开始",
                            },
                            if view.is_running {
                                ButtonTone::Quiet
                            } else {
                                ButtonTone::Primary
                            },
                        )
                    })
                    .inner;
                #[cfg(test)]
                ui.ctx().data_mut(|d| {
                    d.insert_temp(egui::Id::new(("timer_toggle", view.id)), toggle.rect)
                });
                if toggle.clicked() {
                    if !self.slot_dirty {
                        self.select_slot(view.id);
                    }
                    if let Some(slot) = self
                        .data
                        .slots
                        .iter()
                        .find(|slot| slot.id == view.id)
                        .cloned()
                    {
                        self.toggle_slot(&slot, ui.ctx());
                    }
                }
                let edit = action_button(ui, "详情", ButtonTone::Quiet);
                #[cfg(test)]
                ui.ctx()
                    .data_mut(|d| d.insert_temp(egui::Id::new(("timer_edit", view.id)), edit.rect));
                if edit.clicked() {
                    self.open_slot_editor(view.id);
                }
            });
        });
    }

    fn ui_timer_dialogs(&mut self, ctx: &egui::Context) {
        if let Some((intent, version)) = self.desktop_ui.pending_knowledge_delete.clone() {
            let message = match &intent {
                KnowledgeDeleteIntent::Folder(id) => format!(
                    "删除文件夹“{}”？其中的文档将移至未入库。",
                    self.data
                        .note_folders
                        .iter()
                        .find(|folder| folder.id == *id)
                        .map(|folder| folder.name.as_str())
                        .unwrap_or("已不存在的文件夹")
                ),
                KnowledgeDeleteIntent::Trash(id) => format!(
                    "将“{}”移入回收站？",
                    self.data
                        .notes
                        .iter()
                        .find(|n| n.id == *id)
                        .map(|n| n.title.as_str())
                        .unwrap_or("文档")
                ),
                KnowledgeDeleteIntent::Permanent(_) => {
                    "彻底删除这篇文档及其历史版本？此操作无法恢复。".to_string()
                }
                KnowledgeDeleteIntent::Empty => format!(
                    "清空回收站中的 {} 篇文档？此操作无法恢复。",
                    self.data
                        .notes
                        .iter()
                        .filter(|n| desktop_note_kind(n) == DesktopNoteKind::Document
                            && n.deleted_at_epoch_millis.is_some())
                        .count()
                ),
            };
            let mut action = 0;
            egui::Window::new(if matches!(&intent, KnowledgeDeleteIntent::Folder(_)) {
                "删除文件夹"
            } else {
                "删除文档"
            })
            .id(egui::Id::new("knowledge_delete_confirmation"))
            .collapsible(false)
            .resizable(false)
            .default_width(360.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(message);
                ui.horizontal(|ui| {
                    if ui.button("取消").clicked() {
                        action = 1;
                    }
                    if ui
                        .add_enabled(!self.workspace_edit_locked(), egui::Button::new("确认删除"))
                        .clicked()
                    {
                        action = 2;
                    }
                });
            });
            if action != 0 {
                self.desktop_ui.pending_knowledge_delete = None;
            }
            if action == 2 {
                if version != self.data_version {
                    self.status = "内容已发生变化，请重新选择要删除的文档".to_string();
                } else {
                    match intent {
                        KnowledgeDeleteIntent::Folder(id) => {
                            self.knowledge_folder_edit_id = Some(id);
                            self.delete_knowledge_folder();
                        }
                        KnowledgeDeleteIntent::Trash(id) => {
                            if let Some(note) = self.data.notes.iter().find(|n| n.id == id).cloned()
                            {
                                self.select_note(&note);
                                if self.selected_note_id == id {
                                    self.delete_note();
                                }
                            }
                        }
                        KnowledgeDeleteIntent::Permanent(id) => {
                            self.permanently_delete_knowledge_note(&id)
                        }
                        KnowledgeDeleteIntent::Empty => self.empty_knowledge_trash(),
                    }
                }
            }
        }
        if self.desktop_ui.slot_editor_open {
            let mut open = true;
            let mut close = false;
            egui::Window::new(format!("格子 {:02} 详情", self.selected_slot_id))
                .id(egui::Id::new("timer_editor"))
                .open(&mut open)
                .collapsible(false)
                .default_width(440.0)
                .max_width((ctx.screen_rect().width() - 48.0).max(260.0))
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.set_enabled(
                        !self.workspace_edit_locked()
                            && matches!(self.shutdown_state, ClientShutdownState::Running),
                    );
                    egui::ScrollArea::vertical()
                        .max_height((ctx.screen_rect().height() - 130.0).max(200.0))
                        .show(ui, |ui| {
                            if let Some(slot) = self.selected_slot() {
                                self.ui_android_timer_detail(ui, &slot);
                                self.ui_slot_editor(ui, &slot);
                                ui.add_space(10.0);
                                let mut category = slot.category_id.clone().unwrap_or_default();
                                let old = category.clone();
                                egui::ComboBox::from_id_source("edit_timer_category")
                                    .selected_text(category_name(
                                        &self.data.categories,
                                        slot.category_id.as_deref(),
                                    ))
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(&mut category, String::new(), "未分类");
                                        for cat in &self.data.categories {
                                            ui.selectable_value(
                                                &mut category,
                                                cat.id.clone(),
                                                &cat.name,
                                            );
                                        }
                                    });
                                if category != old && self.flush_slot_draft().is_ok() {
                                    self.replace_state(
                                        app_data::set_slot_category_app_data_json(
                                            &self.state_json,
                                            slot.id,
                                            &category,
                                            now_millis(),
                                        ),
                                        "分类已保存",
                                    );
                                }
                                ui.horizontal(|ui| {
                                    ui.add(
                                        egui::TextEdit::singleline(
                                            &mut self.desktop_ui.category_name,
                                        )
                                        .hint_text("新分类")
                                        .desired_width(180.0),
                                    );
                                    if ui
                                        .add_enabled(
                                            !self.desktop_ui.category_name.trim().is_empty(),
                                            egui::Button::new("添加并使用"),
                                        )
                                        .clicked()
                                        && self.flush_slot_draft().is_ok()
                                    {
                                        let id = random_desktop_identifier("category");
                                        if self.replace_state(
                                            app_data::add_category_and_assign_app_data_json(
                                                &self.state_json,
                                                slot.id,
                                                &id,
                                                self.desktop_ui.category_name.trim(),
                                                now_millis(),
                                            ),
                                            "分类已添加",
                                        ) {
                                            self.desktop_ui.category_name.clear();
                                        }
                                    }
                                });
                                ui.add_space(12.0);
                                ui.horizontal(|ui| {
                                    if action_button(ui, "完成", ButtonTone::Primary).clicked() {
                                        close = true;
                                    }
                                    ui.label(
                                        egui::RichText::new("Ctrl+S 保存 · Esc 关闭")
                                            .size(12.0)
                                            .color(palette().muted),
                                    );
                                });
                            }
                        });
                });
            if (!open || close) && self.flush_slot_draft().is_ok() {
                self.desktop_ui.slot_editor_open = false;
            }
        }
        if let Some(pending) = self.desktop_ui.pending_reset.clone() {
            let id = pending.slot_id;
            let mut action = 0;
            egui::Window::new("重置累计时长")
                .id(egui::Id::new("reset_timer_confirmation"))
                .collapsible(false)
                .resizable(false)
                .default_width(360.0)
                .max_width((ctx.screen_rect().width() - 48.0).max(240.0))
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(format!(
                        "格子 {id:02} 的累计将归零，名称、备注和历史记录会保留。"
                    ));
                    ui.horizontal(|ui| {
                        let cancel = ui.button("取消");
                        #[cfg(test)]
                        ctx.data_mut(|d| {
                            d.insert_temp(egui::Id::new("timer_reset_cancel"), cancel.rect)
                        });
                        if cancel.clicked() {
                            action = 1;
                        }
                        let confirm = ui.add_enabled(
                            !self.workspace_edit_locked(),
                            egui::Button::new("确认重置"),
                        );
                        #[cfg(test)]
                        ctx.data_mut(|d| {
                            d.insert_temp(egui::Id::new("timer_reset_confirm"), confirm.rect)
                        });
                        if confirm.clicked() {
                            action = 2;
                        }
                    });
                });
            if action == 1 {
                self.desktop_ui.pending_reset = None;
            }
            if action == 2 {
                self.confirm_timer_reset();
            }
        }
    }
}

// Convert calendar dates independently of the current UTC offset, so a local
// midnight crossing (including DST) agrees with Android's local date windows.
fn desktop_days_from_civil(year: i32, month: i32, day: i32) -> i64 {
    let y = i64::from(year) - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = i64::from(month) + if month > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468
}

#[cfg(target_os = "windows")]
fn desktop_local_parts(epoch: i64) -> Option<WindowsSystemTime> {
    let ticks = (epoch as i128 + 11_644_473_600_000_i128).checked_mul(10_000)?;
    let ticks = u64::try_from(ticks).ok()?;
    let file = [ticks as u32, (ticks >> 32) as u32];
    let mut utc: WindowsSystemTime = unsafe { std::mem::zeroed() };
    let mut local: WindowsSystemTime = unsafe { std::mem::zeroed() };
    unsafe {
        if desktop_file_to_system(file.as_ptr(), &mut utc) == 0
            || desktop_system_to_local(std::ptr::null(), &utc, &mut local) == 0
        {
            return None;
        }
    }
    Some(local)
}

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
extern "system" {
    #[link_name = "FileTimeToSystemTime"]
    fn desktop_file_to_system(file: *const u32, system: *mut WindowsSystemTime) -> i32;
    #[link_name = "SystemTimeToFileTime"]
    fn desktop_system_to_file(system: *const WindowsSystemTime, file: *mut u32) -> i32;
    #[link_name = "SystemTimeToTzSpecificLocalTime"]
    fn desktop_system_to_local(
        zone: *const c_void,
        utc: *const WindowsSystemTime,
        local: *mut WindowsSystemTime,
    ) -> i32;
    #[link_name = "TzSpecificLocalTimeToSystemTime"]
    fn desktop_local_to_system(
        zone: *const c_void,
        local: *const WindowsSystemTime,
        utc: *mut WindowsSystemTime,
    ) -> i32;
}

fn desktop_local_midnight(day: i64) -> i64 {
    let utc_fallback = day.saturating_mul(86_400_000);
    #[cfg(target_os = "windows")]
    {
        let (year, month, date) = utc_ymd_from_epoch_millis(utc_fallback);
        let mut local: WindowsSystemTime = unsafe { std::mem::zeroed() };
        local.year = year as u16;
        local.month = month as u16;
        local.day = date as u16;
        let mut utc: WindowsSystemTime = unsafe { std::mem::zeroed() };
        let mut file = [0_u32; 2];
        unsafe {
            if desktop_local_to_system(std::ptr::null(), &local, &mut utc) != 0
                && desktop_system_to_file(&utc, file.as_mut_ptr()) != 0
            {
                return (((u64::from(file[1]) << 32) | u64::from(file[0])) / 10_000) as i64
                    - 11_644_473_600_000;
            }
        }
    }
    utc_fallback
}

fn desktop_local_day_window(now: i64, days_back: i64) -> (i64, i64) {
    #[cfg(target_os = "windows")]
    let date = desktop_local_parts(now)
        .map(|s| (i32::from(s.year), i32::from(s.month), i32::from(s.day)))
        .unwrap_or_else(|| utc_ymd_from_epoch_millis(now));
    #[cfg(not(target_os = "windows"))]
    let date = utc_ymd_from_epoch_millis(now);
    let day = desktop_days_from_civil(date.0, date.1, date.2).saturating_sub(days_back);
    (
        desktop_local_midnight(day),
        desktop_local_midnight(day.saturating_add(1)),
    )
}

fn desktop_local_timestamp(epoch: i64) -> String {
    #[cfg(target_os = "windows")]
    if let Some(s) = desktop_local_parts(epoch) {
        return format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            s.year, s.month, s.day, s._hour, s._minute
        );
    }
    let (y, m, d) = utc_ymd_from_epoch_millis(epoch);
    let seconds = epoch.div_euclid(1000).rem_euclid(86400);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60
    )
}

fn category_name(categories: &[DesktopCategory], id: Option<&str>) -> String {
    categories
        .iter()
        .find(|c| Some(c.id.as_str()) == id)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| "未分类".to_string())
}

fn category_matches(filter: &str, id: Option<&str>) -> bool {
    filter.is_empty()
        || if filter == "__unfiled" {
            id.is_none() || id == Some("")
        } else {
            id == Some(filter)
        }
}

fn category_filter(
    ui: &mut egui::Ui,
    id: &str,
    categories: &[DesktopCategory],
    filter: &mut String,
) {
    let selected = if filter.is_empty() {
        "全部分类".to_string()
    } else {
        category_name(categories, Some(filter))
    };
    egui::ComboBox::from_id_source(id)
        .selected_text(selected)
        .show_ui(ui, |ui| {
            ui.selectable_value(filter, String::new(), "全部分类");
            ui.selectable_value(filter, "__unfiled".to_string(), "未分类");
            for category in categories {
                ui.selectable_value(filter, category.id.clone(), &category.name);
            }
        });
}
