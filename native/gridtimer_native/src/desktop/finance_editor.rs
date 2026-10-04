// v1.1.0.5 Windows - Keep precise cents in ledger drafts and period summaries.
// v2.22.48 - Open risk overview first and keep the ledger pages within the same workspace.
// v2.22.47 - Add all Android financial overview pages and reference periods.
// v2.22.39 - Separate finance workflows and show live day and month totals.

include!("finance_money_ui.rs");

struct FinanceWorkbenchState {
    money_scope: String,
    money_drafts: DesktopMoneyDrafts,
    overview: FinanceOverviewState,
    tab: usize,
    day_key: String,
    month_key: String,
    period_year: String,
    quarter: i32,
    summary_cache_key: Option<(u64, u64, String, i32, bool, String)>,
    summary_cache: Option<FinancePeriodSummary>,
    pending_delete: Option<String>,
    risk_cache_key: Option<(u64, u64, (i32, i32, i32))>,
    risk_cache: Option<Arc<DesktopFinanceRiskSnapshot>>,
    review: Option<DesktopFinanceReviewState>,
    risk_v2_cache_key: Option<(u64, u64, u64, String)>,
    risk_v2_cache: Option<Arc<gridtimer_native::AndroidFinanceRiskV2>>,
}

impl Default for FinanceWorkbenchState {
    fn default() -> Self {
        Self {
            money_scope: String::new(),
            money_drafts: DesktopMoneyDrafts::default(),
            overview: FinanceOverviewState::default(),
            tab: 5,
            day_key: String::new(),
            month_key: String::new(),
            period_year: String::new(),
            quarter: 0,
            summary_cache_key: None,
            summary_cache: None,
            pending_delete: None,
            risk_cache_key: None,
            risk_cache: None,
            review: None,
            risk_v2_cache_key: None,
            risk_v2_cache: None,
        }
    }
}

impl FinanceWorkbenchState {
    fn cached_risk(
        &mut self,
        key: (u64, u64, (i32, i32, i32)),
        build: impl FnOnce() -> Option<DesktopFinanceRiskSnapshot>,
    ) -> Option<Arc<DesktopFinanceRiskSnapshot>> {
        if self.risk_cache_key != Some(key) {
            self.risk_cache = build().map(Arc::new);
            self.risk_cache_key = Some(key);
        }
        self.risk_cache.clone()
    }
}

impl TimerWindowsClient {
    fn finance_money_inputs_ready(&mut self) -> bool {
        let scope = format!(
            "{}:{}:{}:{}",
            self.state_path.display(),
            self.sync.server_instance_id,
            self.sync.account_namespace,
            self.sync.user_id
        );
        // The route can be edited without changing ownership or the workspace.
        // It must not discard unfinished amounts and bypass the save barrier.
        self.finance_workbench.money_scope = scope.clone();
        self.finance_workbench
            .money_drafts
            .reconcile(scope, &self.finance_draft);
        !self.finance_workbench.money_drafts.has_invalid()
    }

    fn cached_finance_risk_snapshot(&mut self) -> Option<Arc<DesktopFinanceRiskSnapshot>> {
        let key = (
            self.finance_cache_version(),
            self.persistence.revisions.finance,
            local_ymd_now(),
        );
        let profile = &self.finance_draft;
        self.finance_workbench
            .cached_risk(key, || desktop_finance_risk_snapshot(profile))
    }

    fn ui_finance_workbench(&mut self, ui: &mut egui::Ui) {
        self.finance_money_inputs_ready();
        let finance_version = self.finance_cache_version();
        if self.desktop_ui.legal_risk.open {
            self.ui_legal_risk(ui);
            return;
        }
        let now = now_millis();
        let today = desktop_local_timestamp(now)
            .chars()
            .take(10)
            .collect::<String>();
        if self.finance_workbench.day_key.is_empty() {
            self.finance_workbench.day_key = today.clone();
        }
        if self.finance_workbench.month_key.is_empty() {
            self.finance_workbench.month_key = today.chars().take(7).collect();
        }
        if self.finance_workbench.period_year.is_empty() {
            self.finance_workbench.period_year = today.chars().take(4).collect();
            self.finance_workbench.quarter = (local_ymd_now().1 - 1) / 3 + 1;
        }
        // These fields already live in the draft's JSON map. Move ownership
        // for this synchronous UI pass instead of cloning every day/month on
        // every repaint; only the selected record is copied for editing below.
        let mut profile = Value::Object(std::mem::take(&mut self.finance_draft.extra));
        let mut changed = false;
        let mut save_now = false;
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                for (tab, title) in [
                    (5, "总览"),
                    (0, "每日收支"),
                    (1, "月度资产"),
                    (3, "季度汇总"),
                    (4, "年度汇总"),
                    (2, "分类与目标"),
                    (6, "月度参数"),
                ] {
                    let response = desktop_segment(ui, &mut self.finance_workbench.tab, tab, title);
                    #[cfg(test)]
                    ui.ctx().data_mut(|data| {
                        data.insert_temp(egui::Id::new(("finance_tab", tab)), response.rect)
                    });
                    if response.changed() {
                        self.finance_workbench.pending_delete = None;
                    }
                }

                if (self.finance_workbench.tab != 5 || self.finance_dirty)
                    && ui
                        .add_enabled_ui(!self.finance_workbench.money_drafts.has_invalid(), |ui| {
                            action_button(ui, "保存", ButtonTone::Primary)
                        })
                        .inner
                        .clicked()
                {
                    save_now = true;
                }
                if self.finance_dirty {
                    ui.label(egui::RichText::new("未保存").color(palette().warn));
                }
            });
            if let Some(field) = self.finance_workbench.money_drafts.first_invalid() {
                ui.colored_label(
                    palette().warn,
                    "金额输入尚未完成，请修正或恢复原金额后保存、核对。",
                );
                ui.horizontal_wrapped(|ui| {
                    if ui.button("定位输入").clicked() {
                        match &field {
                            DesktopMoneyField::Row {
                                monthly, period, ..
                            } => {
                                self.finance_workbench.tab = if *monthly { 1 } else { 0 };
                                if *monthly {
                                    self.finance_workbench.month_key = period.clone();
                                } else {
                                    self.finance_workbench.day_key = period.clone();
                                }
                            }
                            DesktopMoneyField::Legacy(_) => self.finance_workbench.tab = 6,
                        }
                        self.finance_workbench.money_drafts.focus = Some(field);
                    }
                    if ui
                        .button("恢复原金额")
                        .on_hover_text("撤销本工作区未完成的金额输入，保留各条账目最近一次有效金额")
                        .clicked()
                    {
                        self.finance_workbench.money_drafts.cancel_invalid();
                        self.status = "已恢复最近一次有效金额".into();
                    }
                });
            }
            if !matches!(self.finance_workbench.tab, 5 | 6) {
                ui.add_space(12.0);
            }
            match self.finance_workbench.tab {
                5 | 6 => {}
                3 | 4 => {
                    finance_period_summary_ui(
                        ui,
                        &mut self.finance_workbench,
                        &profile,
                        &today,
                        (finance_version, self.persistence.revisions.finance),
                    );
                }
                1 => {
                    changed = finance_month_editor(
                        ui,
                        &mut self.finance_workbench,
                        &mut profile,
                        &today,
                        now,
                    )
                }
                2 => changed = finance_settings_editor(ui, &mut profile),
                _ => {
                    changed = finance_day_editor(
                        ui,
                        &mut self.finance_workbench,
                        &mut profile,
                        &today,
                        now,
                    )
                }
            }
        });
        if let Value::Object(extra) = profile {
            self.finance_draft.extra = extra;
        }
        if changed {
            self.mark_finance_dirty();
        }
        if self.finance_workbench.money_drafts.has_invalid() {
            self.status = "金额输入尚未完成，请先修正或恢复原金额".into();
        }
        if save_now {
            let _ = self.flush_finance_draft();
        }
    }
}

fn finance_key_selector(
    ui: &mut egui::Ui,
    key: &mut String,
    profile: &Value,
    collection: &str,
    today: &str,
    month: bool,
) {
    ui.horizontal_wrapped(|ui| {
        ui.label(if month { "月份" } else { "日期" });
        let previous = finance_adjacent_key(key, month, -1);
        if ui
            .add_enabled(
                previous.is_some(),
                egui::Button::new(if month { "上月" } else { "前一天" }),
            )
            .clicked()
        {
            *key = previous.expect("enabled for a valid previous date");
        }
        ui.add(
            egui::TextEdit::singleline(key)
                .desired_width(130.0)
                .hint_text(if month { "YYYY-MM" } else { "YYYY-MM-DD" }),
        );
        let next = finance_adjacent_key(key, month, 1);
        if ui
            .add_enabled(
                next.is_some(),
                egui::Button::new(if month { "下月" } else { "后一天" }),
            )
            .clicked()
        {
            *key = next.expect("enabled for a valid next date");
        }
        if ui.button(if month { "本月" } else { "今日" }).clicked() {
            *key = today.chars().take(if month { 7 } else { 10 }).collect();
        }
        if let Some(records) = profile
            .get(collection)
            .and_then(Value::as_object)
            .filter(|records| !records.is_empty())
        {
            egui::ComboBox::from_id_source((collection, "saved_keys"))
                .selected_text("已有记录")
                .show_ui(ui, |ui| {
                    for candidate in records.keys().rev() {
                        ui.selectable_value(key, candidate.clone(), candidate);
                    }
                });
        }
    });
}

fn finance_day_editor(
    ui: &mut egui::Ui,
    state: &mut FinanceWorkbenchState,
    profile: &mut Value,
    today: &str,
    now: i64,
) -> bool {
    let previous_key = state.day_key.clone();
    finance_key_selector(
        ui,
        &mut state.day_key,
        profile,
        "dailyLedgers",
        today,
        false,
    );
    if state.day_key != previous_key {
        state.pending_delete = None;
    }
    if !finance_record_key_valid("dailyLedgers", &state.day_key) {
        ui.colored_label(palette().warn, "请输入有效日期，例如 2026-09-08");
        return false;
    }
    let mut ledger = profile.get("dailyLedgers").and_then(|records| records.get(&state.day_key))
        .cloned().unwrap_or_else(|| serde_json::json!({"incomes":[], "expenses":[], "note":"", "confirmedAtEpochMillis":0}));
    let options = finance_expense_options(profile);
    ui.add_space(10.0);
    let income_options = [
        ("ACTIVE", "主动收入"),
        ("ASSET", "资产收入"),
        ("OTHER", "其他收入"),
    ]
    .into_iter()
    .map(|(code, label)| (code.to_string(), label.to_string()))
    .collect::<Vec<_>>();
    let scope = format!("{}:day:{}", state.money_scope, state.day_key);
    let mut changed = finance_rows_editor(
        ui,
        &mut ledger,
        "incomes",
        "收入",
        "kind",
        &income_options,
        &scope,
        false,
        &state.day_key,
        &mut state.money_drafts,
        &mut state.pending_delete,
        now,
    );
    ui.add_space(10.0);
    changed |= finance_rows_editor(
        ui,
        &mut ledger,
        "expenses",
        "支出",
        "bucket",
        &options,
        &scope,
        false,
        &state.day_key,
        &mut state.money_drafts,
        &mut state.pending_delete,
        now,
    );
    ui.add_space(8.0);
    finance_record_totals_ui(ui, &ledger, false);
    changed |= finance_json_text(ui, &mut ledger, "note", "当日备注", true);
    let was_confirmed = ledger
        .get("confirmedAtEpochMillis")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        > 0;
    let invalid_money = state.money_drafts.invalid_record(false, &state.day_key);
    let mut confirmed = was_confirmed && !invalid_money;
    changed |= was_confirmed && invalid_money;
    let confirm_response = ui.add_enabled(
        !state.money_drafts.has_invalid(),
        egui::Checkbox::new(&mut confirmed, "当天收支已记完整（无收支也可确认）"),
    );
    #[cfg(test)]
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            egui::Id::new(("finance_complete", false, &state.day_key)),
            (confirm_response.rect, confirm_response.enabled()),
        )
    });
    let confirmed_changed = confirm_response.changed();
    if changed || confirmed_changed {
        // Editing a confirmed day requires reconfirmation. A deliberate check
        // in the same frame is honored and provides evidence for a zero day.
        ledger["confirmedAtEpochMillis"] = Value::from(if confirmed_changed && confirmed {
            now.max(1)
        } else {
            0
        });
        return finance_put_record(profile, "dailyLedgers", &state.day_key, ledger);
    }
    false
}

fn finance_month_editor(
    ui: &mut egui::Ui,
    state: &mut FinanceWorkbenchState,
    profile: &mut Value,
    today: &str,
    now: i64,
) -> bool {
    let previous_key = state.month_key.clone();
    finance_key_selector(
        ui,
        &mut state.month_key,
        profile,
        "monthlySnapshots",
        today,
        true,
    );
    if state.month_key != previous_key {
        state.pending_delete = None;
    }
    if !finance_record_key_valid("monthlySnapshots", &state.month_key) {
        ui.colored_label(palette().warn, "请输入有效月份，例如 2026-09");
        return false;
    }
    let mut snapshot = profile.get("monthlySnapshots").and_then(|records| records.get(&state.month_key))
        .cloned().unwrap_or_else(|| serde_json::json!({"assets":[], "liabilities":[], "note":"", "confirmedAtEpochMillis":0}));
    let asset_options = [
        ("CASH_RESERVE", "现金储备"),
        ("PRODUCTIVE_ASSET", "生产资产"),
        ("OTHER_ASSET", "其他资产"),
    ]
    .into_iter()
    .map(|(code, label)| (code.to_string(), label.to_string()))
    .collect::<Vec<_>>();
    let liability_options = [
        ("LIABILITY_BALANCE", "负债余额"),
        ("OTHER_LIABILITY", "其他负债"),
    ]
    .into_iter()
    .map(|(code, label)| (code.to_string(), label.to_string()))
    .collect::<Vec<_>>();
    let scope = format!("{}:month:{}", state.money_scope, state.month_key);
    ui.add_space(10.0);
    let mut changed = finance_rows_editor(
        ui,
        &mut snapshot,
        "assets",
        "资产",
        "kind",
        &asset_options,
        &scope,
        true,
        &state.month_key,
        &mut state.money_drafts,
        &mut state.pending_delete,
        now,
    );
    ui.add_space(10.0);
    changed |= finance_rows_editor(
        ui,
        &mut snapshot,
        "liabilities",
        "负债",
        "kind",
        &liability_options,
        &scope,
        true,
        &state.month_key,
        &mut state.money_drafts,
        &mut state.pending_delete,
        now,
    );
    ui.add_space(8.0);
    finance_record_totals_ui(ui, &snapshot, true);
    changed |= finance_json_text(ui, &mut snapshot, "note", "月度备注", true);
    let was_confirmed = snapshot
        .get("confirmedAtEpochMillis")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        > 0;
    let invalid_money = state.money_drafts.invalid_record(true, &state.month_key);
    let mut confirmed = was_confirmed && !invalid_money;
    changed |= was_confirmed && invalid_money;
    let confirm_response = ui.add_enabled(
        !state.money_drafts.has_invalid(),
        egui::Checkbox::new(&mut confirmed, "本月资产与负债已核对"),
    );
    #[cfg(test)]
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            egui::Id::new(("finance_complete", true, &state.month_key)),
            (confirm_response.rect, confirm_response.enabled()),
        )
    });
    let confirmed_changed = confirm_response.changed();
    if changed || confirmed_changed {
        snapshot["confirmedAtEpochMillis"] = Value::from(if confirmed_changed && confirmed {
            now.max(1)
        } else {
            0
        });
        return finance_put_record(profile, "monthlySnapshots", &state.month_key, snapshot);
    }
    false
}

fn finance_record_totals(record: &Value, monthly: bool) -> Option<(i64, i64, i64)> {
    let sum = |field: &str| {
        record
            .get(field)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|row| {
                row.get("deletedAtEpochMillis")
                    .and_then(Value::as_i64)
                    .unwrap_or(0)
                    <= 0
            })
            .try_fold(0_i64, |total, row| {
                desktop_row_money_minor(row).map(|minor| total.saturating_add(minor.max(0)))
            })
    };
    let positive = sum(if monthly { "assets" } else { "incomes" })?;
    let negative = sum(if monthly { "liabilities" } else { "expenses" })?;
    Some((positive, negative, positive.saturating_sub(negative)))
}

fn finance_record_totals_ui(ui: &mut egui::Ui, record: &Value, monthly: bool) {
    let Some((positive, negative, net)) = finance_record_totals(record, monthly) else {
        ui.colored_label(palette().warn, "金额资料待核对，暂不合计");
        return;
    };
    ui.horizontal_wrapped(|ui| {
        for (title, amount, color) in [
            (
                if monthly {
                    "资产合计"
                } else {
                    "收入合计"
                },
                positive,
                palette().good,
            ),
            (
                if monthly {
                    "负债合计"
                } else {
                    "支出合计"
                },
                negative,
                palette().warn,
            ),
            (
                if monthly { "净资产" } else { "当日结余" },
                net,
                if net < 0 {
                    palette().danger
                } else {
                    palette().accent
                },
            ),
        ] {
            ui.label(egui::RichText::new(format!("{title} {}", money_label(amount))).color(color));
            ui.add_space(8.0);
        }
    });
    ui.add_space(8.0);
}

#[allow(clippy::too_many_arguments)]
fn finance_rows_editor(
    ui: &mut egui::Ui,
    record: &mut Value,
    list_name: &str,
    label: &str,
    kind_field: &str,
    options: &[(String, String)],
    scope: &str,
    monthly: bool,
    period: &str,
    drafts: &mut DesktopMoneyDrafts,
    pending_delete: &mut Option<String>,
    now: i64,
) -> bool {
    let mut rows = record
        .get(list_name)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut changed = false;
    let mut add = false;
    ui.horizontal_wrapped(|ui| {
        section_heading(ui, label);
        if ui.button(format!("＋ 添加{label}")).clicked() {
            add = true;
        }
    });
    for (index, row) in rows.iter_mut().enumerate() {
        if row
            .get("deletedAtEpochMillis")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            > 0
        {
            continue;
        }
        let original_row = row.clone();
        let row_id = row
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let widget_key = format!("{scope}:{list_name}:{row_id}:{index}");
        ui.push_id(&widget_key, |ui| {
            ui.add_space(6.0);
            let mut row_changed = false;
            let mut delete_confirmed = false;
            card_frame().show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let mut name = row
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut name)
                                .desired_width(165.0)
                                .hint_text(format!("{label}名称")),
                        )
                        .changed()
                    {
                        row["name"] = Value::String(name);
                        row_changed = true;
                    }
                    let mut kind = row
                        .get(kind_field)
                        .and_then(Value::as_str)
                        .unwrap_or_else(|| {
                            options
                                .first()
                                .map(|(code, _)| code.as_str())
                                .unwrap_or("OTHER")
                        })
                        .to_string();
                    let selected = options
                        .iter()
                        .find(|(code, _)| *code == kind)
                        .map(|(_, name)| name.as_str())
                        .unwrap_or("未知分类");
                    egui::ComboBox::from_id_source("entry_kind")
                        .selected_text(selected)
                        .show_ui(ui, |ui| {
                            for (code, name) in options {
                                if ui.selectable_value(&mut kind, code.clone(), name).changed() {
                                    row_changed = true;
                                }
                            }
                        });
                    if row_changed {
                        row[kind_field] = Value::String(kind);
                    }
                    let whole = match row.get("amount") {
                        None | Some(Value::Null) => 0,
                        Some(value) => value.as_i64().unwrap_or(i64::MIN),
                    };
                    let minor = match row.get("amountMinor") {
                        None | Some(Value::Null) => None,
                        Some(value) => Some(value.as_i64().unwrap_or(i64::MIN)),
                    };
                    let field = DesktopMoneyField::Row {
                        monthly,
                        period: period.into(),
                        list: list_name.into(),
                        id: row_id.clone(),
                        index: if row_id.is_empty() { index } else { 0 },
                    };
                    if let Some(minor) =
                        desktop_money_input(ui, &widget_key, whole, minor, field, drafts)
                    {
                        row_changed |= desktop_apply_row_money_input(
                            row,
                            &gridtimer_native::finance_money::format_minor(minor, "", false, false),
                        );
                    }
                    ui.label("元");
                    if ui.button("删除").clicked() {
                        *pending_delete = Some(widget_key.clone());
                    }
                });
                if list_name == "incomes" || list_name == "expenses" {
                    row_changed |= finance_json_text(ui, row, "note", "备注", false);
                }
                if pending_delete.as_deref() == Some(widget_key.as_str()) {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("删除这条记录？");
                        if action_button(ui, "确认删除", ButtonTone::Danger).clicked() {
                            delete_confirmed = true;
                            *pending_delete = None;
                        }
                        if ui.button("取消").clicked() {
                            *pending_delete = None;
                        }
                    });
                }
            });
            if row_changed || delete_confirmed {
                if finance_touch_entry(row, now, delete_confirmed) {
                    changed = true;
                } else {
                    *row = original_row;
                    ui.colored_label(palette().warn, "该条记录的修订号无法更新，已保留原内容");
                }
            }
        });
    }
    if add {
        let kind = options
            .first()
            .map(|(code, _)| code.as_str())
            .unwrap_or("OTHER");
        let id = format!("finance-{:032x}", rand::random::<u128>());
        let mut row = serde_json::json!({"id":id,"name":format!("新{label}"), "amount":0,"updatedAtEpochMillis":now.max(1),"deletedAtEpochMillis":0});
        row[kind_field] = Value::String(kind.to_string());
        if list_name == "incomes" || list_name == "expenses" {
            row["note"] = Value::String(String::new());
        }
        rows.push(row);
        changed = true;
    }
    if changed {
        record[list_name] = Value::Array(rows);
    }
    changed
}

fn finance_json_text(
    ui: &mut egui::Ui,
    object: &mut Value,
    field: &str,
    label: &str,
    multiline: bool,
) -> bool {
    let mut value = object
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let response = if multiline {
        ui.add(
            egui::TextEdit::multiline(&mut value)
                .desired_width(f32::INFINITY)
                .desired_rows(2)
                .hint_text(label),
        )
    } else {
        ui.add(
            egui::TextEdit::singleline(&mut value)
                .desired_width(f32::INFINITY)
                .hint_text(label),
        )
    };
    if response.changed() {
        object[field] = Value::String(value);
        true
    } else {
        false
    }
}

fn finance_adjacent_key(key: &str, month: bool, offset: i64) -> Option<String> {
    if month {
        gridtimer_native::shift_finance_month_key(key, offset)
    } else {
        gridtimer_native::shift_finance_day_key(key, offset)
    }
}

#[derive(Clone)]
struct FinancePeriodSummary {
    reference_date: String,
    elapsed_months: i64,
    totals: gridtimer_native::FinanceLedgerTotals,
    net_worth: Option<[i64; 5]>,
}

fn finance_period_reference_date(
    year_text: &str,
    quarter: i32,
    yearly: bool,
    today: &str,
) -> Option<(i32, i32, i32, i64)> {
    if year_text.len() != 4 || !year_text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let year: i32 = year_text.parse().ok()?;
    if !(1..=9999).contains(&year) || (!yearly && !(1..=4).contains(&quarter)) {
        return None;
    }
    let first_month = if yearly { 1 } else { (quarter - 1) * 3 + 1 };
    let last_month = if yearly { 12 } else { first_month + 2 };
    let last_day = (28..=31).rev().find(|day| {
        finance_record_key_valid(
            "dailyLedgers",
            &format!("{year:04}-{last_month:02}-{day:02}"),
        )
    })?;
    let start = format!("{year:04}-{first_month:02}-01");
    let end = format!("{year:04}-{last_month:02}-{last_day:02}");
    // Match Android: current periods stop today; other selected periods use
    // their calendar end. The shared aggregate excludes entries after it.
    let reference = if finance_record_key_valid("dailyLedgers", today)
        && today >= start.as_str()
        && today <= end.as_str()
    {
        today
    } else {
        &end
    };
    let month = reference.get(5..7)?.parse::<i32>().ok()?;
    let day = reference.get(8..10)?.parse::<i32>().ok()?;
    Some((year, month, day, i64::from(month - first_month + 1)))
}

fn finance_build_period_summary(
    profile: &Value,
    year: &str,
    quarter: i32,
    yearly: bool,
    today: &str,
) -> Option<FinancePeriodSummary> {
    let (year, month, day, elapsed_months) =
        finance_period_reference_date(year, quarter, yearly, today)?;
    let raw = gridtimer_native::finance_money::project_profile_json(&profile.to_string())?;
    Some(FinancePeriodSummary {
        reference_date: format!("{year:04}-{month:02}-{day:02}"),
        elapsed_months,
        totals: gridtimer_native::aggregate_finance_ledger_values(
            &raw,
            if yearly { 3 } else { 2 },
            year,
            month,
            day,
        )?,
        net_worth: if yearly {
            gridtimer_native::year_net_worth_summary_values(&raw, year, month)
        } else {
            None
        },
    })
}

fn finance_period_cashflow(totals: &gridtimer_native::FinanceLedgerTotals) -> (i64, i64, i64) {
    let income = totals
        .active_income_total
        .saturating_add(totals.asset_income_total)
        .saturating_add(totals.other_income_total);
    let expense = [
        totals.debt_total,
        totals.food_total,
        totals.btc_total,
        totals.living_total,
        totals.learning_total,
        totals.other_expense_total,
    ]
    .into_iter()
    .fold(0_i64, i64::saturating_add);
    (income, expense, income.saturating_sub(expense))
}

fn finance_period_muted_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).color(palette().muted));
}

fn finance_period_summary_ui(
    ui: &mut egui::Ui,
    state: &mut FinanceWorkbenchState,
    profile: &Value,
    today: &str,
    revisions: (u64, u64),
) {
    let yearly = state.tab == 4;
    ui.horizontal_wrapped(|ui| {
        ui.label("年份");
        let year = state.period_year.parse::<i32>().ok();
        if ui
            .add_enabled(
                year.is_some_and(|year| (2..=9999).contains(&year)),
                egui::Button::new("上一年"),
            )
            .clicked()
        {
            state.period_year = format!("{:04}", year.unwrap() - 1);
        }
        ui.add(
            egui::TextEdit::singleline(&mut state.period_year)
                .desired_width(70.0)
                .hint_text("2026"),
        );
        if ui
            .add_enabled(
                year.is_some_and(|year| (1..9999).contains(&year)),
                egui::Button::new("下一年"),
            )
            .clicked()
        {
            state.period_year = format!("{:04}", year.unwrap() + 1);
        }
        if ui.button(if yearly { "今年" } else { "本季度" }).clicked() {
            state.period_year = today.chars().take(4).collect();
            state.quarter = today
                .get(5..7)
                .and_then(|month| month.parse::<i32>().ok())
                .map(|month| (month - 1) / 3 + 1)
                .unwrap_or(1);
        }
    });
    if !yearly {
        ui.horizontal_wrapped(|ui| {
            for quarter in 1..=4 {
                ui.selectable_value(&mut state.quarter, quarter, format!("第{quarter}季度"));
            }
        });
    }
    let cache_key = (
        revisions.0,
        revisions.1,
        state.period_year.clone(),
        state.quarter,
        yearly,
        today.to_string(),
    );
    if state.summary_cache_key.as_ref() != Some(&cache_key) {
        state.summary_cache =
            finance_build_period_summary(profile, &state.period_year, state.quarter, yearly, today);
        state.summary_cache_key = Some(cache_key);
    }
    let Some(summary) = &state.summary_cache else {
        ui.colored_label(palette().warn, "请输入 0001 至 9999 之间的四位年份");
        return;
    };
    ui.add_space(10.0);
    finance_period_muted_label(
        ui,
        &format!(
            "截至 {} · 已记录 {} 天",
            summary.reference_date, summary.totals.days_with_entries
        ),
    );
    let (income, expense, net) = finance_period_cashflow(&summary.totals);
    let accent = palette().accent;
    metric_tile_grid(
        ui,
        &[
            ("收入", money_label(income), accent),
            ("支出", money_label(expense), accent),
            ("净现金流", money_label(net), accent),
            (
                "月均净现金流",
                money_label(desktop_money_average_minor(net, summary.elapsed_months)),
                accent,
            ),
        ],
    );
    if summary.totals.days_with_entries == 0 {
        finance_period_muted_label(ui, "所选期间还没有已记录的日账");
    }
    ui.add_space(10.0);
    section_heading(ui, "支出分类");
    let values = [
        summary.totals.debt_total,
        summary.totals.food_total,
        summary.totals.btc_total,
        summary.totals.living_total,
        summary.totals.learning_total,
        summary.totals.other_expense_total,
    ];
    for ((_, label), amount) in finance_expense_options(profile).iter().zip(values) {
        ui.horizontal_wrapped(|ui| {
            ui.label(label);
            ui.label(money_label(amount));
            if income > 0 {
                finance_period_muted_label(
                    ui,
                    &format!("占收入 {:.1}%", amount as f64 * 100.0 / income as f64),
                );
            }
        });
    }
    if let Some([opening, closing, opening_month, closing_month, calendar_opening]) =
        summary.net_worth
    {
        ui.add_space(12.0);
        section_heading(ui, "净资产快照");
        for (label, amount, month) in [
            (
                if calendar_opening > 0 {
                    "年初基线"
                } else {
                    "首份快照"
                },
                opening,
                opening_month,
            ),
            ("最新快照", closing, closing_month),
        ] {
            ui.horizontal_wrapped(|ui| {
                ui.label(label);
                if month > 0 {
                    ui.label(money_label(amount));
                    finance_period_muted_label(
                        ui,
                        &format!("{:04}-{:02}", month / 100, month % 100),
                    );
                } else {
                    finance_period_muted_label(ui, "暂无快照");
                }
            });
        }
        if opening_month > 0 && closing_month > opening_month {
            ui.label(format!(
                "快照区间变化：{}",
                money_label(closing.saturating_sub(opening))
            ));
        } else {
            finance_period_muted_label(ui, "两个不同月份的快照可用于比较净资产变化");
        }
    }
}

fn finance_settings_editor(ui: &mut egui::Ui, profile: &mut Value) -> bool {
    let mut settings = profile
        .get("settings")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let defaults = gridtimer_native::default_finance_expense_category_configs_json()
        .and_then(|raw| serde_json::from_str::<Vec<Value>>(&raw).ok())
        .unwrap_or_default();
    let mut categories = settings
        .get("expenseCategories")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for default in defaults {
        if !categories
            .iter()
            .any(|category| category.get("bucket") == default.get("bucket"))
        {
            categories.push(default);
        }
    }
    let mut changed = false;
    for category in &mut categories {
        let bucket = category
            .get("bucket")
            .and_then(Value::as_str)
            .unwrap_or("OTHER")
            .to_string();
        ui.push_id((&bucket, "finance_target"), |ui| {
            ui.horizontal_wrapped(|ui| {
                let mut label = category
                    .get("label")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut label)
                            .desired_width(145.0)
                            .hint_text("分类名称"),
                    )
                    .changed()
                {
                    category["label"] = Value::String(label);
                    changed = true;
                }
                let mut has_target = category
                    .get("targetShareOfIncome")
                    .and_then(Value::as_f64)
                    .is_some();
                if ui.checkbox(&mut has_target, "目标占收入").changed() {
                    category["targetShareOfIncome"] = if has_target {
                        Value::from(0.0)
                    } else {
                        Value::Null
                    };
                    changed = true;
                }
                if has_target {
                    let mut percent = category
                        .get("targetShareOfIncome")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0)
                        * 100.0;
                    if ui
                        .add(
                            egui::DragValue::new(&mut percent)
                                .speed(0.5)
                                .clamp_range(0.0..=100.0)
                                .suffix(" %"),
                        )
                        .changed()
                    {
                        category["targetShareOfIncome"] = Value::from(percent / 100.0);
                        changed = true;
                    }
                }
                let bucket_code = match bucket.as_str() {
                    "DEBT" => 0,
                    "FOOD" => 1,
                    "BTC" => 2,
                    "LIVING" => 3,
                    "LEARNING" => 4,
                    "OTHER" => 5,
                    _ => -1,
                };
                ui.label(if finance_budget_is_minimum(bucket_code) {
                    "最低配置"
                } else if finance_budget_is_cap(bucket_code) {
                    "支出上限"
                } else {
                    "目标比例"
                });
            });
            ui.add_space(8.0);
        });
    }
    if changed {
        settings["expenseCategories"] = Value::Array(categories);
        profile["settings"] = settings;
    }
    ui.separator();
    ui.label("资产配置计划");
    changed |= finance_json_text(ui, profile, "acquisitionFocus", "计划配置的资产", true);
    ui.label("负债处理计划");
    changed |= finance_json_text(ui, profile, "liabilityFocus", "计划偿还或调整的负债", true);
    changed
}

fn finance_expense_options(profile: &Value) -> Vec<(String, String)> {
    let defaults = gridtimer_native::default_finance_expense_category_configs_json()
        .and_then(|raw| serde_json::from_str::<Vec<Value>>(&raw).ok())
        .unwrap_or_default();
    defaults
        .into_iter()
        .filter_map(|default| {
            let code = default.get("bucket")?.as_str()?.to_string();
            let custom = profile
                .pointer("/settings/expenseCategories")
                .and_then(Value::as_array)
                .and_then(|categories| {
                    categories.iter().find(|category| {
                        category.get("bucket").and_then(Value::as_str) == Some(&code)
                    })
                });
            let label = custom
                .and_then(|value| value.get("label"))
                .and_then(Value::as_str)
                .filter(|label| !label.trim().is_empty())
                .or_else(|| default.get("label").and_then(Value::as_str))
                .unwrap_or("其他")
                .to_string();
            Some((code, label))
        })
        .collect()
}

fn finance_record_key_valid(collection: &str, key: &str) -> bool {
    match collection {
        "dailyLedgers" => gridtimer_native::finance_day_ledger_or_default_json("{}", key).is_some(),
        "monthlySnapshots" => {
            gridtimer_native::finance_month_snapshot_or_default_json("{}", key).is_some()
        }
        _ => false,
    }
}

fn finance_put_record(profile: &mut Value, collection: &str, key: &str, record: Value) -> bool {
    if !finance_record_key_valid(collection, key) || !record.is_object() || !profile.is_object() {
        return false;
    }
    let object = profile.as_object_mut().expect("checked object");
    let collection = object
        .entry(collection.to_string())
        .or_insert_with(|| serde_json::json!({}));
    let Some(records) = collection.as_object_mut() else {
        return false;
    };
    records.insert(key.to_string(), record);
    true
}

/// Keep deleted rows as the Android model's tombstones. Revisions must advance
/// even when two edits occur within one millisecond or the wall clock moves back.
fn finance_touch_entry(entry: &mut Value, now: i64, deleted: bool) -> bool {
    let previous = entry
        .get("updatedAtEpochMillis")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        .max(
            entry
                .get("deletedAtEpochMillis")
                .and_then(Value::as_i64)
                .unwrap_or(0),
        );
    let Some(revision) = previous.checked_add(1).map(|next| next.max(now).max(1)) else {
        return false;
    };
    if !entry.is_object() {
        return false;
    }
    if entry
        .get("id")
        .and_then(Value::as_str)
        .is_none_or(|id| id.is_empty())
    {
        entry["id"] = Value::String(format!("finance-{:032x}", rand::random::<u128>()));
    }
    entry["updatedAtEpochMillis"] = Value::from(revision);
    entry["deletedAtEpochMillis"] = Value::from(if deleted { revision } else { 0 });
    true
}

#[cfg(test)]
include!("finance_period_tests.rs");

#[cfg(test)]
mod finance_workbench_tests {
    use super::*;

    #[test]
    fn risk_cache_refreshes_only_for_data_edits_or_local_day_change() {
        let mut state = FinanceWorkbenchState::default();
        let calls = std::cell::Cell::new(0);
        let mut build = || {
            calls.set(calls.get() + 1);
            desktop_finance_risk_snapshot(&DesktopFinanceProfile::default())
        };
        let initial = state.cached_risk((1, 0, (2026, 9, 8)), &mut build).unwrap();
        let again = state.cached_risk((1, 0, (2026, 9, 8)), &mut build).unwrap();
        assert!(Arc::ptr_eq(&initial, &again));
        assert_eq!(1, calls.get());
        let edited = state.cached_risk((1, 1, (2026, 9, 8)), &mut build).unwrap();
        assert!(!Arc::ptr_eq(&initial, &edited));
        state.cached_risk((1, 1, (2026, 9, 9)), &mut build);
        state.cached_risk((2, 1, (2026, 9, 9)), &mut build);
        assert_eq!(4, calls.get());
    }

    #[test]
    fn narrow_workbench_render_does_not_modify_records_without_input() {
        let now = 1_789_000_000_000_i64;
        let root: Value = serde_json::from_str(&app_data::default_app_data_json(now)).unwrap();
        for tab in 0..3 {
            let mut profile = root["financeProfile"].clone();
            finance_put_record(
                &mut profile,
                "dailyLedgers",
                "2026-09-08",
                serde_json::json!({"incomes":[{"id":"income", "name":"工资","kind":"ACTIVE","amount":3000}],"expenses":[],"note":"备注","confirmedAtEpochMillis":now}),
            );
            let before = profile.clone();
            let mut state = FinanceWorkbenchState {
                day_key: "2026-09-08".to_string(),
                month_key: "2026-09".to_string(),
                ..Default::default()
            };
            let context = egui::Context::default();
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(460.0, 800.0),
                )),
                ..Default::default()
            };
            let mut changed = false;
            let _ = context.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    changed = match tab {
                        0 => finance_day_editor(ui, &mut state, &mut profile, "2026-09-08", now),
                        1 => finance_month_editor(ui, &mut state, &mut profile, "2026-09-08", now),
                        _ => finance_settings_editor(ui, &mut profile),
                    };
                    assert!(
                        ui.min_rect().width() <= 460.0,
                        "finance tab {tab} overflows a narrow window"
                    );
                });
            });
            assert!(!changed);
            assert_eq!(before, profile);
        }
    }

    #[test]
    fn day_and_month_keys_reuse_shared_calendar_validation() {
        assert!(finance_record_key_valid("dailyLedgers", "2024-02-29"));
        assert!(!finance_record_key_valid("dailyLedgers", "2026-02-29"));
        assert!(!finance_record_key_valid("dailyLedgers", "2026-09-31"));
        assert!(!finance_record_key_valid("monthlySnapshots", "2026-13"));
        let mut profile = serde_json::json!({"sentinel":1});
        let before = profile.clone();
        assert!(!finance_put_record(
            &mut profile,
            "dailyLedgers",
            "bad",
            serde_json::json!({})
        ));
        assert_eq!(before, profile);
    }

    #[test]
    fn editing_one_record_preserves_other_days_months_settings_and_unknown_draft_fields() {
        let mut profile = serde_json::json!({"dailyLedgers":{"2026-09-07":{"note":"保留"}}, "monthlySnapshots":{"2026-08":{"note":"月快照"}}, "settings":{"futureSetting":true}, "futureProfile":{"keep":1}});
        assert!(finance_put_record(
            &mut profile,
            "dailyLedgers",
            "2026-09-08",
            serde_json::json!({"incomes":[],"expenses":[],"note":"新记录", "futureLedger":7})
        ));
        assert_eq!("保留", profile["dailyLedgers"]["2026-09-07"]["note"]);
        assert_eq!("月快照", profile["monthlySnapshots"]["2026-08"]["note"]);
        assert_eq!(true, profile["settings"]["futureSetting"]);
        assert_eq!(1, profile["futureProfile"]["keep"]);
        assert_eq!(7, profile["dailyLedgers"]["2026-09-08"]["futureLedger"]);
    }

    #[test]
    fn row_delete_keeps_identity_and_advances_revision_with_a_backwards_clock() {
        let mut row = serde_json::json!({"id":"row-one","name":"午餐","amount":20,"updatedAtEpochMillis":1000,"futureRow":true});
        assert!(finance_touch_entry(&mut row, 900, true));
        assert_eq!("row-one", row["id"]);
        assert_eq!(1001, row["updatedAtEpochMillis"]);
        assert_eq!(1001, row["deletedAtEpochMillis"]);
        assert_eq!(true, row["futureRow"]);
        assert!(finance_touch_entry(&mut row, 900, false));
        assert_eq!(1002, row["updatedAtEpochMillis"]);
        assert_eq!(0, row["deletedAtEpochMillis"]);
        row["updatedAtEpochMillis"] = Value::from(i64::MAX);
        let before = row.clone();
        assert!(!finance_touch_entry(&mut row, 900, true));
        assert_eq!(before, row);
    }

    #[test]
    fn shared_commit_preserves_deleted_rows_and_confirms_zero_day() {
        let now = 1_789_000_000_000_i64;
        let state = app_data::default_app_data_json(now);
        let mut root: Value = serde_json::from_str(&state).unwrap();
        let mut profile = root["financeProfile"].clone();
        let mut deleted = serde_json::json!({"id":"deleted-income","name":"工资","kind":"ACTIVE","amount":5000,"note":"","updatedAtEpochMillis":1});
        assert!(finance_touch_entry(&mut deleted, now, true));
        assert!(finance_put_record(
            &mut profile,
            "dailyLedgers",
            "2026-09-08",
            serde_json::json!({"incomes":[deleted],"expenses":[],"note":"","confirmedAtEpochMillis":now})
        ));
        let committed =
            app_data::update_finance_profile_app_data_json(&state, &profile.to_string(), now)
                .unwrap();
        root = serde_json::from_str(&committed).unwrap();
        let ledger = &root["financeProfile"]["dailyLedgers"]["2026-09-08"];
        assert_eq!(1, ledger["incomes"].as_array().unwrap().len());
        assert_eq!(now, ledger["incomes"][0]["deletedAtEpochMillis"]);
        assert_eq!(now, ledger["confirmedAtEpochMillis"]);
        assert!(
            root["financeDayLedgerRevisions"]["2026-09-08"]
                .as_i64()
                .unwrap()
                > 0
        );
    }

    #[test]
    fn shared_commit_retains_month_snapshot_and_custom_budget_target() {
        let now = 1_789_000_000_000_i64;
        let state = app_data::default_app_data_json(now);
        let root: Value = serde_json::from_str(&state).unwrap();
        let mut profile = root["financeProfile"].clone();
        assert!(finance_put_record(
            &mut profile,
            "monthlySnapshots",
            "2026-09",
            serde_json::json!({"assets":[{"id":"cash","name":"银行卡","kind":"CASH_RESERVE","amount":6000,"updatedAtEpochMillis":now}],"liabilities":[],"note":"已核对","confirmedAtEpochMillis":now})
        ));
        profile["settings"]["expenseCategories"][0]["label"] = Value::from("债务");
        profile["settings"]["expenseCategories"][0]["targetShareOfIncome"] = Value::from(0.2);
        let committed =
            app_data::update_finance_profile_app_data_json(&state, &profile.to_string(), now)
                .unwrap();
        let value: Value = serde_json::from_str(&committed).unwrap();
        assert_eq!(
            6000,
            value["financeProfile"]["monthlySnapshots"]["2026-09"]["assets"][0]["amount"]
        );
        let ratio = value["financeProfile"]["settings"]["expenseCategories"][0]
            ["targetShareOfIncome"]
            .as_f64()
            .unwrap();
        assert!((ratio - 0.2).abs() < 0.000001);
        assert_eq!(
            "债务",
            value["financeProfile"]["settings"]["expenseCategories"][0]["label"]
        );
    }
}
