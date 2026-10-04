// v1.1.0.5 Windows - Edit and display RMB amounts as exact integer cents.

fn desktop_signed_money_label(minor: i64) -> String {
    format!(
        "{} 元",
        gridtimer_native::finance_money::format_minor(minor, "", true, false)
    )
}

fn desktop_money_average_minor(total: i64, months: i64) -> i64 {
    if months <= 0 {
        return 0;
    }
    let total = i128::from(total);
    let months = i128::from(months);
    let rounded = if total < 0 {
        (total - months / 2) / months
    } else {
        (total + months / 2) / months
    };
    rounded.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

#[derive(Clone)]
struct DesktopMoneyDraft {
    source: (i64, Option<i64>),
    text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum DesktopMoneyField {
    Row {
        monthly: bool,
        period: String,
        list: String,
        id: String,
        index: usize,
    },
    Legacy(String),
}

impl DesktopMoneyField {
    fn source(&self, profile: &DesktopFinanceProfile) -> Option<(i64, Option<i64>)> {
        match self {
            Self::Legacy(field) => {
                let whole = match field.as_str() {
                    "activeIncomeMonthly" => profile.active_income_monthly,
                    "assetIncomeMonthly" => profile.asset_income_monthly,
                    "livingExpenseMonthly" => profile.living_expense_monthly,
                    "liabilityPaymentMonthly" => profile.liability_payment_monthly,
                    "cashReserve" => profile.cash_reserve,
                    "productiveAssetValue" => profile.productive_asset_value,
                    "liabilityBalance" => profile.liability_balance,
                    _ => return None,
                };
                Some((whole, profile.legacy_amount_minor.get(field).copied()))
            }
            Self::Row {
                monthly,
                period,
                list,
                id,
                index,
            } => {
                let collection = if *monthly {
                    "monthlySnapshots"
                } else {
                    "dailyLedgers"
                };
                let rows = profile
                    .extra
                    .get(collection)?
                    .get(period)?
                    .get(list)?
                    .as_array()?;
                let row = if id.is_empty() {
                    rows.get(*index)?
                } else {
                    rows.iter()
                        .find(|row| row.get("id").and_then(Value::as_str) == Some(id.as_str()))?
                };
                if row.get("id").and_then(Value::as_str).unwrap_or("") != id
                    || row
                        .get("deletedAtEpochMillis")
                        .and_then(Value::as_i64)
                        .unwrap_or(0)
                        > 0
                {
                    return None;
                }
                let whole = row
                    .get("amount")
                    .filter(|value| !value.is_null())
                    .map(|value| value.as_i64().unwrap_or(i64::MIN))
                    .unwrap_or(0);
                let minor = row
                    .get("amountMinor")
                    .filter(|value| !value.is_null())
                    .map(|value| value.as_i64().unwrap_or(i64::MIN));
                Some((whole, minor))
            }
        }
    }
}

#[derive(Default)]
struct DesktopMoneyDrafts {
    scope: String,
    fields: std::collections::BTreeMap<DesktopMoneyField, DesktopMoneyDraft>,
    focus: Option<DesktopMoneyField>,
}

impl DesktopMoneyDrafts {
    fn reconcile(&mut self, scope: String, profile: &DesktopFinanceProfile) {
        if self.scope != scope {
            self.fields.clear();
            self.focus = None;
            self.scope = scope;
        }
        // A deleted/replaced record or an external amount update owns its new
        // value. Date/tab navigation alone must never discard unfinished input.
        self.fields
            .retain(|field, draft| field.source(profile) == Some(draft.source));
    }

    fn has_invalid(&self) -> bool {
        self.fields
            .values()
            .any(|draft| desktop_parse_money_input(&draft.text).is_none())
    }

    fn invalid_record(&self, monthly: bool, period: &str) -> bool {
        self.fields.iter().any(|(field, draft)| {
            matches!(field, DesktopMoneyField::Row { monthly: row_monthly, period: row_period, .. }
                if *row_monthly == monthly && row_period == period)
                && desktop_parse_money_input(&draft.text).is_none()
        })
    }

    fn first_invalid(&self) -> Option<DesktopMoneyField> {
        self.fields
            .iter()
            .find(|(_, draft)| desktop_parse_money_input(&draft.text).is_none())
            .map(|(field, _)| field.clone())
    }

    fn cancel_invalid(&mut self) {
        self.fields
            .retain(|_, draft| desktop_parse_money_input(&draft.text).is_some());
        self.focus = None;
    }
}

fn desktop_parse_money_input(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.is_empty() || text == "." {
        return None;
    }
    gridtimer_native::finance_money::parse_minor_draft(
        text,
        9,
        gridtimer_native::finance_money::INPUT_MAX_WHOLE,
    )
}

fn desktop_row_money_minor(row: &Value) -> Option<i64> {
    let whole = match row.get("amount") {
        None | Some(Value::Null) => 0,
        Some(value) => value.as_i64()?,
    };
    let minor = match row.get("amountMinor") {
        None | Some(Value::Null) => None,
        Some(value) => Some(value.as_i64()?),
    };
    gridtimer_native::finance_money::validated_minor(whole, minor)
}

fn desktop_apply_row_money_input(row: &mut Value, text: &str) -> bool {
    let Some(minor) = desktop_parse_money_input(text) else {
        return false;
    };
    let Some(object) = row.as_object_mut() else {
        return false;
    };
    if object.get("amount") == Some(&Value::from(minor / 100))
        && object.get("amountMinor") == Some(&Value::from(minor))
    {
        return false;
    }
    object.insert("amount".into(), Value::from(minor / 100));
    object.insert("amountMinor".into(), Value::from(minor));
    true
}

fn desktop_money_input(
    ui: &mut egui::Ui,
    key: impl std::hash::Hash,
    whole: i64,
    minor: Option<i64>,
    field: DesktopMoneyField,
    drafts: &mut DesktopMoneyDrafts,
) -> Option<i64> {
    let id = ui.make_persistent_id(("money_cent_draft", key));
    let source = (whole, minor);
    let mut draft = drafts
        .fields
        .get(&field)
        .cloned()
        .filter(|draft| draft.source == source)
        .unwrap_or_else(|| DesktopMoneyDraft {
            source,
            text: gridtimer_native::finance_money::validated_minor(whole, minor)
                .map(|value| gridtimer_native::finance_money::format_minor(value, "", false, false))
                .unwrap_or_default(),
        });
    let response = ui.add(
        egui::TextEdit::singleline(&mut draft.text)
            .id(id.with("field"))
            .desired_width(125.0)
            .hint_text("0.00 元"),
    );
    #[cfg(test)]
    ui.ctx().data_mut(|data| {
        data.insert_temp(egui::Id::new(("finance_money_input", &field)), response.id)
    });
    if drafts.focus.as_ref() == Some(&field) {
        response.request_focus();
        drafts.focus = None;
    }
    let parsed = desktop_parse_money_input(&draft.text);
    let result = response.changed().then_some(parsed).flatten();
    if let Some(value) = result {
        draft.source = (value / 100, Some(value));
    }
    if parsed.is_none() {
        ui.colored_label(palette().warn, "请输入金额，最多两位小数");
    }
    drafts.fields.insert(field, draft);
    result
}

fn desktop_legacy_minor(profile: &DesktopFinanceProfile, field: &str, whole: i64) -> Option<i64> {
    gridtimer_native::finance_money::validated_minor(
        whole,
        profile.legacy_amount_minor.get(field).copied(),
    )
}

fn desktop_legacy_money_field(
    ui: &mut egui::Ui,
    scope: &str,
    profile: &mut DesktopFinanceProfile,
    field: &str,
    label: &str,
    drafts: &mut DesktopMoneyDrafts,
) -> bool {
    let whole = match field {
        "activeIncomeMonthly" => profile.active_income_monthly,
        "assetIncomeMonthly" => profile.asset_income_monthly,
        "livingExpenseMonthly" => profile.living_expense_monthly,
        "liabilityPaymentMonthly" => profile.liability_payment_monthly,
        "cashReserve" => profile.cash_reserve,
        "productiveAssetValue" => profile.productive_asset_value,
        "liabilityBalance" => profile.liability_balance,
        _ => return false,
    };
    let old_minor = profile.legacy_amount_minor.get(field).copied();
    ui.label(
        egui::RichText::new(label)
            .size(13.0)
            .strong()
            .color(palette().text),
    );
    let Some(minor) = desktop_money_input(
        ui,
        (scope, field),
        whole,
        old_minor,
        DesktopMoneyField::Legacy(field.into()),
        drafts,
    ) else {
        return false;
    };
    if old_minor == Some(minor) && whole == minor / 100 {
        return false;
    }
    let next_whole = minor / 100;
    match field {
        "activeIncomeMonthly" => profile.active_income_monthly = next_whole,
        "assetIncomeMonthly" => profile.asset_income_monthly = next_whole,
        "livingExpenseMonthly" => profile.living_expense_monthly = next_whole,
        "liabilityPaymentMonthly" => profile.liability_payment_monthly = next_whole,
        "cashReserve" => profile.cash_reserve = next_whole,
        "productiveAssetValue" => profile.productive_asset_value = next_whole,
        "liabilityBalance" => profile.liability_balance = next_whole,
        _ => unreachable!("checked field"),
    }
    profile.legacy_amount_minor.insert(field.to_owned(), minor);
    true
}

#[cfg(test)]
mod desktop_money_precision_tests {
    use super::*;

    #[test]
    fn invalid_money_drafts_never_replace_a_saved_row_or_clear_precision() {
        let original =
            serde_json::json!({"amount":58,"amountMinor":5812,"id":"subscription","future":true});
        for text in ["", " ", ".", "58.123", "58..12", "-1", "1000000000", "1e2"] {
            let mut row = original.clone();
            assert!(!desktop_apply_row_money_input(&mut row, text), "{text}");
            assert_eq!(original, row, "{text}");
        }
        let mut row = original.clone();
        assert!(desktop_apply_row_money_input(&mut row, "0.01"));
        assert_eq!(0, row["amount"]);
        assert_eq!(1, row["amountMinor"]);
        assert_eq!(true, row["future"]);
        assert_eq!(Some(1), desktop_row_money_minor(&row));
        assert!(!desktop_apply_row_money_input(&mut row, "0.01"));
    }

    #[test]
    fn precise_rows_reach_totals_overviews_and_review_fingerprints_without_changing_wire_units() {
        let raw = serde_json::json!({
            "dailyLedgers": {"2026-10-02": {
                "incomes":[{"id":"income","name":"income","kind":"ACTIVE","amount":10,"amountMinor":1001}],
                "expenses":[{"id":"expense","name":"expense","bucket":"FOOD","amount":3,"amountMinor":333}]
            }},
            "monthlySnapshots": {"2026-10": {
                "assets":[{"id":"cash","name":"cash","kind":"CASH_RESERVE","amount":101,"amountMinor":10121}],
                "liabilities":[{"id":"debt","name":"debt","kind":"LIABILITY_BALANCE","amount":20,"amountMinor":2001}]
            }}
        });
        let original = raw.clone();
        assert_eq!(
            Some((1001, 333, 668)),
            finance_record_totals(&raw["dailyLedgers"]["2026-10-02"], false)
        );
        assert_eq!(
            Some((10121, 2001, 8120)),
            finance_record_totals(&raw["monthlySnapshots"]["2026-10"], true)
        );
        let period = finance_build_period_summary(&raw, "2026", 4, true, "2026-10-02").unwrap();
        assert_eq!((1001, 333, 668), finance_period_cashflow(&period.totals));
        let overview = build_finance_overview(&raw.to_string(), 1, "2026-10-02").unwrap();
        assert_eq!(1001, overview.report.total_income);
        assert_eq!(333, overview.report.total_outflow);
        assert_eq!(668, overview.report.net_cashflow);
        let profile: DesktopFinanceProfile = serde_json::from_value(raw.clone()).unwrap();
        let risk = desktop_finance_risk_v2(&profile, &[], "2026-10-02").unwrap();
        assert_eq!(1001, risk.recorded_income);
        assert_eq!(333, risk.recorded_outflow);
        assert_eq!(Some(10121), risk.recorded_cash);
        let fp = desktop_finance_fingerprints(&profile, "2026-10").unwrap();
        let mut receipts = Vec::new();
        desktop_finance_confirm_receipt(
            &mut receipts,
            &fp,
            &risk,
            "2026-10-02",
            "2026-10-02",
            DesktopFinanceReviewLane::Cash,
            false,
        )
        .unwrap();
        assert_eq!(
            Some(10121),
            desktop_finance_risk_v2(&profile, &receipts, "2026-10-02")
                .unwrap()
                .verified_cash
        );
        let mut changed = raw.clone();
        changed["monthlySnapshots"]["2026-10"]["assets"][0]["amountMinor"] = Value::from(10122);
        let changed_profile = serde_json::from_value(changed).unwrap();
        assert_eq!(
            None,
            desktop_finance_risk_v2(&changed_profile, &receipts, "2026-10-02")
                .unwrap()
                .verified_cash
        );
        assert_eq!(original, raw);
        assert_eq!(
            10,
            serde_json::to_value(&profile).unwrap()["dailyLedgers"]["2026-10-02"]["incomes"][0]
                ["amount"]
        );
        assert!(build_finance_overview(
            &gridtimer_native::finance_money::project_profile_json(&raw.to_string()).unwrap(),
            1,
            "2026-10-02"
        )
        .is_none());
    }

    #[test]
    fn row_money_reads_old_yuan_and_rejects_corrupt_precision() {
        assert_eq!(
            Some(5800),
            desktop_row_money_minor(&serde_json::json!({"amount":58}))
        );
        assert_eq!(
            Some(5812),
            desktop_row_money_minor(&serde_json::json!({"amount":58,"amountMinor":5812}))
        );
        for row in [
            serde_json::json!({"amount":58,"amountMinor":5912}),
            serde_json::json!({"amount":58,"amountMinor":"5812"}),
            serde_json::json!({"amount":i64::MAX}),
        ] {
            assert_eq!(None, desktop_row_money_minor(&row));
        }
        assert_eq!("58.12 元", money_label(5812));
        assert_eq!("-0.01 元", money_label(-1));
        assert_eq!("+0.01 元", desktop_signed_money_label(1));
        assert_eq!(1, desktop_money_average_minor(1, 2));
        assert_eq!(-1, desktop_money_average_minor(-1, 2));
    }
}
