// v1.1.0.5 Windows - Bind finance calculations and review receipts to exact cents.
// Windows finance reviews use the same Rust policy as Android. The review
// receipt is local to an account workspace and never enters AppData or sync.

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopFinanceReviewEnvelope {
    version: u32,
    scope: String,
    receipts: Vec<gridtimer_native::FinanceReviewReceipt>,
}

#[derive(Clone, Debug)]
struct DesktopFinanceReviewState {
    scope: String,
    receipts: Vec<gridtimer_native::FinanceReviewReceipt>,
    revision: u64,
    load_error: Option<String>,
}

impl DesktopFinanceReviewState {
    fn load(state_path: &Path, sync: &DesktopSyncSession) -> Self {
        let scope = desktop_finance_review_scope(state_path, sync);
        let mut result = Self {
            scope: scope.clone(),
            receipts: Vec::new(),
            revision: 0,
            load_error: None,
        };
        let path = desktop_finance_review_path(state_path);
        if !path.exists() {
            return result;
        }
        match read_desktop_finance_review(&path, &scope) {
            Ok(receipts) => result.receipts = receipts,
            Err(error) => result.load_error = Some(error.to_string()),
        }
        result
    }

    fn replace(
        &mut self,
        state_path: &Path,
        receipts: Vec<gridtimer_native::FinanceReviewReceipt>,
    ) -> io::Result<()> {
        if self.load_error.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "核对记录读取失败，请先恢复原文件",
            ));
        }
        validate_desktop_finance_receipts(&receipts)?;
        let path = desktop_finance_review_path(state_path);
        let envelope = DesktopFinanceReviewEnvelope {
            version: 1,
            scope: self.scope.clone(),
            receipts: receipts.clone(),
        };
        let encoded = serde_json::to_vec(&envelope).map_err(io::Error::other)?;
        let protected = protect_secret_bytes(&encoded)?;
        atomic_replace_bytes_no_backup(&path, &protected)?;
        self.receipts = receipts;
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }
}

fn desktop_finance_review_path(state_path: &Path) -> PathBuf {
    state_path.with_file_name("finance_review.secrets")
}

fn desktop_finance_review_scope(state_path: &Path, sync: &DesktopSyncSession) -> String {
    let checkpoint = workspace_checkpoint_for_account(
        sync,
        &sync.server_instance_id,
        &sync.account_namespace,
        &sync.user_id,
    );
    format!(
        "{}\0{}\0{}\0{}\0{}",
        state_path.to_string_lossy(),
        sync.server_instance_id,
        sync.account_namespace,
        sync.user_id,
        checkpoint.workspace_id
    )
}

fn read_desktop_finance_review(
    path: &Path,
    expected_scope: &str,
) -> io::Result<Vec<gridtimer_native::FinanceReviewReceipt>> {
    let raw = fs::read(path)?;
    #[cfg(target_os = "windows")]
    let body = raw
        .strip_prefix(DPAPI_SECRET_HEADER)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "财务核对文件未加密"))?;
    #[cfg(not(target_os = "windows"))]
    let body = raw
        .strip_prefix(PLAIN_SECRET_HEADER)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "财务核对文件格式无效"))?;
    let plain = unprotect_secret_bytes(body)?;
    let envelope: DesktopFinanceReviewEnvelope = serde_json::from_slice(&plain)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if envelope.version != 1 || envelope.scope != expected_scope {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "财务核对记录不属于当前工作区",
        ));
    }
    validate_desktop_finance_receipts(&envelope.receipts)?;
    Ok(envelope.receipts)
}

fn validate_desktop_finance_receipts(
    receipts: &[gridtimer_native::FinanceReviewReceipt],
) -> io::Result<()> {
    if receipts.len() > 240 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "财务核对月份过多",
        ));
    }
    let mut months = std::collections::HashSet::new();
    for receipt in receipts {
        if !finance_record_key_valid("monthlySnapshots", &receipt.month_key)
            || !months.insert(&receipt.month_key)
            || receipt.confirmed_recurring_keys.len() > 512
            || receipt
                .confirmed_recurring_keys
                .iter()
                .any(|key| key.is_empty() || key.len() > 256)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "财务核对记录内容无效",
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DesktopFinanceReviewLane {
    IncomeExpense,
    Cash,
    Assets,
    Liabilities,
}

#[derive(Clone, Debug)]
enum DesktopFinanceReviewAction {
    Confirm(DesktopFinanceReviewLane, bool),
    ToggleRecurring(String),
    ClearMonth,
    RetryLoad,
    OpenDay(String),
}

fn desktop_finance_risk_v2(
    profile: &DesktopFinanceProfile,
    receipts: &[gridtimer_native::FinanceReviewReceipt],
    reference: &str,
) -> Option<gridtimer_native::AndroidFinanceRiskV2> {
    let year = reference.get(..4)?.parse().ok()?;
    let month = reference.get(5..7)?.parse().ok()?;
    let day = reference.get(8..10)?.parse().ok()?;
    let profile_json = gridtimer_native::finance_money::project_profile_json(
        &serde_json::to_string(profile).ok()?,
    )?;
    let receipts_json = serde_json::to_string(receipts).ok()?;
    let raw = gridtimer_native::build_desktop_finance_risk_v2_json(
        &profile_json,
        &receipts_json,
        year,
        month,
        day,
    )?;
    serde_json::from_str(&raw).ok()
}

fn desktop_finance_fingerprints(
    profile: &DesktopFinanceProfile,
    month_key: &str,
) -> Option<gridtimer_native::FinanceReviewFingerprints> {
    let raw = gridtimer_native::build_desktop_finance_review_fingerprints_json(
        &gridtimer_native::finance_money::project_profile_json(
            &serde_json::to_string(profile).ok()?,
        )?,
        month_key,
    )?;
    serde_json::from_str(&raw).ok()
}

fn desktop_finance_review_receipt_mut<'a>(
    receipts: &'a mut Vec<gridtimer_native::FinanceReviewReceipt>,
    month_key: &str,
) -> &'a mut gridtimer_native::FinanceReviewReceipt {
    if !receipts
        .iter()
        .any(|receipt| receipt.month_key == month_key)
    {
        receipts.push(gridtimer_native::FinanceReviewReceipt {
            month_key: month_key.to_string(),
            ..Default::default()
        });
    }
    receipts
        .iter_mut()
        .find(|receipt| receipt.month_key == month_key)
        .expect("month receipt inserted")
}

fn desktop_finance_confirm_receipt(
    receipts: &mut Vec<gridtimer_native::FinanceReviewReceipt>,
    fingerprints: &gridtimer_native::FinanceReviewFingerprints,
    risk: &gridtimer_native::AndroidFinanceRiskV2,
    reference: &str,
    today: &str,
    lane: DesktopFinanceReviewLane,
    zero_confirmed: bool,
) -> io::Result<()> {
    let month_key = &fingerprints.month_key;
    if month_key != &risk.month_key || reference.get(..7) != Some(month_key.as_str()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "月份已变化，请重新核对",
        ));
    }
    let has_record = match lane {
        DesktopFinanceReviewLane::IncomeExpense => risk.recorded_days > 0,
        DesktopFinanceReviewLane::Cash => risk.recorded_cash.is_some(),
        DesktopFinanceReviewLane::Assets => risk.recorded_assets.is_some(),
        DesktopFinanceReviewLane::Liabilities => risk.recorded_liabilities.is_some(),
    };
    if has_record == zero_confirmed {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "请按已录资料选择核对或确认零值",
        ));
    }
    let receipt = desktop_finance_review_receipt_mut(receipts, month_key);
    match lane {
        DesktopFinanceReviewLane::IncomeExpense => {
            let fingerprint = if month_key.as_str() < today.get(..7).unwrap_or("") {
                if zero_confirmed {
                    format!("zero-full:{}", fingerprints.income_expense_fingerprint)
                } else {
                    format!("full:{}", fingerprints.income_expense_fingerprint)
                }
            } else {
                format!(
                    "through:{}:{}",
                    &reference[8..10].parse::<i32>().unwrap_or(0),
                    fingerprints.income_expense_fingerprint
                )
            };
            if receipt.income_expense_fingerprint != fingerprint {
                receipt.confirmed_recurring_keys.clear();
            }
            receipt.income_expense_fingerprint = fingerprint;
            receipt.income_expense_verified = true;
        }
        DesktopFinanceReviewLane::Cash => {
            receipt.cash_fingerprint = fingerprints.cash_fingerprint.clone();
            receipt.cash_verified = true;
            receipt.zero_cash_confirmed = zero_confirmed;
        }
        DesktopFinanceReviewLane::Assets => {
            receipt.assets_fingerprint = fingerprints.assets_fingerprint.clone();
            receipt.assets_verified = true;
            receipt.zero_assets_confirmed = zero_confirmed;
        }
        DesktopFinanceReviewLane::Liabilities => {
            receipt.liabilities_fingerprint = fingerprints.liabilities_fingerprint.clone();
            receipt.liabilities_verified = true;
            receipt.zero_liabilities_confirmed = zero_confirmed;
        }
    }
    Ok(())
}

fn desktop_finance_risk_state_label(state: gridtimer_native::FinanceRiskState) -> &'static str {
    match state {
        gridtimer_native::FinanceRiskState::Unknown => "待核对",
        gridtimer_native::FinanceRiskState::Stable => "平稳",
        gridtimer_native::FinanceRiskState::Watch => "留意",
        gridtimer_native::FinanceRiskState::Tight => "资金偏紧",
        gridtimer_native::FinanceRiskState::Critical => "风险较高",
    }
}

fn desktop_finance_risk_reason(reason: &str) -> &'static str {
    match reason {
        "THREE_REVIEWED_MONTHS_REQUIRED" => "先核对最近六个月中的三个完整月份，才能建立预测基线。",
        "CURRENT_REVIEW_INCOMPLETE" => "本月收支、现金、资产和负债尚未核对齐全。",
        "VERIFIED_NEGATIVE_NET_WORTH" => "已核对负债超过资产。",
        "VERIFIED_CASH_GAP" => "已核对现金覆盖出现缺口。",
        "VERIFIED_90_DAY_GAP" => "按已核对基线推算，九十天内可能出现缺口。",
        "VERIFIED_BASELINE_STABLE" => "当前已核对资料支持平稳判断。",
        _ => "资料不足，暂不能形成健康结论。",
    }
}

fn desktop_finance_known_money(value: Option<i64>, unknown: &str) -> String {
    value
        .map(money_label)
        .unwrap_or_else(|| unknown.to_string())
}

fn desktop_finance_review_row(
    ui: &mut egui::Ui,
    label: &str,
    recorded: bool,
    verified: bool,
    lane: DesktopFinanceReviewLane,
    review_text: &str,
    zero_text: &str,
    action: &mut Option<DesktopFinanceReviewAction>,
) {
    ui.horizontal_wrapped(|ui| {
        ui.strong(label);
        ui.label(if verified {
            "已核对"
        } else if recorded {
            "未核对"
        } else {
            "未记录"
        });
        if recorded && ui.small_button(review_text).clicked() {
            *action = Some(DesktopFinanceReviewAction::Confirm(lane, false));
        }
        if !recorded && ui.small_button(zero_text).clicked() {
            *action = Some(DesktopFinanceReviewAction::Confirm(lane, true));
        }
    });
}

impl TimerWindowsClient {
    fn ui_finance_android_overview(&mut self, ui: &mut egui::Ui) {
        self.ui_finance_risk_v2(ui);
    }

    fn ensure_desktop_finance_review(&mut self) {
        let scope = desktop_finance_review_scope(&self.state_path, &self.sync);
        if self
            .finance_workbench
            .review
            .as_ref()
            .is_none_or(|review| review.scope != scope)
        {
            self.finance_workbench.review = Some(DesktopFinanceReviewState::load(
                &self.state_path,
                &self.sync,
            ));
            self.finance_workbench.risk_v2_cache_key = None;
            self.finance_workbench.risk_v2_cache = None;
        }
    }

    fn desktop_finance_risk_v2_cached(
        &mut self,
        reference: &str,
    ) -> Option<Arc<gridtimer_native::AndroidFinanceRiskV2>> {
        self.ensure_desktop_finance_review();
        let revision = self.finance_workbench.review.as_ref()?.revision;
        let key = (
            self.finance_cache_version(),
            self.persistence.revisions.finance,
            revision,
            reference.to_string(),
        );
        if self.finance_workbench.risk_v2_cache_key.as_ref() != Some(&key) {
            let receipts = &self.finance_workbench.review.as_ref()?.receipts;
            self.finance_workbench.risk_v2_cache =
                desktop_finance_risk_v2(&self.finance_draft, receipts, reference).map(Arc::new);
            self.finance_workbench.risk_v2_cache_key = Some(key);
        }
        self.finance_workbench.risk_v2_cache.clone()
    }

    fn desktop_finance_apply_review_action(
        &mut self,
        action: DesktopFinanceReviewAction,
        reference: &str,
        today: &str,
    ) {
        if let DesktopFinanceReviewAction::OpenDay(day) = action {
            self.open_finance_risk_action(7, reference, &day);
            return;
        }
        if matches!(action, DesktopFinanceReviewAction::RetryLoad) {
            self.finance_workbench.review = Some(DesktopFinanceReviewState::load(
                &self.state_path,
                &self.sync,
            ));
            self.finance_workbench.risk_v2_cache_key = None;
            return;
        }
        if !self.workspace_persistence_ready {
            self.status = "本地工作区只读，暂不能保存核对记录".into();
            return;
        }
        if let Err(error) = self.flush_finance_draft() {
            self.status = format!("账目未保存，无法核对：{error}");
            return;
        }
        let Some(risk) = self.desktop_finance_risk_v2_cached(reference) else {
            self.status = "风控资料暂时无法读取".into();
            return;
        };
        let Some(fingerprints) = desktop_finance_fingerprints(&self.finance_draft, &risk.month_key)
        else {
            self.status = "无法生成本月资料指纹".into();
            return;
        };
        let Some(review) = self.finance_workbench.review.as_mut() else {
            return;
        };
        let mut receipts = review.receipts.clone();
        let outcome = match action {
            DesktopFinanceReviewAction::Confirm(lane, zero) => desktop_finance_confirm_receipt(
                &mut receipts,
                &fingerprints,
                &risk,
                reference,
                today,
                lane,
                zero,
            ),
            DesktopFinanceReviewAction::ToggleRecurring(key) => {
                if !risk.income_expense_verified
                    || !risk.recurring_expenses.iter().any(|item| item.key == key)
                {
                    Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "周期项已变化，请重新核对",
                    ))
                } else {
                    let receipt =
                        desktop_finance_review_receipt_mut(&mut receipts, &risk.month_key);
                    if let Some(index) = receipt
                        .confirmed_recurring_keys
                        .iter()
                        .position(|item| item == &key)
                    {
                        receipt.confirmed_recurring_keys.remove(index);
                    } else {
                        receipt.confirmed_recurring_keys.push(key);
                    }
                    Ok(())
                }
            }
            DesktopFinanceReviewAction::ClearMonth => {
                receipts.retain(|receipt| receipt.month_key != risk.month_key);
                Ok(())
            }
            DesktopFinanceReviewAction::RetryLoad | DesktopFinanceReviewAction::OpenDay(_) => {
                unreachable!()
            }
        };
        if let Err(error) = outcome.and_then(|_| review.replace(&self.state_path, receipts)) {
            self.status = format!("财务核对未保存：{error}");
        } else {
            self.finance_workbench.risk_v2_cache_key = None;
            self.status = "财务核对已保存到本机".into();
        }
    }

    fn ui_finance_risk_v2(&mut self, ui: &mut egui::Ui) {
        let money_inputs_ready = self.finance_money_inputs_ready();
        let today = desktop_local_timestamp(now_millis())
            .chars()
            .take(10)
            .collect::<String>();
        let current_month = today.get(..7).unwrap_or("").to_string();
        let mut open_legal = false;
        let month = {
            let state = &mut self.finance_workbench.overview;
            if state.month.is_empty() {
                state.month = current_month.clone();
            }
            card_frame().show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    section_heading(ui, "财务风控");
                    if ui.small_button("法律风险线索").clicked() {
                        open_legal = true;
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    let previous = finance_adjacent_key(&state.month, true, -1);
                    if ui
                        .add_enabled(previous.is_some(), egui::Button::new("上个月"))
                        .clicked()
                    {
                        state.month = previous.expect("previous month enabled");
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut state.month)
                            .desired_width(85.0)
                            .hint_text("YYYY-MM"),
                    );
                    let next = finance_adjacent_key(&state.month, true, 1)
                        .filter(|candidate| candidate <= &current_month);
                    if ui
                        .add_enabled(next.is_some(), egui::Button::new("下个月"))
                        .clicked()
                    {
                        state.month = next.expect("next month enabled");
                    }
                    if ui.small_button("本月").clicked() {
                        state.month = current_month.clone();
                    }
                });
            });
            state.month.clone()
        };
        if ui
            .add_sized(
                [ui.available_width(), 42.0],
                egui::Button::new("开始 AI 分析").fill(palette().accent),
            )
            .on_hover_text("核对扫描范围与接收方后，再确认发送")
            .clicked()
        {
            open_legal = true;
        }
        if open_legal {
            self.desktop_ui.legal_risk.open = true;
            self.finance_workbench.tab = 7;
            return;
        }
        if !finance_record_key_valid("monthlySnapshots", &month) || month > current_month {
            ui.colored_label(palette().warn, "请输入不晚于本月的有效月份");
            return;
        }
        let reference = if month == current_month {
            today.clone()
        } else {
            (28..=31)
                .rev()
                .map(|day| format!("{month}-{day:02}"))
                .find(|key| finance_record_key_valid("dailyLedgers", key))
                .unwrap_or_default()
        };
        let Some(risk) = self.desktop_finance_risk_v2_cached(&reference) else {
            empty_state(ui, "风控资料暂时无法读取");
            return;
        };
        let load_error = self
            .finance_workbench
            .review
            .as_ref()
            .and_then(|review| review.load_error.as_deref());
        let mut action = None;
        if let Some(error) = load_error {
            ui.colored_label(palette().warn, format!("本机核对记录读取失败：{error}"));
            if ui.small_button("重试读取").clicked() {
                action = Some(DesktopFinanceReviewAction::RetryLoad);
            }
        }
        if self.finance_dirty {
            ui.label("账目尚未保存；点击核对时会先保存账目。");
        }
        ui.add_space(8.0);
        card_frame().show(ui, |ui| {
            section_heading(ui, "已录事实");
            let recorded = risk.recorded_days > 0;
            finance_detail_row(
                ui,
                "收入",
                &if recorded {
                    money_label(risk.recorded_income)
                } else {
                    "未记录".into()
                },
            );
            finance_detail_row(
                ui,
                "流出",
                &if recorded {
                    money_label(risk.recorded_outflow)
                } else {
                    "未记录".into()
                },
            );
            finance_detail_row(
                ui,
                "净现金流",
                &if recorded {
                    money_label(risk.recorded_net_cashflow)
                } else {
                    "未记录".into()
                },
            );
            finance_detail_row(
                ui,
                "记账天数",
                &if recorded {
                    format!("{} 天", risk.recorded_days)
                } else {
                    "未记录".into()
                },
            );
            finance_detail_row(
                ui,
                "现金余额",
                &desktop_finance_known_money(risk.recorded_cash, "未记录"),
            );
            finance_detail_row(
                ui,
                "资产",
                &desktop_finance_known_money(risk.recorded_assets, "未记录"),
            );
            finance_detail_row(
                ui,
                "负债",
                &desktop_finance_known_money(risk.recorded_liabilities, "未知"),
            );
        });
        ui.add_space(8.0);
        card_frame().show(ui, |ui| {
            section_heading(ui, "经核对的判断");
            finance_detail_row(
                ui,
                "健康结论",
                desktop_finance_risk_state_label(risk.risk_state),
            );
            ui.label(desktop_finance_risk_reason(&risk.reason_code));
            let safe = match risk.safe_to_spend {
                None => "待核对".into(),
                Some(value) if value < 0 => format!("缺口 {}", money_label(value.saturating_abs())),
                Some(value) => money_label(value),
            };
            finance_detail_row(ui, "安全可支配", &safe);
            finance_detail_row(
                ui,
                "30 天余额",
                &desktop_finance_known_money(risk.projected_balance30_days, "待核对"),
            );
            finance_detail_row(
                ui,
                "90 天余额",
                &desktop_finance_known_money(risk.projected_balance90_days, "待核对"),
            );
            finance_detail_row(
                ui,
                "待付必要项",
                &desktop_finance_known_money(risk.pending_essential_reserve, "待核对"),
            );
            finance_detail_row(
                ui,
                "保护配置",
                &desktop_finance_known_money(risk.protected_allocation, "待核对"),
            );
            ui.label(if risk.forecast_available {
                format!("预测基线：{}", risk.baseline_months.join("、"))
            } else {
                format!(
                    "预测基线：还需核对 {} 个完整月份",
                    3_usize.saturating_sub(risk.baseline_months.len())
                )
            });
            if risk.forecast_available {
                finance_detail_row(
                    ui,
                    "月收入基线",
                    &desktop_finance_known_money(risk.monthly_income_baseline, "待核对"),
                );
                finance_detail_row(
                    ui,
                    "月流出基线",
                    &desktop_finance_known_money(risk.monthly_outflow_baseline, "待核对"),
                );
            }
        });
        ui.add_space(8.0);
        card_frame().show(ui, |ui| {
            section_heading(ui, "核对所选月份");
            ui.label("仅对所选月份生效。账目改变后，对应核对自动失效。");
            ui.label(if month < current_month {
                "完整月份经核对后才进入预测基线；无收支的月份需明确确认整月为零。"
            } else {
                "本月只核对截至今日的记录，月末后仍须核对整月。"
            });
            if load_error.is_none() && self.workspace_persistence_ready && money_inputs_ready {
                desktop_finance_review_row(
                    ui,
                    "收支",
                    risk.recorded_days > 0,
                    risk.income_expense_verified,
                    DesktopFinanceReviewLane::IncomeExpense,
                    if month < current_month {
                        "确认整月收支已记全"
                    } else {
                        "确认截至今日已记全"
                    },
                    if month < current_month {
                        "确认整月无收支（0）"
                    } else {
                        "确认截至今日无收支（0）"
                    },
                    &mut action,
                );
                desktop_finance_review_row(
                    ui,
                    "现金余额",
                    risk.recorded_cash.is_some(),
                    risk.verified_cash.is_some(),
                    DesktopFinanceReviewLane::Cash,
                    "核对现金",
                    "确认现金为零",
                    &mut action,
                );
                desktop_finance_review_row(
                    ui,
                    "资产",
                    risk.recorded_assets.is_some(),
                    risk.verified_assets.is_some(),
                    DesktopFinanceReviewLane::Assets,
                    "核对资产",
                    "确认资产为零",
                    &mut action,
                );
                desktop_finance_review_row(
                    ui,
                    "负债",
                    risk.recorded_liabilities.is_some(),
                    risk.verified_liabilities.is_some(),
                    DesktopFinanceReviewLane::Liabilities,
                    "核对负债",
                    "确认零负债",
                    &mut action,
                );
                if ui.small_button("清除本月核对").clicked() {
                    action = Some(DesktopFinanceReviewAction::ClearMonth);
                }
            } else if !money_inputs_ready {
                ui.label("请先修正金额输入或恢复原金额，再核对账目。");
            } else if load_error.is_none() {
                ui.label("本地工作区只读，暂不能保存核对。");
            }
        });
        ui.add_space(8.0);
        card_frame().show(ui, |ui| {
            section_heading(ui, "周期支出");
            if risk.recurring_expenses.is_empty() {
                ui.label("暂无稳定的跨月记录");
            }
            for item in &risk.recurring_expenses {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!(
                        "{} · 每月约 {} · {}",
                        item.name,
                        money_label(item.monthly_amount),
                        if item.paid_this_month {
                            "本月已记"
                        } else {
                            "本月未记"
                        }
                    ));
                    if load_error.is_none()
                        && self.workspace_persistence_ready
                        && money_inputs_ready
                        && ui
                            .small_button(if item.confirmed {
                                "取消确认"
                            } else {
                                "确认周期项"
                            })
                            .clicked()
                    {
                        action = Some(DesktopFinanceReviewAction::ToggleRecurring(
                            item.key.clone(),
                        ));
                    }
                });
            }
        });
        ui.add_space(8.0);
        card_frame().show(ui, |ui| {
            section_heading(ui, "疑似重复付款");
            if risk.duplicate_payments.is_empty() {
                ui.label("未发现需要核对的同日同额记录");
            }
            for item in &risk.duplicate_payments {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!(
                        "{} · {} · {} · {} 笔不同记录",
                        item.day_key,
                        item.name,
                        money_label(item.amount),
                        item.entry_ids.len()
                    ));
                    if ui.small_button("查看当日账目").clicked() {
                        action = Some(DesktopFinanceReviewAction::OpenDay(item.day_key.clone()));
                    }
                });
            }
            if let (Some(day), Some(amount)) = (&risk.anomaly_day_key, risk.anomaly_amount) {
                ui.add_space(6.0);
                ui.label(format!(
                    "异常支出提醒：{day} · {}。请核对原账目。",
                    money_label(amount)
                ));
                if ui.small_button("查看异常日账").clicked() {
                    action = Some(DesktopFinanceReviewAction::OpenDay(day.clone()));
                }
            }
        });
        if let Some(action) = action {
            self.desktop_finance_apply_review_action(action, &reference, &today);
        }
    }
}

#[cfg(test)]
mod desktop_finance_risk_v2_tests {
    use super::*;

    #[test]
    fn receipt_storage_is_scoped_and_fails_closed_on_corruption() {
        let root = std::env::temp_dir().join(format!(
            "finance_review_scope_{}_{}",
            std::process::id(),
            now_millis()
        ));
        let state_a = root.join("a").join("timer_state.json");
        let state_b = root.join("b").join("timer_state.json");
        let sync = DesktopSyncSession::default();
        let mut review = DesktopFinanceReviewState::load(&state_a, &sync);
        let receipt = gridtimer_native::FinanceReviewReceipt {
            month_key: "2026-08".into(),
            ..Default::default()
        };
        review.replace(&state_a, vec![receipt.clone()]).unwrap();
        assert_eq!(
            DesktopFinanceReviewState::load(&state_a, &sync).receipts,
            vec![receipt]
        );
        let ciphertext = fs::read(desktop_finance_review_path(&state_a)).unwrap();
        atomic_replace_bytes_no_backup(&desktop_finance_review_path(&state_b), &ciphertext)
            .unwrap();
        let other = DesktopFinanceReviewState::load(&state_b, &sync);
        assert!(other.load_error.is_some());
        assert!(other.receipts.is_empty());
        assert!(fs::read(&desktop_finance_review_path(&state_a))
            .unwrap()
            .starts_with(if cfg!(target_os = "windows") {
                DPAPI_SECRET_HEADER
            } else {
                PLAIN_SECRET_HEADER
            }));
        fs::write(desktop_finance_review_path(&state_a), b"broken").unwrap();
        let mut broken = DesktopFinanceReviewState::load(&state_a, &sync);
        assert!(broken.load_error.is_some());
        assert!(broken.replace(&state_a, vec![]).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn empty_month_needs_explicit_zero_and_receipt_fingerprint_invalidates() {
        let profile = DesktopFinanceProfile::default();
        let fp = desktop_finance_fingerprints(&profile, "2026-08").unwrap();
        let risk = desktop_finance_risk_v2(&profile, &[], "2026-08-31").unwrap();
        let mut receipts = Vec::new();
        assert!(desktop_finance_confirm_receipt(
            &mut receipts,
            &fp,
            &risk,
            "2026-08-31",
            "2026-09-01",
            DesktopFinanceReviewLane::IncomeExpense,
            false
        )
        .is_err());
        desktop_finance_confirm_receipt(
            &mut receipts,
            &fp,
            &risk,
            "2026-08-31",
            "2026-09-01",
            DesktopFinanceReviewLane::IncomeExpense,
            true,
        )
        .unwrap();
        assert!(receipts[0]
            .income_expense_fingerprint
            .starts_with("zero-full:"));
        assert!(
            desktop_finance_risk_v2(&profile, &receipts, "2026-08-31")
                .unwrap()
                .income_expense_verified
        );
        let mut changed = profile.clone();
        changed.extra.insert("dailyLedgers".into(), serde_json::json!({"2026-08-05":{"incomes":[{"id":"wage","name":"工资","kind":"ACTIVE","amount":5000}],"expenses":[]}}));
        assert!(
            !desktop_finance_risk_v2(&changed, &receipts, "2026-08-31")
                .unwrap()
                .income_expense_verified
        );
    }
}
