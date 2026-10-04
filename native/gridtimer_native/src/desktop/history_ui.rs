// v2.22.48 - Present timer records as a secondary timer workspace.
// v2.22.46 - Align workspace controls and responsive pane spacing.
// v2.22.39 - Searchable archive notes, explicit restore destinations, and guarded history deletion.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HistoryRecordKind {
    Session,
    Archive,
}

impl HistoryRecordKind {
    fn collection(self) -> &'static str {
        match self {
            Self::Session => "sessions",
            Self::Archive => "archivedTasks",
        }
    }
}

#[derive(Clone)]
struct PendingHistoryDelete {
    kind: HistoryRecordKind,
    id: String,
    title: String,
    workspace: String,
    record: Value,
}

fn history_record_snapshot(raw: &str, kind: HistoryRecordKind, id: &str) -> Option<Value> {
    let value = serde_json::from_str::<Value>(raw).ok()?;
    value
        .get(kind.collection())?
        .as_array()?
        .iter()
        .find(|record| record.get("id").and_then(Value::as_str) == Some(id))
        .cloned()
}

impl TimerWindowsClient {
    fn request_history_delete(&mut self, kind: HistoryRecordKind, id: &str) {
        if self.workspace_edit_locked() {
            self.status = "账户数据正在切换，请稍后删除".to_string();
            return;
        }
        let Some(record) = history_record_snapshot(&self.state_json, kind, id) else {
            self.status = "这条历史记录已不存在".to_string();
            return;
        };
        let title_field = if kind == HistoryRecordKind::Session {
            "slotTitle"
        } else {
            "title"
        };
        let title = record
            .get(title_field)
            .and_then(Value::as_str)
            .filter(|title| !title.trim().is_empty())
            .unwrap_or("未命名记录")
            .to_string();
        self.desktop_ui.pending_history_delete = Some(PendingHistoryDelete {
            kind,
            id: id.to_string(),
            title,
            workspace: self.background_job_workspace_fingerprint(),
            record,
        });
    }

    fn confirm_history_delete(&mut self) -> bool {
        let Some(pending) = self.desktop_ui.pending_history_delete.clone() else {
            return false;
        };
        if self.workspace_edit_locked()
            || !matches!(self.shutdown_state, ClientShutdownState::Running)
        {
            self.status = "当前暂不能删除，请稍后重试".to_string();
            return false;
        }
        if let Err(error) = self.flush_all_pending_saves() {
            self.status = format!("保存失败，历史记录未删除：{error}");
            return false;
        }
        if pending.workspace != self.background_job_workspace_fingerprint()
            || history_record_snapshot(&self.state_json, pending.kind, &pending.id).as_ref()
                != Some(&pending.record)
        {
            self.desktop_ui.pending_history_delete = None;
            self.status = "这条记录或账户已变化，请重新选择".to_string();
            return false;
        }
        let next = match pending.kind {
            HistoryRecordKind::Session => {
                app_data::delete_session_app_data_json(&self.state_json, &pending.id, now_millis())
            }
            HistoryRecordKind::Archive => app_data::delete_archived_task_app_data_json(
                &self.state_json,
                &pending.id,
                now_millis(),
            ),
        };
        if self.replace_state(
            next,
            if pending.kind == HistoryRecordKind::Session {
                "计时记录已删除，格子累计保留"
            } else {
                "归档已删除，计时记录保留"
            },
        ) {
            self.desktop_ui.pending_history_delete = None;
            true
        } else {
            false
        }
    }

    fn ui_history_delete_dialog(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.desktop_ui.pending_history_delete.clone() else {
            return;
        };
        let mut action = 0;
        egui::Window::new(if pending.kind == HistoryRecordKind::Session {
            "删除计时记录"
        } else {
            "删除归档"
        })
        .id(egui::Id::new("history_delete_confirmation"))
        .collapsible(false)
        .resizable(false)
        .default_width(360.0)
        .max_width((ctx.screen_rect().width() - 48.0).max(240.0))
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.label(format!("删除“{}”？", pending.title));
            ui.label(if pending.kind == HistoryRecordKind::Session {
                "这条记录将从历史统计中移除，格子累计不变。此操作无法撤销。"
            } else {
                "删除后无法再恢复这个归档任务，已有计时记录不受影响。此操作无法撤销。"
            });
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let cancel = ui.button("取消");
                #[cfg(test)]
                ctx.data_mut(|d| {
                    d.insert_temp(egui::Id::new("history_delete_cancel"), cancel.rect)
                });
                if cancel.clicked() {
                    action = 1;
                }
                let confirm =
                    ui.add_enabled(!self.workspace_edit_locked(), egui::Button::new("确认删除"));
                #[cfg(test)]
                ctx.data_mut(|d| {
                    d.insert_temp(egui::Id::new("history_delete_confirm"), confirm.rect)
                });
                if confirm.clicked() {
                    action = 2;
                }
            });
        });
        if action == 1 {
            self.desktop_ui.pending_history_delete = None;
        }
        if action == 2 {
            self.confirm_history_delete();
        }
    }

    fn open_history_slot(&mut self, slot_id: i32) {
        if !self.data.slots.iter().any(|slot| slot.id == slot_id) {
            self.status = "原格子已不存在，历史记录仍保留".to_string();
            return;
        }
        self.switch_tab(AppTab::Board);
        if self.tab == AppTab::Board {
            self.desktop_ui.board_query.clear();
            self.desktop_ui.board_category.clear();
            self.desktop_ui.running_only = false;
            self.open_slot_editor(slot_id);
        }
    }
    fn refresh_history_index(&mut self, now: i64) {
        if self.desktop_ui.history_version != self.timers_cache_version() {
            self.desktop_ui.history_index =
                desktop_timer::TimerHistoryIndex::parse(&self.state_json)
                    .ok()
                    .map(Arc::new);
            self.desktop_ui.history_version = self.timers_cache_version();
            self.desktop_ui.history_summary_key.clear();
        }
        let today = desktop_local_day_window(now, 0);
        let week = (desktop_local_day_window(now, 6).0, today.1);
        let key = format!(
            "{}\0{}\0{}\0{}\0{}\0{:?}",
            self.timers_cache_version(),
            today.0,
            self.desktop_ui.history_period,
            self.desktop_ui.history_category,
            self.desktop_ui.history_query,
            self.desktop_ui.history_slot
        );
        if key == self.desktop_ui.history_summary_key {
            return;
        }
        let Some(index) = &self.desktop_ui.history_index else {
            return;
        };
        let category_id = match self.desktop_ui.history_category.as_str() {
            "" => None,
            "__unfiled" => Some(""),
            id => Some(id),
        };
        let summary = index.summarize(
            &desktop_timer::TimerHistoryFilter {
                query: &self.desktop_ui.history_query,
                category_id,
                slot_id: self.desktop_ui.history_slot,
            },
            today,
            week,
        );
        let session_ids = summary.session_ids.iter().collect::<HashSet<_>>();
        let archive_ids = summary.archived_task_ids.iter().collect::<HashSet<_>>();
        let window = match self.desktop_ui.history_period {
            1 => today,
            2 => week,
            _ => (i64::MIN, i64::MAX),
        };
        self.desktop_ui.history_session_rows = index
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                session_ids.contains(&s.id)
                    && s.ended_at_epoch_millis >= window.0
                    && s.ended_at_epoch_millis < window.1
            })
            .map(|(i, _)| i)
            .collect();
        self.desktop_ui.history_archive_rows = index
            .archived_tasks
            .iter()
            .enumerate()
            .filter(|(_, a)| {
                archive_ids.contains(&a.id)
                    && a.archived_at_epoch_millis >= window.0
                    && a.archived_at_epoch_millis < window.1
            })
            .map(|(i, _)| i)
            .collect();
        self.desktop_ui.history_summary = Arc::new(summary);
        self.desktop_ui.history_summary_key = key;
    }

    fn ui_history(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            desktop_segment(ui, &mut self.desktop_ui.history_archives, false, "计时记录");
            desktop_segment(ui, &mut self.desktop_ui.history_archives, true, "归档");
            ui.separator();
            for (code, label) in [(0, "全部"), (1, "今天"), (2, "近7天")] {
                desktop_segment(ui, &mut self.desktop_ui.history_period, code, label);
            }
        });
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.desktop_ui.history_query)
                    .id(egui::Id::new("history_search"))
                    .margin(egui::vec2(8.0, 6.0))
                    .desired_width(210.0)
                    .hint_text(if self.desktop_ui.history_archives {
                        "搜索名称、备注或编号"
                    } else {
                        "搜索名称、分类或编号"
                    }),
            );
            category_filter(
                ui,
                "history_category",
                &self.data.categories,
                &mut self.desktop_ui.history_category,
            );
            egui::ComboBox::from_id_source("history_slot_filter")
                .selected_text(
                    self.desktop_ui
                        .history_slot
                        .map(|id| format!("格子 {id:02}"))
                        .unwrap_or_else(|| "全部格子".to_string()),
                )
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.desktop_ui.history_slot, None, "全部格子");
                    for slot in &self.data.slots {
                        let title = if slot.title.trim().is_empty() {
                            format!("格子 {:02}", slot.id)
                        } else {
                            format!("{:02} · {}", slot.id, preview_text(&slot.title, ""))
                        };
                        ui.selectable_value(
                            &mut self.desktop_ui.history_slot,
                            Some(slot.id),
                            title,
                        );
                    }
                });
            if (!self.desktop_ui.history_query.is_empty()
                || !self.desktop_ui.history_category.is_empty()
                || self.desktop_ui.history_slot.is_some()
                || self.desktop_ui.history_period != 0)
                && ui.button("清除筛选").clicked()
            {
                self.desktop_ui.history_query.clear();
                self.desktop_ui.history_category.clear();
                self.desktop_ui.history_slot = None;
                self.desktop_ui.history_period = 0;
            }
        });
        let previous_summary_key = self.desktop_ui.history_summary_key.clone();
        self.refresh_history_index(now_millis());
        let filters_changed = previous_summary_key != self.desktop_ui.history_summary_key;
        let Some(index) = self.desktop_ui.history_index.clone() else {
            empty_state(ui, "计时记录暂不可用");
            return;
        };
        let summary = self.desktop_ui.history_summary.clone();
        ui.add_space(14.0);
        metric_tile_grid(
            ui,
            &[
                (
                    "今日专注",
                    format_duration(summary.today_millis),
                    palette().accent,
                ),
                (
                    "近7天",
                    format_duration(summary.week_millis),
                    palette().good,
                ),
                (
                    "全部记录",
                    format_duration(summary.total_millis),
                    palette().blue,
                ),
            ],
        );
        ui.add_space(12.0);
        let archives = self.desktop_ui.history_archives;
        if archives && self.desktop_ui.projection_version != self.timers_cache_version() {
            self.refresh_timer_projection(now_millis());
        }
        let count = if archives {
            self.desktop_ui.history_archive_rows.len()
        } else {
            self.desktop_ui.history_session_rows.len()
        };
        ui.label(
            egui::RichText::new(format!(
                "{count} 条{}",
                if archives { "归档" } else { "记录" }
            ))
            .size(13.0)
            .color(palette().muted),
        );
        if count == 0 {
            empty_state(
                ui,
                if archives {
                    "没有符合条件的归档"
                } else {
                    "没有符合条件的记录"
                },
            );
            return;
        }
        let mut restore = None;
        let mut open_slot = None;
        let mut delete = None;
        let mut scroll = egui::ScrollArea::vertical()
            .id_source(("history_rows", archives))
            .max_height(
                (ui.available_height() - 12.0)
                    .clamp(180.0, (ui.ctx().screen_rect().height() - 270.0).max(180.0)),
            );
        if filters_changed {
            scroll = scroll.vertical_scroll_offset(0.0);
        }
        scroll.show_rows(
            ui,
            if archives { 134.0 } else { 112.0 },
            count,
            |ui, rows| {
                for row in rows {
                    ui.push_id(("history_row", archives, row), |ui| {
                        let (title, category, start, end, duration, record_id, slot_id) =
                            if archives {
                                let task = &index.archived_tasks
                                    [self.desktop_ui.history_archive_rows[row]];
                                (
                                    if task.title.is_empty() {
                                        format!("格子 {:02}", task.original_slot_id)
                                    } else {
                                        task.title.clone()
                                    },
                                    task.category_id.as_deref(),
                                    task.archived_at_epoch_millis,
                                    task.archived_at_epoch_millis,
                                    task.accumulated_millis,
                                    task.id.as_str(),
                                    task.original_slot_id,
                                )
                            } else {
                                let session =
                                    &index.sessions[self.desktop_ui.history_session_rows[row]];
                                (
                                    if session.slot_title.is_empty() {
                                        format!("格子 {:02}", session.slot_id)
                                    } else {
                                        session.slot_title.clone()
                                    },
                                    session.category_id.as_deref(),
                                    session.started_at_epoch_millis,
                                    session.ended_at_epoch_millis,
                                    session.duration_millis,
                                    session.id.as_str(),
                                    session.slot_id,
                                )
                            };
                        card_frame().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.set_min_height(if archives { 104.0 } else { 82.0 });
                            ui.horizontal(|ui| {
                                let right_width = 120.0;
                                let left_width = (ui.available_width() - right_width).max(80.0);
                                ui.allocate_ui_with_layout(
                                    egui::vec2(left_width, 44.0),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        ui.add(
                                            egui::Label::new(egui::RichText::new(&title).strong())
                                                .truncate(true),
                                        )
                                        .on_hover_text(&title);
                                        let time = if archives {
                                            desktop_local_timestamp(end)
                                        } else {
                                            format!(
                                                "{} 至 {}",
                                                desktop_local_timestamp(start),
                                                desktop_local_timestamp(end)
                                            )
                                        };
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(format!(
                                                    "格子 {slot_id:02} · {} · {time}",
                                                    category_name(&self.data.categories, category)
                                                ))
                                                .size(11.0)
                                                .color(palette().muted),
                                            )
                                            .truncate(true),
                                        )
                                        .on_hover_text(time);
                                    },
                                );
                                ui.label(
                                    egui::RichText::new(format_duration(duration))
                                        .size(18.0)
                                        .strong()
                                        .color(palette().accent),
                                );
                            });
                            if archives {
                                let note = &index.archived_tasks
                                    [self.desktop_ui.history_archive_rows[row]]
                                    .note;
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(preview_text(note, "无备注"))
                                            .size(12.0)
                                            .color(palette().muted),
                                    )
                                    .truncate(true),
                                )
                                .on_hover_text(note);
                            }
                            ui.horizontal(|ui| {
                                if archives {
                                    let target =
                                        self.desktop_ui.projection.as_ref().and_then(|p| {
                                            desktop_timer::restore_target_slot_id(p, slot_id)
                                        });
                                    let response = ui
                                        .add_enabled(
                                            !self.workspace_edit_locked() && target.is_some(),
                                            egui::Button::new(
                                                target
                                                    .map(|id| format!("恢复到格子 {id:02}"))
                                                    .unwrap_or_else(|| "暂无空白格子".to_string()),
                                            ),
                                        )
                                        .on_hover_text(if target.is_some() {
                                            "恢复名称、分类、备注和累计时长"
                                        } else {
                                            "先归档一个已暂停的格子，再恢复此任务"
                                        });
                                    #[cfg(test)]
                                    ui.ctx().data_mut(|d| {
                                        d.insert_temp(
                                            egui::Id::new(("history_restore", record_id)),
                                            response.rect,
                                        )
                                    });
                                    if response.clicked() {
                                        restore = Some(record_id.to_string());
                                    }
                                } else {
                                    let available =
                                        self.data.slots.iter().any(|slot| slot.id == slot_id);
                                    let response =
                                        ui.add_enabled(available, egui::Button::new("打开格子"));
                                    #[cfg(test)]
                                    ui.ctx().data_mut(|d| {
                                        d.insert_temp(
                                            egui::Id::new(("history_open", record_id)),
                                            response.rect,
                                        )
                                    });
                                    if response.clicked() {
                                        open_slot = Some(slot_id);
                                    }
                                }
                                let response = ui.add_enabled(
                                    !self.workspace_edit_locked(),
                                    egui::Button::new("删除"),
                                );
                                #[cfg(test)]
                                ui.ctx().data_mut(|d| {
                                    d.insert_temp(
                                        egui::Id::new(("history_delete", record_id)),
                                        response.rect,
                                    )
                                });
                                if response.clicked() {
                                    delete = Some((
                                        if archives {
                                            HistoryRecordKind::Archive
                                        } else {
                                            HistoryRecordKind::Session
                                        },
                                        record_id.to_string(),
                                    ));
                                }
                            });
                        });
                    });
                }
            },
        );
        if let Some(id) = restore {
            self.restore_timer_archive(&id);
        } else if let Some(slot_id) = open_slot {
            self.open_history_slot(slot_id);
        } else if let Some((kind, id)) = delete {
            self.request_history_delete(kind, &id);
        }
    }
}
