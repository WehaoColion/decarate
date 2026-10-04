// v1.1.0.5 Windows - Calculate finance overviews from a transient integer-cent projection.
// v1.0.3 Windows - Decode overview input once and cache the trend category label.
// v2.22.48 - Use the Android risk overview page names.
// v2.22.47 - Use Android's period, risk, health and trend calculations on Windows.

include!("finance_risk_v2.rs");
include!("legal_risk.rs");

struct FinanceOverviewState {
    period: i32,
    page: usize,
    date: String,
    month: String,
    year: String,
    quarter: i32,
    cache_key: Option<(u64, u64, i32, String)>,
    cache: Option<Arc<FinanceOverviewSnapshot>>,
}

impl Default for FinanceOverviewState {
    fn default() -> Self {
        Self {
            period: 1,
            page: 0,
            date: String::new(),
            month: String::new(),
            year: String::new(),
            quarter: 1,
            cache_key: None,
            cache: None,
        }
    }
}

struct FinanceOverviewSnapshot {
    report: gridtimer_native::FinanceSnapshotValues,
    trend: gridtimer_native::FinanceTrendValues,
    health: Option<gridtimer_native::FinanceHealthScoreValues>,
    risk: DesktopFinanceRiskSnapshot,
    latest_month: Option<String>,
    off_target_bucket_label: Option<String>,
}

fn finance_overview_reference(state: &FinanceOverviewState, today: &str) -> Option<String> {
    match state.period {
        0 => finance_record_key_valid("dailyLedgers", &state.date).then(|| state.date.clone()),
        1 => {
            if !finance_record_key_valid("monthlySnapshots", &state.month) {
                return None;
            }
            if today.get(..7) == Some(state.month.as_str()) {
                return Some(today.into());
            }
            (28..=31)
                .rev()
                .map(|d| format!("{}-{d:02}", state.month))
                .find(|s| finance_record_key_valid("dailyLedgers", s))
        }
        2 | 3 => {
            finance_period_reference_date(&state.year, state.quarter, state.period == 3, today)
                .map(|(y, m, d, _)| format!("{y:04}-{m:02}-{d:02}"))
        }
        _ => None,
    }
}

fn build_finance_overview(
    raw: &str,
    period: i32,
    reference: &str,
) -> Option<FinanceOverviewSnapshot> {
    if !(0..=3).contains(&period) || !finance_record_key_valid("dailyLedgers", reference) {
        return None;
    }
    let y = reference.get(..4)?.parse().ok()?;
    let m = reference.get(5..7)?.parse().ok()?;
    let d = reference.get(8..10)?.parse().ok()?;
    let raw = gridtimer_native::finance_money::project_profile_json(raw)?;
    let values = gridtimer_native::build_desktop_finance_overview_values(&raw, period, y, m, d)?;
    Some(FinanceOverviewSnapshot {
        report: values.report,
        trend: values.trend,
        health: values.health,
        risk: serde_json::from_str(&values.risk_json).ok()?,
        latest_month: values.latest_month,
        off_target_bucket_label: None,
    })
}

fn finance_overview_expense_label(settings: Option<&Value>, bucket_code: i32) -> Option<String> {
    let index = usize::try_from(bucket_code).ok()?;
    // Only six category settings are needed. Never copy the ledger profile to
    // resolve a label, and keep the editor's raw/custom-label fallback rules.
    let profile = serde_json::json!({ "settings": settings });
    finance_expense_options(&profile)
        .get(index)
        .map(|(_, label)| label.clone())
}

impl TimerWindowsClient {
    fn open_finance_risk_action(&mut self, kind: i32, reference: &str, anomaly: &str) {
        // The destination and selected period match Android's openRiskAction.
        match kind {
            2 | 3 | 6 => {
                self.finance_workbench.tab = 1;
                self.finance_workbench.month_key = reference.chars().take(7).collect();
            }
            7 => {
                self.finance_workbench.tab = 0;
                self.finance_workbench.day_key =
                    if finance_record_key_valid("dailyLedgers", anomaly) {
                        anomaly.into()
                    } else {
                        reference.into()
                    };
            }
            _ => {
                self.finance_workbench.tab = 0;
                self.finance_workbench.day_key = reference.into();
            }
        }
        self.finance_workbench.pending_delete = None;
    }

    fn ui_finance_android_overview_legacy(&mut self, ui: &mut egui::Ui) {
        let today = desktop_local_timestamp(now_millis())
            .chars()
            .take(10)
            .collect::<String>();
        let finance_version = self.finance_cache_version();
        let state = &mut self.finance_workbench.overview;
        if state.date.is_empty() {
            state.date = today.clone();
            state.month = today.chars().take(7).collect();
            state.year = today.chars().take(4).collect();
            state.quarter = (local_ymd_now().1 - 1) / 3 + 1;
        }
        ui.horizontal_wrapped(|ui| {
            for (code, label) in [(0, "日"), (1, "月"), (2, "季"), (3, "年")] {
                desktop_segment(ui, &mut state.period, code, label);
            }
            match state.period {
                0 => {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.date)
                            .desired_width(110.0)
                            .hint_text("YYYY-MM-DD"),
                    );
                }
                1 => {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.month)
                            .desired_width(90.0)
                            .hint_text("YYYY-MM"),
                    );
                }
                _ => {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.year)
                            .desired_width(65.0)
                            .hint_text("YYYY"),
                    );
                    if state.period == 2 {
                        for q in 1..=4 {
                            ui.selectable_value(&mut state.quarter, q, format!("第{q}季"));
                        }
                    }
                }
            }
            if ui.button("当前期间").clicked() {
                state.date = today.clone();
                state.month = today.chars().take(7).collect();
                state.year = today.chars().take(4).collect();
                state.quarter = (local_ymd_now().1 - 1) / 3 + 1;
            }
        });
        let Some(reference) = finance_overview_reference(state, &today) else {
            ui.colored_label(palette().warn, "请输入有效日期");
            return;
        };
        let key = (
            finance_version,
            self.persistence.revisions.finance,
            state.period,
            reference.clone(),
        );
        if state.cache_key.as_ref() != Some(&key) {
            state.cache = serde_json::to_string(&self.finance_draft)
                .ok()
                .and_then(|raw| build_finance_overview(&raw, state.period, &reference))
                .map(|mut snapshot| {
                    snapshot.off_target_bucket_label = finance_overview_expense_label(
                        self.finance_draft.extra.get("settings"),
                        snapshot.trend.most_off_target_bucket_code,
                    );
                    snapshot
                })
                .map(Arc::new);
            state.cache_key = Some(key);
        }
        let Some(snapshot) = state.cache.clone() else {
            empty_state(ui, "风控数据暂不可用");
            return;
        };
        ui.horizontal_wrapped(|ui| {
            for (page, label) in [
                "驾驶舱",
                "预算余量",
                "预测与扫描",
                "风控体检",
                "资产负债快照",
                "趋势与提醒",
            ]
            .into_iter()
            .enumerate()
            {
                desktop_segment(ui, &mut state.page, page, label);
            }
        });
        ui.label(
            egui::RichText::new(format!(
                "截至 {reference} · 已记录 {} 天",
                snapshot.report.recorded_days
            ))
            .size(12.0)
            .color(palette().muted),
        );
        ui.add_space(10.0);
        let page = state.page;
        let risk = &snapshot.risk;
        let mut action = None;
        match page {
            0 => {
                action = self.ui_finance_risk_cockpit(ui, risk);
            }
            1 => {
                section_heading(ui, "分类预算");
                if !finance_has_cashflow_evidence(risk) {
                    ui.label("尚无现金流基线");
                } else {
                    for item in &risk.budget_envelopes {
                        card_frame().show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.strong(&item.label);
                                ui.label(finance_budget_target_text(item));
                                ui.label(finance_budget_remaining_text(item));
                            });
                            if item.target_amount > 0 {
                                ui.add(egui::ProgressBar::new(
                                    (item.actual_amount as f64 / item.target_amount as f64)
                                        .clamp(0.0, 1.0) as f32,
                                ));
                            }
                        });
                        ui.add_space(6.0);
                    }
                }
                if ui.button("编辑分类与目标").clicked() {
                    self.finance_workbench.tab = 2;
                }
            }
            2 => {
                section_heading(ui, "现金流预测");
                if risk.forecast_available {
                    finance_detail_row(ui, "月净流预测", &money_label(risk.projected_monthly_net));
                    if risk.has_fresh_snapshot {
                        for (label, value) in [
                            ("30 日余额", risk.projected_balance30_days),
                            ("90 日余额", risk.projected_balance90_days),
                            ("压力情景余额", risk.stressed_balance30_days),
                        ] {
                            finance_detail_row(ui, label, &money_label(value));
                        }
                    } else {
                        ui.label("补充所选月份的资产快照后可查看余额预测");
                    }
                } else {
                    ui.label("已记录天数或覆盖率不足，暂不预测");
                }
                ui.add_space(12.0);
                section_heading(ui, "应急与负债");
                if risk.has_fresh_snapshot {
                    finance_detail_row(ui, "应急缺口", &money_label(risk.emergency_gap));
                    finance_detail_row(ui, "负债余额", &money_label(risk.debt_balance));
                    finance_detail_row(
                        ui,
                        "预计还清",
                        &risk
                            .debt_free_months
                            .map(|v| format!("{v:.1} 月"))
                            .unwrap_or_else(|| "暂无可计算的偿还计划".into()),
                    );
                } else {
                    ui.label("缺少所选月份的资产负债快照");
                }
                ui.add_space(12.0);
                section_heading(ui, "异常与重复支出");
                if risk.anomaly_amount > 0 {
                    ui.label(format!(
                        "{} · {} · 基线的 {:.1} 倍",
                        risk.anomaly_day_key,
                        money_label(risk.anomaly_amount),
                        risk.anomaly_ratio
                    ));
                    if ui.button("查看异常日账").clicked() {
                        action = Some(7);
                    }
                } else {
                    ui.label("未发现异常支出");
                }
                for item in &risk.recurring_candidates {
                    ui.label(format!(
                        "{} · {} 次 · 均额 {} · 月估算 {}",
                        item.name,
                        item.occurrences,
                        money_label(item.average_amount),
                        money_label(item.monthly_estimate)
                    ));
                }
                if risk.recurring_candidates.is_empty() {
                    ui.label("未发现重复支出");
                }
            }
            3 => {
                if let Some(h) = snapshot.health {
                    ui.heading(format!("{} / 100", h.score));
                    ui.label(format!("{} 项提醒", h.warning_count));
                    for (label, score) in [
                        ("现金流", h.cashflow_score),
                        ("防御储备", h.defensive_score),
                        ("负债压力", h.liability_score),
                        ("被动收入", h.passive_score),
                        ("工资依赖", h.wage_score),
                        ("记录覆盖", h.record_score),
                        ("预算纪律", h.discipline_score),
                        ("改善趋势", h.momentum_score),
                    ] {
                        finance_detail_row(ui, label, &format!("{:.0} / 100", score * 100.0));
                    }
                } else {
                    empty_state(ui, "尚无现金流基线，暂不评分");
                }
            }
            4 => {
                let s = &snapshot.report;
                metric_tile_grid(
                    ui,
                    &[
                        ("收入", money_label(s.total_income), palette().good),
                        ("支出", money_label(s.total_outflow), palette().warn),
                        ("净现金流", money_label(s.net_cashflow), palette().accent),
                        ("收入缺口", money_label(s.freedom_gap), palette().blue),
                    ],
                );
                ui.add_space(12.0);
                if let Some(month) = &snapshot.latest_month {
                    ui.label(format!("资产快照 {month}"));
                    finance_detail_row(ui, "净资产", &money_label(s.net_worth));
                    finance_detail_row(
                        ui,
                        "防御覆盖",
                        &s.defensive_coverage
                            .map(|v| format!("{v:.1} 月"))
                            .unwrap_or_else(|| "暂无支出基线".into()),
                    );
                } else {
                    ui.label("尚无资产负债快照");
                }
                if finance_has_cashflow_evidence(risk) {
                    for (label, value) in [
                        ("被动收入覆盖", s.passive_coverage_ratio),
                        ("工资依赖", s.wage_dependence_ratio),
                        ("负债支付压力", s.liability_pressure_ratio),
                    ] {
                        finance_detail_row(ui, label, &format!("{:.1}%", value * 100.0));
                    }
                }
            }
            _ => {
                let t = snapshot.trend;
                section_heading(ui, "与上期比较");
                ui.label(format!(
                    "本期记录 {} 天 · 上期 {} 天",
                    t.current_recorded_days, t.previous_recorded_days
                ));
                if t.cashflow_comparison_available {
                    for (label, value) in [
                        ("收入变化", t.income_delta),
                        ("支出变化", t.outflow_delta),
                        ("净现金流变化", t.net_cashflow_delta),
                    ] {
                        finance_detail_row(ui, label, &desktop_signed_money_label(value));
                    }
                } else {
                    ui.label("两期现金流记录不足，暂不比较");
                }
                if t.net_worth_comparison_available {
                    finance_detail_row(
                        ui,
                        "净资产变化",
                        &desktop_signed_money_label(t.net_worth_delta),
                    );
                } else {
                    ui.label("缺少可比较的两期资产快照");
                }
                if let Some(label) = &snapshot.off_target_bucket_label {
                    ui.label(format!(
                        "{label}偏离目标：{:+.1} 个百分点",
                        t.most_off_target_ratio_delta * 100.0
                    ));
                }
            }
        }
        if let Some(kind) = action {
            self.open_finance_risk_action(kind, &reference, &risk.anomaly_day_key);
        }
    }
}
