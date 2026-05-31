use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

const MAX_FINANCE_AMOUNT: i64 = 999_999_999;
const MAX_FINANCE_DAILY_RECORDS: usize = 1_460;
const MAX_FINANCE_MONTHLY_RECORDS: usize = 240;
const MAX_FINANCE_LEDGER_LINES: usize = 24;
const MAX_FINANCE_ENTRY_NAME_LENGTH: usize = 32;
const MAX_FINANCE_ENTRY_NOTE_LENGTH: usize = 48;
const MAX_FINANCE_LEDGER_NOTE_LENGTH: usize = 180;
const MAX_FINANCE_CATEGORY_LABEL_LENGTH: usize = 12;
const FINANCE_BACKUP_SCHEMA_VERSION: i32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FinanceExpenseBucket {
    Debt,
    Food,
    Btc,
    Living,
    Learning,
    Other,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FinanceIncomeKind {
    Active,
    Asset,
    Other,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FinanceNamedAmountKind {
    CashReserve,
    ProductiveAsset,
    OtherAsset,
    LiabilityBalance,
    OtherLiability,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FinanceProfile {
    #[serde(default)]
    pub active_income_monthly: i64,
    #[serde(default)]
    pub asset_income_monthly: i64,
    #[serde(default)]
    pub living_expense_monthly: i64,
    #[serde(default)]
    pub liability_payment_monthly: i64,
    #[serde(default)]
    pub cash_reserve: i64,
    #[serde(default)]
    pub productive_asset_value: i64,
    #[serde(default)]
    pub liability_balance: i64,
    #[serde(default)]
    pub acquisition_focus: String,
    #[serde(default)]
    pub liability_focus: String,
    #[serde(default)]
    pub settings: FinanceSettings,
    #[serde(default)]
    pub daily_ledgers: IndexMap<String, FinanceDayLedger>,
    #[serde(default)]
    pub monthly_snapshots: IndexMap<String, FinanceMonthSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct FinanceBackupPayload {
    #[serde(default = "default_finance_backup_schema_version")]
    schema_version: i32,
    #[serde(default)]
    exported_at_epoch_millis: i64,
    #[serde(default)]
    app_version_name: String,
    finance_profile: FinanceProfile,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FinanceSettings {
    #[serde(default = "default_finance_expense_category_configs")]
    pub expense_categories: Vec<FinanceExpenseCategoryConfig>,
}

impl Default for FinanceSettings {
    fn default() -> Self {
        Self {
            expense_categories: default_finance_expense_category_configs(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FinanceExpenseCategoryConfig {
    #[serde(default = "default_expense_bucket")]
    pub bucket: FinanceExpenseBucket,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub target_share_of_income: Option<f32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FinanceIncomeEntry {
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_income_kind")]
    pub kind: FinanceIncomeKind,
    #[serde(default)]
    pub amount: i64,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FinanceExpenseEntry {
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_expense_bucket")]
    pub bucket: FinanceExpenseBucket,
    #[serde(default)]
    pub amount: i64,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FinanceNamedAmountEntry {
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_named_amount_kind")]
    pub kind: FinanceNamedAmountKind,
    #[serde(default)]
    pub amount: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FinanceDayLedger {
    #[serde(default)]
    pub incomes: Vec<FinanceIncomeEntry>,
    #[serde(default)]
    pub expenses: Vec<FinanceExpenseEntry>,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FinanceMonthSnapshot {
    #[serde(default)]
    pub assets: Vec<FinanceNamedAmountEntry>,
    #[serde(default)]
    pub liabilities: Vec<FinanceNamedAmountEntry>,
    #[serde(default)]
    pub note: String,
}

impl Default for FinanceExpenseBucket {
    fn default() -> Self {
        Self::Other
    }
}

impl Default for FinanceIncomeKind {
    fn default() -> Self {
        Self::Active
    }
}

impl Default for FinanceNamedAmountKind {
    fn default() -> Self {
        Self::OtherAsset
    }
}

pub fn sanitize_finance_profile_json(raw: &str) -> Option<String> {
    let profile = serde_json::from_str::<FinanceProfile>(raw).ok()?;
    serde_json::to_string(&profile.sanitized()).ok()
}

pub fn decode_finance_backup_profile_json(raw: &str) -> Option<String> {
    let root = serde_json::from_str::<serde_json::Value>(raw).ok()?;
    let looks_like_backup_payload = root.as_object().is_some_and(|object| {
        object.contains_key("financeProfile")
            || object.contains_key("schemaVersion")
            || object.contains_key("appVersionName")
    });

    if looks_like_backup_payload {
        let payload = serde_json::from_value::<FinanceBackupPayload>(root).ok()?;
        if payload.schema_version < 1 || payload.schema_version > FINANCE_BACKUP_SCHEMA_VERSION {
            return None;
        }
        serde_json::to_string(&payload.finance_profile.sanitized()).ok()
    } else {
        let profile = serde_json::from_str::<FinanceProfile>(raw).ok()?;
        serde_json::to_string(&profile.sanitized()).ok()
    }
}

pub fn encode_finance_backup_json(
    profile_json: &str,
    app_version_name: &str,
    exported_at_epoch_millis: i64,
) -> Option<String> {
    let profile = serde_json::from_str::<FinanceProfile>(profile_json)
        .ok()?
        .sanitized();
    let payload = FinanceBackupPayload {
        schema_version: FINANCE_BACKUP_SCHEMA_VERSION,
        exported_at_epoch_millis: exported_at_epoch_millis.max(0),
        app_version_name: app_version_name.trim().to_string(),
        finance_profile: profile,
    };
    serde_json::to_string_pretty(&payload).ok()
}

pub fn upsert_finance_day_ledger_json(
    profile_json: &str,
    day_key: &str,
    ledger_json: &str,
) -> Option<String> {
    if !is_valid_finance_day_key(day_key) {
        return None;
    }
    let mut profile = serde_json::from_str::<FinanceProfile>(profile_json).ok()?;
    let ledger = serde_json::from_str::<FinanceDayLedger>(ledger_json)
        .ok()?
        .sanitized();
    if ledger.has_entries() {
        profile.daily_ledgers.insert(day_key.to_string(), ledger);
    } else {
        profile.daily_ledgers.shift_remove(day_key);
    }
    serde_json::to_string(&profile).ok()
}

pub fn upsert_finance_month_snapshot_json(
    profile_json: &str,
    month_key: &str,
    snapshot_json: &str,
) -> Option<String> {
    if !is_valid_finance_month_key(month_key) {
        return None;
    }
    let mut profile = serde_json::from_str::<FinanceProfile>(profile_json).ok()?;
    let snapshot = serde_json::from_str::<FinanceMonthSnapshot>(snapshot_json)
        .ok()?
        .sanitized();
    if snapshot.has_entries() {
        profile
            .monthly_snapshots
            .insert(month_key.to_string(), snapshot);
    } else {
        profile.monthly_snapshots.shift_remove(month_key);
    }
    serde_json::to_string(&profile).ok()
}

pub fn finance_day_ledger_or_default_json(profile_json: &str, day_key: &str) -> Option<String> {
    if !is_valid_finance_day_key(day_key) {
        return None;
    }
    let profile = serde_json::from_str::<FinanceProfile>(profile_json).ok()?;
    let ledger = profile
        .daily_ledgers
        .get(day_key)
        .cloned()
        .unwrap_or_default();
    serde_json::to_string(&ledger).ok()
}

pub fn finance_month_snapshot_or_default_json(
    profile_json: &str,
    month_key: &str,
) -> Option<String> {
    if !is_valid_finance_month_key(month_key) {
        return None;
    }
    let profile = serde_json::from_str::<FinanceProfile>(profile_json).ok()?;
    let snapshot = profile
        .monthly_snapshots
        .get(month_key)
        .cloned()
        .unwrap_or_default();
    serde_json::to_string(&snapshot).ok()
}

pub fn sanitize_finance_settings_json(raw: &str) -> Option<String> {
    let settings = serde_json::from_str::<FinanceSettings>(raw).ok()?;
    serde_json::to_string(&settings.sanitized()).ok()
}

pub fn finance_settings_config_for_json(raw: &str, bucket_code: i32) -> Option<String> {
    let bucket = bucket_from_code(bucket_code)?;
    let settings = serde_json::from_str::<FinanceSettings>(raw)
        .ok()?
        .sanitized();
    serde_json::to_string(&settings.config_for(bucket)).ok()
}

pub fn replace_finance_expense_category_config_json(
    settings_json: &str,
    bucket_code: i32,
    config_json: &str,
) -> Option<String> {
    let bucket = bucket_from_code(bucket_code)?;
    let mut settings = serde_json::from_str::<FinanceSettings>(settings_json)
        .ok()?
        .sanitized();
    let mut updated = serde_json::from_str::<FinanceExpenseCategoryConfig>(config_json).ok()?;
    updated.bucket = bucket;
    updated = updated.sanitized();
    settings.expense_categories = primary_bucket_order()
        .into_iter()
        .map(|current_bucket| {
            if current_bucket == bucket {
                updated.clone()
            } else {
                settings.config_for(current_bucket)
            }
        })
        .collect();
    serde_json::to_string(&settings.sanitized()).ok()
}

pub fn sanitize_finance_expense_category_config_json(
    raw: &str,
    fallback_bucket_code: i32,
) -> Option<String> {
    let fallback_bucket = bucket_from_code(fallback_bucket_code)?;
    let mut config = serde_json::from_str::<FinanceExpenseCategoryConfig>(raw).ok()?;
    config.bucket = fallback_bucket;
    serde_json::to_string(&config.sanitized()).ok()
}

pub fn default_finance_expense_category_configs_json() -> Option<String> {
    serde_json::to_string(&default_finance_expense_category_configs()).ok()
}

pub fn default_finance_expense_category_config_json(bucket_code: i32) -> Option<String> {
    let bucket = bucket_from_code(bucket_code)?;
    let config = FinanceExpenseCategoryConfig {
        bucket,
        label: bucket_default_label(&bucket).to_string(),
        target_share_of_income: bucket_default_target(&bucket),
    };
    serde_json::to_string(&config).ok()
}

pub fn default_finance_expense_entries_json() -> Option<String> {
    serde_json::to_string(&default_finance_expense_entries()).ok()
}

pub fn default_finance_income_entries_json() -> Option<String> {
    serde_json::to_string(&default_finance_income_entries()).ok()
}

pub fn default_finance_asset_entries_json() -> Option<String> {
    serde_json::to_string(&default_finance_asset_entries()).ok()
}

pub fn default_finance_liability_entries_json() -> Option<String> {
    serde_json::to_string(&default_finance_liability_entries()).ok()
}

pub fn append_missing_income_templates_json(
    rows_json: &str,
    templates_json: &str,
) -> Option<String> {
    let rows = serde_json::from_str::<Vec<FinanceIncomeEntry>>(rows_json).ok()?;
    let templates = serde_json::from_str::<Vec<FinanceIncomeEntry>>(templates_json).ok()?;
    let appended = append_missing_templates(rows, templates, |entry| {
        template_key(income_kind_code(&entry.kind), &entry.name)
    });
    serde_json::to_string(&appended).ok()
}

pub fn append_missing_expense_templates_json(
    rows_json: &str,
    templates_json: &str,
) -> Option<String> {
    let rows = serde_json::from_str::<Vec<FinanceExpenseEntry>>(rows_json).ok()?;
    let templates = serde_json::from_str::<Vec<FinanceExpenseEntry>>(templates_json).ok()?;
    let appended = append_missing_templates(rows, templates, |entry| {
        template_key(bucket_code(&entry.bucket), &entry.name)
    });
    serde_json::to_string(&appended).ok()
}

pub fn append_missing_named_amount_templates_json(
    rows_json: &str,
    templates_json: &str,
) -> Option<String> {
    let rows = serde_json::from_str::<Vec<FinanceNamedAmountEntry>>(rows_json).ok()?;
    let templates = serde_json::from_str::<Vec<FinanceNamedAmountEntry>>(templates_json).ok()?;
    let appended = append_missing_templates(rows, templates, |entry| {
        template_key(named_amount_kind_code(&entry.kind), &entry.name)
    });
    serde_json::to_string(&appended).ok()
}

pub fn finance_row_edit_json(
    row_kind_code: i32,
    operation_code: i32,
    rows_json: &str,
    index: i32,
    entry_json: &str,
) -> Option<String> {
    match row_kind_code {
        0 => edit_finance_rows::<FinanceIncomeEntry>(operation_code, rows_json, index, entry_json),
        1 => edit_finance_rows::<FinanceExpenseEntry>(operation_code, rows_json, index, entry_json),
        2 => edit_finance_rows::<FinanceNamedAmountEntry>(
            operation_code,
            rows_json,
            index,
            entry_json,
        ),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FinanceLedgerTotals {
    pub active_income_total: i64,
    pub asset_income_total: i64,
    pub other_income_total: i64,
    pub debt_total: i64,
    pub food_total: i64,
    pub btc_total: i64,
    pub living_total: i64,
    pub learning_total: i64,
    pub other_expense_total: i64,
    pub days_with_entries: i64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FinanceMonthSnapshotTotals {
    pub asset_total: i64,
    pub liability_total: i64,
    pub cash_reserve_total: i64,
    pub productive_asset_total: i64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FinanceTrendValues {
    pub income_delta: i64,
    pub outflow_delta: i64,
    pub net_cashflow_delta: i64,
    pub net_worth_delta: i64,
    pub current_recorded_days: i64,
    pub previous_recorded_days: i64,
    pub most_off_target_bucket_code: i32,
    pub most_off_target_ratio_delta: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FinanceSnapshotValues {
    pub total_income: i64,
    pub total_outflow: i64,
    pub net_cashflow: i64,
    pub freedom_gap: i64,
    pub passive_coverage_ratio: f32,
    pub wage_dependence_ratio: f32,
    pub liability_pressure_ratio: f32,
    pub net_worth: i64,
    pub defensive_coverage: Option<f32>,
    pub asset_yield_ratio: f32,
    pub recorded_days: i64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FinanceAlertPlanItem {
    pub kind_code: i32,
    pub bucket_code: i32,
    pub drift: f32,
}

const ALERT_KIND_INITIAL_DATA: i32 = 0;
const ALERT_KIND_NEGATIVE_CASHFLOW: i32 = 1;
const ALERT_KIND_LOW_DEFENSIVE_COVERAGE: i32 = 2;
const ALERT_KIND_LOW_RECORD_DENSITY: i32 = 3;
const ALERT_KIND_TARGET_DRIFT: i32 = 4;
const ALERT_KIND_EMPTY_MONTH_SNAPSHOT: i32 = 5;
const ALERT_KIND_CLEAN: i32 = 6;

const PERIOD_DAY: i32 = 0;
const PERIOD_MONTH: i32 = 1;
const PERIOD_QUARTER: i32 = 2;
const PERIOD_YEAR: i32 = 3;

const BUCKET_CODE_DEBT: i32 = 0;
const BUCKET_CODE_FOOD: i32 = 1;
const BUCKET_CODE_BTC: i32 = 2;
const BUCKET_CODE_LIVING: i32 = 3;
const BUCKET_CODE_LEARNING: i32 = 4;
const BUCKET_CODE_OTHER: i32 = 5;
const BUCKET_CODE_NONE: i32 = -1;

impl FinanceLedgerTotals {
    fn empty() -> Self {
        Self {
            active_income_total: 0,
            asset_income_total: 0,
            other_income_total: 0,
            debt_total: 0,
            food_total: 0,
            btc_total: 0,
            living_total: 0,
            learning_total: 0,
            other_expense_total: 0,
            days_with_entries: 0,
        }
    }

    fn income_total(self) -> i64 {
        self.active_income_total
            .saturating_add(self.asset_income_total)
            .saturating_add(self.other_income_total)
    }

    fn expense_total(self) -> i64 {
        self.debt_total
            .saturating_add(self.food_total)
            .saturating_add(self.btc_total)
            .saturating_add(self.living_total)
            .saturating_add(self.learning_total)
            .saturating_add(self.other_expense_total)
    }

    fn net_cashflow(self) -> i64 {
        self.income_total().saturating_sub(self.expense_total())
    }

    fn bucket_total(self, bucket: &FinanceExpenseBucket) -> i64 {
        match bucket {
            FinanceExpenseBucket::Debt => self.debt_total,
            FinanceExpenseBucket::Food => self.food_total,
            FinanceExpenseBucket::Btc => self.btc_total,
            FinanceExpenseBucket::Living => self.living_total,
            FinanceExpenseBucket::Learning => self.learning_total,
            FinanceExpenseBucket::Other => self.other_expense_total,
        }
    }

    fn share_of_income(self, bucket: &FinanceExpenseBucket) -> f32 {
        let income_total = self.income_total();
        if income_total <= 0 {
            0.0
        } else {
            self.bucket_total(bucket) as f32 / income_total as f32
        }
    }
}

impl FinanceMonthSnapshotTotals {
    fn empty() -> Self {
        Self {
            asset_total: 0,
            liability_total: 0,
            cash_reserve_total: 0,
            productive_asset_total: 0,
        }
    }

    fn net_worth(self) -> i64 {
        self.asset_total.saturating_sub(self.liability_total)
    }
}

pub fn build_finance_trend_values(
    raw: &str,
    period_code: i32,
    reference_year: i32,
    reference_month: i32,
    reference_day: i32,
) -> Option<FinanceTrendValues> {
    let profile = serde_json::from_str::<FinanceProfile>(raw).ok()?;
    Some(profile.finance_trend_values(period_code, reference_year, reference_month, reference_day))
}

pub fn build_finance_alert_plan_values(
    raw: &str,
    period_code: i32,
    reference_year: i32,
    reference_month: i32,
    reference_day: i32,
) -> Option<Vec<i32>> {
    let profile = serde_json::from_str::<FinanceProfile>(raw).ok()?;
    let plan =
        profile.finance_alert_plan(period_code, reference_year, reference_month, reference_day);
    let mut values = Vec::with_capacity(plan.len() * 3);
    for item in plan {
        values.push(item.kind_code);
        values.push(item.bucket_code);
        values.push((item.drift * 1_000_000.0).round() as i32);
    }
    Some(values)
}

pub fn build_finance_alert_argument_values(
    raw: &str,
    period_code: i32,
    reference_year: i32,
    reference_month: i32,
    reference_day: i32,
) -> Option<Vec<String>> {
    let profile = serde_json::from_str::<FinanceProfile>(raw).ok()?;
    let aggregate =
        profile.aggregate_for_period(period_code, reference_year, reference_month, reference_day);
    let settings = profile.settings.clone().sanitized();
    let plan =
        profile.finance_alert_plan(period_code, reference_year, reference_month, reference_day);
    let mut values = Vec::with_capacity(plan.len() * 10);
    for item in plan {
        let bucket = bucket_from_code(item.bucket_code);
        let actual_percent = bucket
            .map(|bucket| format_model_ratio_percent(aggregate.share_of_income(&bucket)))
            .unwrap_or_default();
        let target_percent = bucket
            .and_then(|bucket| settings.target_for(&bucket))
            .map(format_model_ratio_percent)
            .unwrap_or_default();
        let bucket_label = bucket
            .map(|bucket| settings.label_for(&bucket))
            .unwrap_or_default();
        values.push(item.kind_code.to_string());
        values.push(item.bucket_code.to_string());
        values.push(((item.drift * 1_000_000.0).round() as i32).to_string());
        values.push(if item.drift > 0.0 { "高于" } else { "低于" }.to_string());
        values.push(actual_percent);
        values.push(target_percent);
        values.push(bucket_label);
        values.push(period_scope_label(period_code).to_string());
        values.push(period_coverage_unit_label(period_code).to_string());
        values.push(aggregate.days_with_entries.to_string());
    }
    Some(values)
}

pub fn aggregate_finance_ledger_values(
    raw: &str,
    period_code: i32,
    reference_year: i32,
    reference_month: i32,
    reference_day: i32,
) -> Option<FinanceLedgerTotals> {
    let profile = serde_json::from_str::<FinanceProfile>(raw).ok()?;
    Some(profile.aggregate_for_period(period_code, reference_year, reference_month, reference_day))
}

pub fn build_detailed_finance_snapshot_values(
    raw: &str,
    reference_year: i32,
    reference_month: i32,
) -> Option<FinanceSnapshotValues> {
    let profile = serde_json::from_str::<FinanceProfile>(raw).ok()?;
    Some(profile.detailed_finance_snapshot_values(reference_year, reference_month))
}

pub fn build_detailed_finance_report_values(
    raw: &str,
    period_code: i32,
    reference_year: i32,
    reference_month: i32,
    reference_day: i32,
) -> Option<FinanceSnapshotValues> {
    let profile = serde_json::from_str::<FinanceProfile>(raw).ok()?;
    Some(profile.detailed_finance_report_values(
        period_code,
        reference_year,
        reference_month,
        reference_day,
    ))
}

pub fn year_net_worth_summary_values(raw: &str, year: i32) -> Option<[i64; 2]> {
    let profile = serde_json::from_str::<FinanceProfile>(raw).ok()?;
    let summary = profile.year_net_worth_summary(year);
    Some([summary.0, summary.1])
}

pub fn latest_snapshot_month_key_up_to(raw: &str, reference_month_key: &str) -> Option<String> {
    let profile = serde_json::from_str::<FinanceProfile>(raw).ok()?;
    if !is_valid_finance_month_key(reference_month_key) {
        return Some(String::new());
    }
    Some(
        profile
            .latest_snapshot_month_key_up_to(reference_month_key)
            .unwrap_or_default(),
    )
}

pub fn previous_recorded_day_key(raw: &str, before_day_key: &str) -> Option<String> {
    let profile = serde_json::from_str::<FinanceProfile>(raw).ok()?;
    Some(
        profile
            .previous_recorded_day_key(before_day_key)
            .unwrap_or_default(),
    )
}

pub fn previous_recorded_month_key(raw: &str, before_month_key: &str) -> Option<String> {
    let profile = serde_json::from_str::<FinanceProfile>(raw).ok()?;
    Some(
        profile
            .previous_recorded_month_key(before_month_key)
            .unwrap_or_default(),
    )
}

pub fn profile_summary_flags(raw: &str) -> Option<[i32; 2]> {
    let profile = serde_json::from_str::<FinanceProfile>(raw)
        .ok()?
        .sanitized();
    Some([
        if profile.has_entries() { 1 } else { 0 },
        if profile.has_detailed_finance_data() {
            1
        } else {
            0
        },
    ])
}

impl FinanceProfile {
    pub fn sanitized(self) -> Self {
        Self {
            active_income_monthly: sanitized_finance_amount(self.active_income_monthly),
            asset_income_monthly: sanitized_finance_amount(self.asset_income_monthly),
            living_expense_monthly: sanitized_finance_amount(self.living_expense_monthly),
            liability_payment_monthly: sanitized_finance_amount(self.liability_payment_monthly),
            cash_reserve: sanitized_finance_amount(self.cash_reserve),
            productive_asset_value: sanitized_finance_amount(self.productive_asset_value),
            liability_balance: sanitized_finance_amount(self.liability_balance),
            acquisition_focus: compact_whitespace(&self.acquisition_focus, 24),
            liability_focus: compact_whitespace(&self.liability_focus, 24),
            settings: self.settings.sanitized(),
            daily_ledgers: sanitize_finance_day_ledgers(self.daily_ledgers),
            monthly_snapshots: sanitize_finance_month_snapshots(self.monthly_snapshots),
        }
    }

    fn has_entries(&self) -> bool {
        self.active_income_monthly > 0
            || self.asset_income_monthly > 0
            || self.living_expense_monthly > 0
            || self.liability_payment_monthly > 0
            || self.cash_reserve > 0
            || self.productive_asset_value > 0
            || self.liability_balance > 0
            || !self.acquisition_focus.trim().is_empty()
            || !self.liability_focus.trim().is_empty()
            || !self.daily_ledgers.is_empty()
            || !self.monthly_snapshots.is_empty()
    }

    fn has_detailed_finance_data(&self) -> bool {
        !self.daily_ledgers.is_empty() || !self.monthly_snapshots.is_empty()
    }

    fn finance_trend_values(
        &self,
        period_code: i32,
        reference_year: i32,
        reference_month: i32,
        reference_day: i32,
    ) -> FinanceTrendValues {
        if !self.has_detailed_finance_data() {
            return FinanceTrendValues {
                income_delta: 0,
                outflow_delta: 0,
                net_cashflow_delta: 0,
                net_worth_delta: 0,
                current_recorded_days: 0,
                previous_recorded_days: 0,
                most_off_target_bucket_code: BUCKET_CODE_NONE,
                most_off_target_ratio_delta: 0.0,
            };
        }

        let current_aggregate =
            self.aggregate_for_period(period_code, reference_year, reference_month, reference_day);
        let (previous_year, previous_month, previous_day) =
            previous_reference_date(period_code, reference_year, reference_month, reference_day);
        let previous_aggregate =
            self.aggregate_for_period(period_code, previous_year, previous_month, previous_day);
        let current_summary = self.latest_snapshot_summary_up_to(reference_year, reference_month);
        let previous_summary = self.latest_snapshot_summary_up_to(previous_year, previous_month);
        let target_drift = self.most_problematic_target_drift(current_aggregate);

        FinanceTrendValues {
            income_delta: current_aggregate.income_total() - previous_aggregate.income_total(),
            outflow_delta: current_aggregate.expense_total() - previous_aggregate.expense_total(),
            net_cashflow_delta: current_aggregate.net_cashflow()
                - previous_aggregate.net_cashflow(),
            net_worth_delta: current_summary.net_worth() - previous_summary.net_worth(),
            current_recorded_days: current_aggregate.days_with_entries,
            previous_recorded_days: previous_aggregate.days_with_entries,
            most_off_target_bucket_code: target_drift
                .map(|(bucket, _)| bucket_code(&bucket))
                .unwrap_or(BUCKET_CODE_NONE),
            most_off_target_ratio_delta: target_drift.map(|(_, drift)| drift).unwrap_or(0.0),
        }
    }

    fn finance_alert_plan(
        &self,
        period_code: i32,
        reference_year: i32,
        reference_month: i32,
        reference_day: i32,
    ) -> Vec<FinanceAlertPlanItem> {
        if !self.has_detailed_finance_data() {
            return vec![FinanceAlertPlanItem {
                kind_code: ALERT_KIND_INITIAL_DATA,
                bucket_code: BUCKET_CODE_NONE,
                drift: 0.0,
            }];
        }

        let aggregate =
            self.aggregate_for_period(period_code, reference_year, reference_month, reference_day);
        let summary = self.latest_snapshot_summary_up_to(reference_year, reference_month);
        let defensive_base = aggregate.expense_total() - aggregate.asset_income_total;
        let defensive_coverage = if summary.cash_reserve_total <= 0 {
            Some(0.0)
        } else if defensive_base <= 0 {
            None
        } else {
            Some(summary.cash_reserve_total as f32 / defensive_base as f32)
        };

        let mut warnings = Vec::new();
        let mut infos = Vec::new();
        if aggregate.net_cashflow() < 0 {
            warnings.push(FinanceAlertPlanItem {
                kind_code: ALERT_KIND_NEGATIVE_CASHFLOW,
                bucket_code: BUCKET_CODE_NONE,
                drift: 0.0,
            });
        }
        if defensive_coverage.is_some_and(|value| value < 2.0) {
            warnings.push(FinanceAlertPlanItem {
                kind_code: ALERT_KIND_LOW_DEFENSIVE_COVERAGE,
                bucket_code: BUCKET_CODE_NONE,
                drift: 0.0,
            });
        }
        let expected_days =
            expected_recorded_days(period_code, reference_year, reference_month, reference_day);
        if expected_days > 0 && aggregate.days_with_entries * 2 < expected_days as i64 {
            infos.push(FinanceAlertPlanItem {
                kind_code: ALERT_KIND_LOW_RECORD_DENSITY,
                bucket_code: BUCKET_CODE_NONE,
                drift: 0.0,
            });
        }
        if let Some((bucket, drift)) = self.most_problematic_target_drift(aggregate) {
            warnings.push(FinanceAlertPlanItem {
                kind_code: ALERT_KIND_TARGET_DRIFT,
                bucket_code: bucket_code(&bucket),
                drift,
            });
        }
        if summary.asset_total <= 0 && summary.liability_total <= 0 {
            infos.push(FinanceAlertPlanItem {
                kind_code: ALERT_KIND_EMPTY_MONTH_SNAPSHOT,
                bucket_code: BUCKET_CODE_NONE,
                drift: 0.0,
            });
        }

        let mut plan = Vec::with_capacity(3);
        plan.extend(warnings);
        plan.extend(infos);
        if plan.is_empty() {
            plan.push(FinanceAlertPlanItem {
                kind_code: ALERT_KIND_CLEAN,
                bucket_code: BUCKET_CODE_NONE,
                drift: 0.0,
            });
        }
        plan.truncate(3);
        plan
    }

    fn detailed_finance_snapshot_values(
        &self,
        reference_year: i32,
        reference_month: i32,
    ) -> FinanceSnapshotValues {
        let aggregate = self.aggregate_for_period(PERIOD_MONTH, reference_year, reference_month, 1);
        let summary = self.latest_snapshot_summary_up_to(reference_year, reference_month);
        snapshot_values_from_totals(aggregate, summary, 0)
    }

    fn detailed_finance_report_values(
        &self,
        period_code: i32,
        reference_year: i32,
        reference_month: i32,
        reference_day: i32,
    ) -> FinanceSnapshotValues {
        let aggregate =
            self.aggregate_for_period(period_code, reference_year, reference_month, reference_day);
        let summary = self.latest_snapshot_summary_up_to(reference_year, reference_month);
        snapshot_values_from_totals(aggregate, summary, aggregate.days_with_entries)
    }

    fn aggregate_for_period(
        &self,
        period_code: i32,
        reference_year: i32,
        reference_month: i32,
        reference_day: i32,
    ) -> FinanceLedgerTotals {
        let mut aggregate = FinanceLedgerTotals::empty();
        for (key, ledger) in &self.daily_ledgers {
            let Some((year, month, day)) = parse_finance_day_key(key) else {
                continue;
            };
            if !date_matches_period(
                period_code,
                year,
                month,
                day,
                reference_year,
                reference_month,
                reference_day,
            ) {
                continue;
            }
            let daily = ledger.to_aggregate_totals();
            aggregate.active_income_total = aggregate
                .active_income_total
                .saturating_add(daily.active_income_total);
            aggregate.asset_income_total = aggregate
                .asset_income_total
                .saturating_add(daily.asset_income_total);
            aggregate.other_income_total = aggregate
                .other_income_total
                .saturating_add(daily.other_income_total);
            aggregate.debt_total = aggregate.debt_total.saturating_add(daily.debt_total);
            aggregate.food_total = aggregate.food_total.saturating_add(daily.food_total);
            aggregate.btc_total = aggregate.btc_total.saturating_add(daily.btc_total);
            aggregate.living_total = aggregate.living_total.saturating_add(daily.living_total);
            aggregate.learning_total = aggregate
                .learning_total
                .saturating_add(daily.learning_total);
            aggregate.other_expense_total = aggregate
                .other_expense_total
                .saturating_add(daily.other_expense_total);
            aggregate.days_with_entries += daily.days_with_entries;
        }
        aggregate
    }

    fn latest_snapshot_summary_up_to(
        &self,
        reference_year: i32,
        reference_month: i32,
    ) -> FinanceMonthSnapshotTotals {
        let reference_key = format!("{reference_year:04}-{reference_month:02}");
        let Some(key) = self.latest_snapshot_month_key_up_to(&reference_key) else {
            return FinanceMonthSnapshotTotals::empty();
        };
        self.monthly_snapshots
            .get(&key)
            .map(FinanceMonthSnapshot::to_summary_totals)
            .unwrap_or_else(FinanceMonthSnapshotTotals::empty)
    }

    fn latest_snapshot_month_key_up_to(&self, reference_month_key: &str) -> Option<String> {
        self.monthly_snapshots
            .keys()
            .filter(|key| is_valid_finance_month_key(key) && key.as_str() <= reference_month_key)
            .max()
            .cloned()
    }

    fn previous_recorded_day_key(&self, before_day_key: &str) -> Option<String> {
        self.daily_ledgers
            .keys()
            .filter(|key| is_valid_finance_day_key(key) && key.as_str() < before_day_key)
            .max()
            .cloned()
    }

    fn previous_recorded_month_key(&self, before_month_key: &str) -> Option<String> {
        self.monthly_snapshots
            .keys()
            .filter(|key| is_valid_finance_month_key(key) && key.as_str() < before_month_key)
            .max()
            .cloned()
    }

    fn year_net_worth_summary(&self, year: i32) -> (i64, i64) {
        if year <= 0 {
            return (0, 0);
        }
        let mut keys = self
            .monthly_snapshots
            .keys()
            .filter(|key| {
                is_valid_finance_month_key(key)
                    && key.get(0..4).and_then(|value| value.parse::<i32>().ok()) == Some(year)
            })
            .cloned()
            .collect::<Vec<_>>();
        keys.sort();
        let Some(first_key) = keys.first() else {
            return (0, 0);
        };
        let Some(last_key) = keys.last() else {
            return (0, 0);
        };
        let opening = self
            .monthly_snapshots
            .get(first_key)
            .map(FinanceMonthSnapshot::to_summary_totals)
            .unwrap_or_else(FinanceMonthSnapshotTotals::empty)
            .net_worth();
        let closing = self
            .monthly_snapshots
            .get(last_key)
            .map(FinanceMonthSnapshot::to_summary_totals)
            .unwrap_or_else(FinanceMonthSnapshotTotals::empty)
            .net_worth();
        (opening, closing)
    }

    fn most_problematic_target_drift(
        &self,
        aggregate: FinanceLedgerTotals,
    ) -> Option<(FinanceExpenseBucket, f32)> {
        if aggregate.income_total() <= 0 {
            return None;
        }
        primary_bucket_order()
            .into_iter()
            .filter_map(|bucket| {
                let target = self.settings.target_for(&bucket)?;
                let drift = aggregate.share_of_income(&bucket) - target;
                if is_problematic_target_drift(&bucket, drift) {
                    Some((bucket, drift))
                } else {
                    None
                }
            })
            .fold(None, |best, candidate| {
                Some(match best {
                    None => candidate,
                    Some(current) => {
                        if candidate.1.abs() > current.1.abs() {
                            candidate
                        } else {
                            current
                        }
                    }
                })
            })
    }
}

impl FinanceSettings {
    fn sanitized(self) -> Self {
        let incoming = self.expense_categories;
        let expense_categories = primary_bucket_order()
            .into_iter()
            .map(|bucket| {
                incoming
                    .iter()
                    .find(|entry| entry.bucket == bucket)
                    .cloned()
                    .unwrap_or_else(|| FinanceExpenseCategoryConfig {
                        label: bucket_default_label(&bucket).to_string(),
                        target_share_of_income: bucket_default_target(&bucket),
                        bucket,
                    })
                    .sanitized()
            })
            .collect();
        Self { expense_categories }
    }

    fn config_for(&self, bucket: FinanceExpenseBucket) -> FinanceExpenseCategoryConfig {
        self.expense_categories
            .iter()
            .find(|entry| entry.bucket == bucket)
            .cloned()
            .unwrap_or_else(|| FinanceExpenseCategoryConfig {
                bucket,
                label: bucket_default_label(&bucket).to_string(),
                target_share_of_income: bucket_default_target(&bucket),
            })
            .sanitized()
    }

    fn target_for(&self, bucket: &FinanceExpenseBucket) -> Option<f32> {
        self.expense_categories
            .iter()
            .find(|entry| entry.bucket == *bucket)
            .map(|entry| entry.target_share_of_income)
            .unwrap_or_else(|| bucket_default_target(bucket))
            .map(|value| value.clamp(0.0, 1.0))
    }

    fn label_for(&self, bucket: &FinanceExpenseBucket) -> String {
        self.expense_categories
            .iter()
            .find(|entry| entry.bucket == *bucket)
            .map(|entry| entry.label.clone())
            .filter(|label| !label.trim().is_empty())
            .unwrap_or_else(|| bucket_default_label(bucket).to_string())
    }
}

impl FinanceExpenseCategoryConfig {
    fn sanitized(self) -> Self {
        let fallback_label = bucket_default_label(&self.bucket);
        Self {
            bucket: self.bucket,
            label: truncate_chars(
                trimmed_or(&self.label, fallback_label),
                MAX_FINANCE_CATEGORY_LABEL_LENGTH,
            ),
            target_share_of_income: self
                .target_share_of_income
                .map(|value| value.clamp(0.0, 1.0)),
        }
    }
}

impl FinanceIncomeEntry {
    fn sanitized(self) -> Self {
        Self {
            name: truncate_chars(self.name.trim(), MAX_FINANCE_ENTRY_NAME_LENGTH),
            kind: self.kind,
            amount: sanitized_finance_amount(self.amount),
            note: compact_whitespace(&self.note, MAX_FINANCE_ENTRY_NOTE_LENGTH),
        }
    }

    fn has_entries(&self) -> bool {
        !self.name.is_empty() || self.amount > 0 || !self.note.is_empty()
    }
}

impl FinanceExpenseEntry {
    fn sanitized(self) -> Self {
        Self {
            name: truncate_chars(self.name.trim(), MAX_FINANCE_ENTRY_NAME_LENGTH),
            bucket: self.bucket,
            amount: sanitized_finance_amount(self.amount),
            note: compact_whitespace(&self.note, MAX_FINANCE_ENTRY_NOTE_LENGTH),
        }
    }

    fn has_entries(&self) -> bool {
        !self.name.is_empty() || self.amount > 0 || !self.note.is_empty()
    }
}

impl FinanceNamedAmountEntry {
    fn sanitized_asset(self) -> Self {
        let kind = if is_asset_kind(&self.kind) {
            self.kind
        } else {
            FinanceNamedAmountKind::OtherAsset
        };
        Self {
            name: truncate_chars(self.name.trim(), MAX_FINANCE_ENTRY_NAME_LENGTH),
            kind,
            amount: sanitized_finance_amount(self.amount),
        }
    }

    fn sanitized_liability(self) -> Self {
        let kind = if is_liability_kind(&self.kind) {
            self.kind
        } else {
            FinanceNamedAmountKind::OtherLiability
        };
        Self {
            name: truncate_chars(self.name.trim(), MAX_FINANCE_ENTRY_NAME_LENGTH),
            kind,
            amount: sanitized_finance_amount(self.amount),
        }
    }

    fn has_entries(&self) -> bool {
        !self.name.is_empty() || self.amount > 0
    }
}

impl FinanceDayLedger {
    fn sanitized(self) -> Self {
        let incomes = self
            .incomes
            .into_iter()
            .map(FinanceIncomeEntry::sanitized)
            .filter(FinanceIncomeEntry::has_entries)
            .collect::<Vec<_>>();
        let expenses = self
            .expenses
            .into_iter()
            .map(FinanceExpenseEntry::sanitized)
            .filter(FinanceExpenseEntry::has_entries)
            .collect::<Vec<_>>();
        Self {
            incomes: take_last(incomes, MAX_FINANCE_LEDGER_LINES),
            expenses: take_last(expenses, MAX_FINANCE_LEDGER_LINES),
            note: truncate_chars(
                self.note.replace("\r\n", "\n").trim(),
                MAX_FINANCE_LEDGER_NOTE_LENGTH,
            ),
        }
    }

    fn has_entries(&self) -> bool {
        !self.incomes.is_empty() || !self.expenses.is_empty() || !self.note.trim().is_empty()
    }

    fn to_aggregate_totals(&self) -> FinanceLedgerTotals {
        let mut aggregate = FinanceLedgerTotals::empty();
        for entry in &self.incomes {
            let amount = entry.amount.max(0);
            match entry.kind {
                FinanceIncomeKind::Active => {
                    aggregate.active_income_total =
                        aggregate.active_income_total.saturating_add(amount)
                }
                FinanceIncomeKind::Asset => {
                    aggregate.asset_income_total =
                        aggregate.asset_income_total.saturating_add(amount)
                }
                FinanceIncomeKind::Other => {
                    aggregate.other_income_total =
                        aggregate.other_income_total.saturating_add(amount)
                }
            }
        }
        for entry in &self.expenses {
            let amount = entry.amount.max(0);
            match entry.bucket {
                FinanceExpenseBucket::Debt => {
                    aggregate.debt_total = aggregate.debt_total.saturating_add(amount)
                }
                FinanceExpenseBucket::Food => {
                    aggregate.food_total = aggregate.food_total.saturating_add(amount)
                }
                FinanceExpenseBucket::Btc => {
                    aggregate.btc_total = aggregate.btc_total.saturating_add(amount)
                }
                FinanceExpenseBucket::Living => {
                    aggregate.living_total = aggregate.living_total.saturating_add(amount)
                }
                FinanceExpenseBucket::Learning => {
                    aggregate.learning_total = aggregate.learning_total.saturating_add(amount)
                }
                FinanceExpenseBucket::Other => {
                    aggregate.other_expense_total =
                        aggregate.other_expense_total.saturating_add(amount)
                }
            }
        }
        let has_data = aggregate.income_total() > 0
            || aggregate.expense_total() > 0
            || !self.note.trim().is_empty();
        aggregate.days_with_entries = if has_data { 1 } else { 0 };
        aggregate
    }
}

impl FinanceMonthSnapshot {
    fn sanitized(self) -> Self {
        let assets = self
            .assets
            .into_iter()
            .map(FinanceNamedAmountEntry::sanitized_asset)
            .filter(FinanceNamedAmountEntry::has_entries)
            .collect::<Vec<_>>();
        let liabilities = self
            .liabilities
            .into_iter()
            .map(FinanceNamedAmountEntry::sanitized_liability)
            .filter(FinanceNamedAmountEntry::has_entries)
            .collect::<Vec<_>>();
        Self {
            assets: take_last(assets, MAX_FINANCE_LEDGER_LINES),
            liabilities: take_last(liabilities, MAX_FINANCE_LEDGER_LINES),
            note: truncate_chars(
                self.note.replace("\r\n", "\n").trim(),
                MAX_FINANCE_LEDGER_NOTE_LENGTH,
            ),
        }
    }

    fn has_entries(&self) -> bool {
        !self.assets.is_empty() || !self.liabilities.is_empty() || !self.note.trim().is_empty()
    }

    fn to_summary_totals(&self) -> FinanceMonthSnapshotTotals {
        let mut summary = FinanceMonthSnapshotTotals::empty();
        for entry in &self.assets {
            let amount = entry.amount.max(0);
            summary.asset_total = summary.asset_total.saturating_add(amount);
            match entry.kind {
                FinanceNamedAmountKind::CashReserve => {
                    summary.cash_reserve_total = summary.cash_reserve_total.saturating_add(amount)
                }
                FinanceNamedAmountKind::ProductiveAsset => {
                    summary.productive_asset_total =
                        summary.productive_asset_total.saturating_add(amount)
                }
                FinanceNamedAmountKind::OtherAsset
                | FinanceNamedAmountKind::LiabilityBalance
                | FinanceNamedAmountKind::OtherLiability => {}
            }
        }
        for entry in &self.liabilities {
            summary.liability_total = summary.liability_total.saturating_add(entry.amount.max(0));
        }
        summary
    }
}

fn sanitize_finance_day_ledgers(
    values: IndexMap<String, FinanceDayLedger>,
) -> IndexMap<String, FinanceDayLedger> {
    let mut entries = values
        .into_iter()
        .filter(|(key, _)| is_valid_finance_day_key(key))
        .map(|(key, value)| (key, value.sanitized()))
        .filter(|(_, value)| value.has_entries())
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| right.0.cmp(&left.0));
    entries
        .into_iter()
        .take(MAX_FINANCE_DAILY_RECORDS)
        .collect()
}

fn sanitize_finance_month_snapshots(
    values: IndexMap<String, FinanceMonthSnapshot>,
) -> IndexMap<String, FinanceMonthSnapshot> {
    let mut entries = values
        .into_iter()
        .filter(|(key, _)| is_valid_finance_month_key(key))
        .map(|(key, value)| (key, value.sanitized()))
        .filter(|(_, value)| value.has_entries())
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| right.0.cmp(&left.0));
    entries
        .into_iter()
        .take(MAX_FINANCE_MONTHLY_RECORDS)
        .collect()
}

fn default_finance_expense_category_configs() -> Vec<FinanceExpenseCategoryConfig> {
    primary_bucket_order()
        .into_iter()
        .map(|bucket| FinanceExpenseCategoryConfig {
            label: bucket_default_label(&bucket).to_string(),
            target_share_of_income: bucket_default_target(&bucket),
            bucket,
        })
        .collect()
}

fn default_finance_expense_entries() -> Vec<FinanceExpenseEntry> {
    vec![
        FinanceExpenseEntry {
            name: "\u{503a}\u{52a1}\u{538b}\u{964d}".to_string(),
            bucket: FinanceExpenseBucket::Debt,
            amount: 0,
            note: String::new(),
        },
        FinanceExpenseEntry {
            name: "\u{65e5}\u{5e38}\u{6d88}\u{8d39}".to_string(),
            bucket: FinanceExpenseBucket::Food,
            amount: 0,
            note: String::new(),
        },
        FinanceExpenseEntry {
            name: "\u{56fa}\u{5b9a}\u{652f}\u{51fa}".to_string(),
            bucket: FinanceExpenseBucket::Living,
            amount: 0,
            note: String::new(),
        },
        FinanceExpenseEntry {
            name: "\u{957f}\u{671f}\u{914d}\u{7f6e}".to_string(),
            bucket: FinanceExpenseBucket::Btc,
            amount: 0,
            note: String::new(),
        },
        FinanceExpenseEntry {
            name: "\u{6210}\u{957f}\u{6295}\u{5165}".to_string(),
            bucket: FinanceExpenseBucket::Learning,
            amount: 0,
            note: String::new(),
        },
    ]
}

fn default_finance_income_entries() -> Vec<FinanceIncomeEntry> {
    vec![
        FinanceIncomeEntry {
            name: "\u{5de5}\u{8d44}\u{6536}\u{5165}".to_string(),
            kind: FinanceIncomeKind::Active,
            amount: 0,
            note: String::new(),
        },
        FinanceIncomeEntry {
            name: "\u{526f}\u{4e1a}\u{6536}\u{5165}".to_string(),
            kind: FinanceIncomeKind::Active,
            amount: 0,
            note: String::new(),
        },
        FinanceIncomeEntry {
            name: "\u{88ab}\u{52a8}\u{6536}\u{5165}".to_string(),
            kind: FinanceIncomeKind::Asset,
            amount: 0,
            note: String::new(),
        },
        FinanceIncomeEntry {
            name: "\u{5176}\u{4ed6}\u{6536}\u{5165}".to_string(),
            kind: FinanceIncomeKind::Other,
            amount: 0,
            note: String::new(),
        },
    ]
}

fn default_finance_asset_entries() -> Vec<FinanceNamedAmountEntry> {
    vec![
        FinanceNamedAmountEntry {
            name: "\u{73b0}\u{91d1}\u{4e0e}\u{6d3b}\u{671f}\u{5b58}\u{6b3e}".to_string(),
            kind: FinanceNamedAmountKind::CashReserve,
            amount: 0,
        },
        FinanceNamedAmountEntry {
            name: "\u{80a1}\u{7968} / \u{57fa}\u{91d1} / \u{52a0}\u{5bc6}\u{8d44}\u{4ea7}"
                .to_string(),
            kind: FinanceNamedAmountKind::ProductiveAsset,
            amount: 0,
        },
        FinanceNamedAmountEntry {
            name: "\u{7ecf}\u{8425}\u{6027}\u{8d44}\u{4ea7}\u{51c0}\u{503c}".to_string(),
            kind: FinanceNamedAmountKind::ProductiveAsset,
            amount: 0,
        },
        FinanceNamedAmountEntry {
            name: "\u{5176}\u{4ed6}\u{8d44}\u{4ea7}".to_string(),
            kind: FinanceNamedAmountKind::OtherAsset,
            amount: 0,
        },
    ]
}

fn default_finance_liability_entries() -> Vec<FinanceNamedAmountEntry> {
    vec![
        FinanceNamedAmountEntry {
            name: "\u{623f}\u{8d37} / \u{79df}\u{8d41}\u{8d1f}\u{503a}".to_string(),
            kind: FinanceNamedAmountKind::LiabilityBalance,
            amount: 0,
        },
        FinanceNamedAmountEntry {
            name: "\u{4fe1}\u{7528}\u{5361}\u{6b20}\u{6b3e}".to_string(),
            kind: FinanceNamedAmountKind::LiabilityBalance,
            amount: 0,
        },
        FinanceNamedAmountEntry {
            name: "\u{6d88}\u{8d39}\u{8d37}\u{6b3e} / \u{5206}\u{671f}\u{4ed8}\u{6b3e}".to_string(),
            kind: FinanceNamedAmountKind::LiabilityBalance,
            amount: 0,
        },
        FinanceNamedAmountEntry {
            name: "\u{4e2a}\u{4eba}\u{503a}\u{52a1}".to_string(),
            kind: FinanceNamedAmountKind::OtherLiability,
            amount: 0,
        },
    ]
}

fn primary_bucket_order() -> Vec<FinanceExpenseBucket> {
    vec![
        FinanceExpenseBucket::Debt,
        FinanceExpenseBucket::Food,
        FinanceExpenseBucket::Btc,
        FinanceExpenseBucket::Living,
        FinanceExpenseBucket::Learning,
        FinanceExpenseBucket::Other,
    ]
}

fn bucket_default_label(bucket: &FinanceExpenseBucket) -> &'static str {
    match bucket {
        FinanceExpenseBucket::Debt => "\u{503a}\u{52a1}\u{538b}\u{964d}",
        FinanceExpenseBucket::Food => "\u{65e5}\u{5e38}\u{6d88}\u{8d39}",
        FinanceExpenseBucket::Btc => "\u{957f}\u{671f}\u{914d}\u{7f6e}",
        FinanceExpenseBucket::Living => "\u{56fa}\u{5b9a}\u{652f}\u{51fa}",
        FinanceExpenseBucket::Learning => "\u{6210}\u{957f}\u{6295}\u{5165}",
        FinanceExpenseBucket::Other => "\u{673a}\u{52a8}\u{9884}\u{7559}",
    }
}

fn bucket_default_target(bucket: &FinanceExpenseBucket) -> Option<f32> {
    match bucket {
        FinanceExpenseBucket::Debt => Some(0.25),
        FinanceExpenseBucket::Food => Some(0.18),
        FinanceExpenseBucket::Btc => Some(0.12),
        FinanceExpenseBucket::Living => Some(0.30),
        FinanceExpenseBucket::Learning => Some(0.15),
        FinanceExpenseBucket::Other => None,
    }
}

fn default_income_kind() -> FinanceIncomeKind {
    FinanceIncomeKind::Active
}

fn default_expense_bucket() -> FinanceExpenseBucket {
    FinanceExpenseBucket::Other
}

fn default_named_amount_kind() -> FinanceNamedAmountKind {
    FinanceNamedAmountKind::OtherAsset
}

fn default_finance_backup_schema_version() -> i32 {
    FINANCE_BACKUP_SCHEMA_VERSION
}

fn is_asset_kind(kind: &FinanceNamedAmountKind) -> bool {
    matches!(
        kind,
        FinanceNamedAmountKind::CashReserve
            | FinanceNamedAmountKind::ProductiveAsset
            | FinanceNamedAmountKind::OtherAsset
    )
}

fn is_liability_kind(kind: &FinanceNamedAmountKind) -> bool {
    matches!(
        kind,
        FinanceNamedAmountKind::LiabilityBalance | FinanceNamedAmountKind::OtherLiability
    )
}

fn sanitized_finance_amount(value: i64) -> i64 {
    value.clamp(0, MAX_FINANCE_AMOUNT)
}

fn compact_whitespace(value: &str, max_chars: usize) -> String {
    truncate_chars(
        &value
            .split_whitespace()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        max_chars,
    )
}

fn trimmed_or<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback
    } else {
        trimmed
    }
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn take_last<T>(values: Vec<T>, limit: usize) -> Vec<T> {
    let skip = values.len().saturating_sub(limit);
    values.into_iter().skip(skip).collect()
}

fn parse_finance_day_key(value: &str) -> Option<(i32, i32, i32)> {
    if !is_valid_finance_day_key(value) {
        return None;
    }
    let bytes = value.as_bytes();
    Some((
        parse_fixed_u32(&bytes[0..4]) as i32,
        parse_fixed_u32(&bytes[5..7]) as i32,
        parse_fixed_u32(&bytes[8..10]) as i32,
    ))
}

#[allow(dead_code)]
fn parse_finance_month_key(value: &str) -> Option<(i32, i32)> {
    if !is_valid_finance_month_key(value) {
        return None;
    }
    let bytes = value.as_bytes();
    Some((
        parse_fixed_u32(&bytes[0..4]) as i32,
        parse_fixed_u32(&bytes[5..7]) as i32,
    ))
}

fn date_matches_period(
    period_code: i32,
    year: i32,
    month: i32,
    day: i32,
    reference_year: i32,
    reference_month: i32,
    reference_day: i32,
) -> bool {
    match period_code {
        PERIOD_DAY => year == reference_year && month == reference_month && day == reference_day,
        PERIOD_MONTH => year == reference_year && month == reference_month,
        PERIOD_QUARTER => {
            year == reference_year && quarter_for_month(month) == quarter_for_month(reference_month)
        }
        PERIOD_YEAR => year == reference_year,
        _ => false,
    }
}

fn previous_reference_date(period_code: i32, year: i32, month: i32, day: i32) -> (i32, i32, i32) {
    match period_code {
        PERIOD_DAY => add_days(year, month, day, -1),
        PERIOD_MONTH => add_months(year, month, day, -1),
        PERIOD_QUARTER => add_months(year, month, day, -3),
        PERIOD_YEAR => add_months(year, month, day, -12),
        _ => (year, month, day),
    }
}

fn expected_recorded_days(period_code: i32, year: i32, month: i32, day: i32) -> i32 {
    match period_code {
        PERIOD_DAY => 1,
        PERIOD_MONTH => day.max(1),
        PERIOD_QUARTER => {
            let quarter_start_month = ((month - 1) / 3) * 3 + 1;
            let current = days_from_civil(year as i64, month as i64, day as i64);
            let start = days_from_civil(year as i64, quarter_start_month as i64, 1);
            (current - start + 1).max(0) as i32
        }
        PERIOD_YEAR => {
            let current = days_from_civil(year as i64, month as i64, day as i64);
            let start = days_from_civil(year as i64, 1, 1);
            (current - start + 1).max(0) as i32
        }
        _ => 0,
    }
}

fn snapshot_values_from_totals(
    aggregate: FinanceLedgerTotals,
    summary: FinanceMonthSnapshotTotals,
    recorded_days: i64,
) -> FinanceSnapshotValues {
    let freedom_gap = aggregate
        .expense_total()
        .saturating_sub(aggregate.asset_income_total)
        .max(0);
    let defensive_base = aggregate.expense_total() - aggregate.asset_income_total;
    let defensive_coverage = if summary.cash_reserve_total <= 0 {
        Some(0.0)
    } else if defensive_base <= 0 {
        None
    } else {
        Some(summary.cash_reserve_total as f32 / defensive_base as f32)
    };

    FinanceSnapshotValues {
        total_income: aggregate.income_total(),
        total_outflow: aggregate.expense_total(),
        net_cashflow: aggregate.net_cashflow(),
        freedom_gap,
        passive_coverage_ratio: safe_finance_ratio(
            aggregate.asset_income_total,
            aggregate.expense_total(),
        ),
        wage_dependence_ratio: safe_finance_ratio(
            aggregate.active_income_total,
            aggregate.income_total(),
        ),
        liability_pressure_ratio: safe_finance_ratio(
            aggregate.debt_total,
            aggregate.income_total(),
        ),
        net_worth: summary.net_worth(),
        defensive_coverage,
        asset_yield_ratio: safe_finance_ratio(
            aggregate.asset_income_total,
            summary.productive_asset_total,
        ),
        recorded_days,
    }
}

fn safe_finance_ratio(numerator: i64, denominator: i64) -> f32 {
    if numerator <= 0 || denominator <= 0 {
        0.0
    } else {
        numerator as f32 / denominator as f32
    }
}

fn quarter_for_month(month: i32) -> i32 {
    ((month - 1) / 3) + 1
}

fn add_days(year: i32, month: i32, day: i32, offset: i64) -> (i32, i32, i32) {
    let days = days_from_civil(year as i64, month as i64, day as i64) + offset;
    let (next_year, next_month, next_day) = civil_from_days(days);
    (next_year as i32, next_month as i32, next_day as i32)
}

fn add_months(year: i32, month: i32, day: i32, offset: i32) -> (i32, i32, i32) {
    let month_index = year * 12 + (month - 1) + offset;
    let next_year = month_index.div_euclid(12);
    let next_month = month_index.rem_euclid(12) + 1;
    let next_day = day.min(days_in_month(next_year, next_month));
    (next_year, next_month, next_day)
}

fn days_from_civil(mut year: i64, month: i64, day: i64) -> i64 {
    year -= (month <= 2) as i64;
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * month_prime + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += (month <= 2) as i64;
    (year, month, day)
}

fn days_in_month(year: i32, month: i32) -> i32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 30,
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn append_missing_templates<T, F>(mut rows: Vec<T>, templates: Vec<T>, key_for: F) -> Vec<T>
where
    T: Clone,
    F: Fn(&T) -> Option<(i32, String)>,
{
    let mut present = HashSet::<(i32, String)>::new();
    for row in &rows {
        if let Some(key) = key_for(row) {
            present.insert(key);
        }
    }
    for template in templates {
        if let Some(key) = key_for(&template) {
            if present.insert(key) {
                rows.push(template);
            }
        }
    }
    rows
}

fn edit_finance_rows<T>(
    operation_code: i32,
    rows_json: &str,
    index: i32,
    entry_json: &str,
) -> Option<String>
where
    T: Clone + for<'de> Deserialize<'de> + Serialize,
{
    let mut rows = serde_json::from_str::<Vec<T>>(rows_json).ok()?;
    match operation_code {
        0 => {
            let entry = serde_json::from_str::<T>(entry_json).ok()?;
            rows.push(entry);
        }
        1 => {
            let row_index = usize::try_from(index).ok()?;
            if row_index >= rows.len() {
                return None;
            }
            let entry = serde_json::from_str::<T>(entry_json).ok()?;
            rows[row_index] = entry;
        }
        2 => {
            let row_index = usize::try_from(index).ok()?;
            if row_index >= rows.len() {
                return None;
            }
            rows.remove(row_index);
        }
        _ => return None,
    }
    serde_json::to_string(&rows).ok()
}

fn template_key(kind_code: i32, name: &str) -> Option<(i32, String)> {
    let normalized = name.trim().to_lowercase();
    if normalized.is_empty() {
        None
    } else {
        Some((kind_code, normalized))
    }
}

fn bucket_code(bucket: &FinanceExpenseBucket) -> i32 {
    match bucket {
        FinanceExpenseBucket::Debt => BUCKET_CODE_DEBT,
        FinanceExpenseBucket::Food => BUCKET_CODE_FOOD,
        FinanceExpenseBucket::Btc => BUCKET_CODE_BTC,
        FinanceExpenseBucket::Living => BUCKET_CODE_LIVING,
        FinanceExpenseBucket::Learning => BUCKET_CODE_LEARNING,
        FinanceExpenseBucket::Other => BUCKET_CODE_OTHER,
    }
}

fn income_kind_code(kind: &FinanceIncomeKind) -> i32 {
    match kind {
        FinanceIncomeKind::Active => 0,
        FinanceIncomeKind::Asset => 1,
        FinanceIncomeKind::Other => 2,
    }
}

fn named_amount_kind_code(kind: &FinanceNamedAmountKind) -> i32 {
    match kind {
        FinanceNamedAmountKind::CashReserve => 0,
        FinanceNamedAmountKind::ProductiveAsset => 1,
        FinanceNamedAmountKind::OtherAsset => 2,
        FinanceNamedAmountKind::LiabilityBalance => 3,
        FinanceNamedAmountKind::OtherLiability => 4,
    }
}

fn bucket_from_code(bucket_code: i32) -> Option<FinanceExpenseBucket> {
    match bucket_code {
        BUCKET_CODE_DEBT => Some(FinanceExpenseBucket::Debt),
        BUCKET_CODE_FOOD => Some(FinanceExpenseBucket::Food),
        BUCKET_CODE_BTC => Some(FinanceExpenseBucket::Btc),
        BUCKET_CODE_LIVING => Some(FinanceExpenseBucket::Living),
        BUCKET_CODE_LEARNING => Some(FinanceExpenseBucket::Learning),
        BUCKET_CODE_OTHER => Some(FinanceExpenseBucket::Other),
        _ => None,
    }
}

fn period_scope_label(period_code: i32) -> &'static str {
    match period_code {
        PERIOD_DAY => "今天",
        PERIOD_QUARTER => "本季度",
        PERIOD_YEAR => "今年",
        _ => "本月",
    }
}

fn period_coverage_unit_label(period_code: i32) -> &'static str {
    match period_code {
        PERIOD_DAY => "天",
        PERIOD_QUARTER => "季",
        PERIOD_YEAR => "年",
        _ => "月",
    }
}

fn format_model_ratio_percent(value: f32) -> String {
    format!("{:.0}%", value * 100.0)
}

fn is_problematic_target_drift(bucket: &FinanceExpenseBucket, drift: f32) -> bool {
    match bucket {
        FinanceExpenseBucket::Debt | FinanceExpenseBucket::Food | FinanceExpenseBucket::Living => {
            drift > 0.08
        }
        FinanceExpenseBucket::Btc | FinanceExpenseBucket::Learning => drift < -0.08,
        FinanceExpenseBucket::Other => drift.abs() > 0.08,
    }
}

fn is_valid_finance_day_key(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes[0..4].iter().all(u8::is_ascii_digit)
        || !bytes[5..7].iter().all(u8::is_ascii_digit)
        || !bytes[8..10].iter().all(u8::is_ascii_digit)
    {
        return false;
    }
    let year = parse_fixed_u32(&bytes[0..4]);
    let month = parse_fixed_u32(&bytes[5..7]);
    let day = parse_fixed_u32(&bytes[8..10]);
    is_valid_ymd(year, month, day)
}

fn is_valid_finance_month_key(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 7
        || bytes[4] != b'-'
        || !bytes[0..4].iter().all(u8::is_ascii_digit)
        || !bytes[5..7].iter().all(u8::is_ascii_digit)
    {
        return false;
    }
    let year = parse_fixed_u32(&bytes[0..4]);
    let month = parse_fixed_u32(&bytes[5..7]);
    year > 0 && (1..=12).contains(&month)
}

fn parse_fixed_u32(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .fold(0, |acc, value| acc * 10 + (value - b'0') as u32)
}

fn is_valid_ymd(year: u32, month: u32, day: u32) -> bool {
    if year == 0 || !(1..=12).contains(&month) || day == 0 {
        return false;
    }
    day <= days_in_month(year as i32, month as i32) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_amounts_and_compacts_focus_text() {
        let profile = FinanceProfile {
            active_income_monthly: i64::MAX,
            asset_income_monthly: -1,
            acquisition_focus: "  Dividend     snowball  ".to_string(),
            liability_focus: "  Loan     trim  ".to_string(),
            ..empty_profile()
        }
        .sanitized();

        assert_eq!(MAX_FINANCE_AMOUNT, profile.active_income_monthly);
        assert_eq!(0, profile.asset_income_monthly);
        assert_eq!("Dividend snowball", profile.acquisition_focus);
        assert_eq!("Loan trim", profile.liability_focus);
    }

    #[test]
    fn profile_summary_flags_report_basic_and_detailed_data() {
        let mut daily_ledgers = IndexMap::new();
        daily_ledgers.insert(
            "2026-04-08".to_string(),
            FinanceDayLedger {
                incomes: vec![FinanceIncomeEntry {
                    name: "Salary".to_string(),
                    amount: 100,
                    ..default_income_entry()
                }],
                ..FinanceDayLedger::default()
            },
        );
        let profile = FinanceProfile {
            daily_ledgers,
            ..empty_profile()
        };
        let raw = serde_json::to_string(&profile).unwrap();

        assert_eq!(Some([1, 1]), profile_summary_flags(&raw));
        assert_eq!(Some([0, 0]), profile_summary_flags("{}"));
    }

    #[test]
    fn alert_argument_values_include_render_parameters() {
        let mut daily_ledgers = IndexMap::new();
        daily_ledgers.insert(
            "2026-04-18".to_string(),
            FinanceDayLedger {
                incomes: vec![FinanceIncomeEntry {
                    name: "Salary".to_string(),
                    amount: 1_000,
                    ..default_income_entry()
                }],
                expenses: vec![FinanceExpenseEntry {
                    name: "Debt".to_string(),
                    bucket: FinanceExpenseBucket::Debt,
                    amount: 700,
                    ..default_expense_entry()
                }],
                ..FinanceDayLedger::default()
            },
        );
        let profile = FinanceProfile {
            daily_ledgers,
            settings: FinanceSettings {
                expense_categories: vec![FinanceExpenseCategoryConfig {
                    bucket: FinanceExpenseBucket::Debt,
                    label: "还债".to_string(),
                    target_share_of_income: Some(0.30),
                }],
            },
            ..empty_profile()
        };
        let raw = serde_json::to_string(&profile).unwrap();

        let values = build_finance_alert_argument_values(&raw, PERIOD_MONTH, 2026, 4, 18)
            .expect("alert args");

        assert_eq!(0, values.len() % 10);
        let target_drift = values
            .chunks(10)
            .find(|chunk| chunk[0] == ALERT_KIND_TARGET_DRIFT.to_string())
            .expect("target drift");
        assert_eq!(BUCKET_CODE_DEBT.to_string(), target_drift[1]);
        assert_eq!("高于", target_drift[3]);
        assert_eq!("70%", target_drift[4]);
        assert_eq!("30%", target_drift[5]);
        assert_eq!("还债", target_drift[6]);
        assert_eq!("本月", target_drift[7]);
        assert_eq!("月", target_drift[8]);
        assert_eq!("1", target_drift[9]);
    }

    #[test]
    fn removes_invalid_ledger_keys_and_blank_rows() {
        let mut daily_ledgers = IndexMap::new();
        daily_ledgers.insert(
            "bad-key".to_string(),
            FinanceDayLedger {
                incomes: vec![FinanceIncomeEntry {
                    name: "Ghost".to_string(),
                    amount: 100,
                    ..default_income_entry()
                }],
                ..FinanceDayLedger::default()
            },
        );
        daily_ledgers.insert(
            "2026-02-31".to_string(),
            FinanceDayLedger {
                incomes: vec![FinanceIncomeEntry {
                    name: "Impossible".to_string(),
                    amount: 100,
                    ..default_income_entry()
                }],
                ..FinanceDayLedger::default()
            },
        );
        daily_ledgers.insert(
            "2026-04-08".to_string(),
            FinanceDayLedger {
                incomes: vec![FinanceIncomeEntry {
                    name: " Salary ".to_string(),
                    amount: 1_200,
                    ..default_income_entry()
                }],
                expenses: vec![
                    FinanceExpenseEntry {
                        name: "Debt".to_string(),
                        bucket: FinanceExpenseBucket::Debt,
                        amount: 300,
                        ..default_expense_entry()
                    },
                    default_expense_entry(),
                ],
                note: " steady ".to_string(),
            },
        );

        let sanitized = FinanceProfile {
            daily_ledgers,
            ..empty_profile()
        }
        .sanitized();

        assert_eq!(1, sanitized.daily_ledgers.len());
        let ledger = sanitized.daily_ledgers.get("2026-04-08").unwrap();
        assert_eq!(1, ledger.incomes.len());
        assert_eq!(1, ledger.expenses.len());
        assert_eq!("Salary", ledger.incomes[0].name);
        assert_eq!("steady", ledger.note);
    }

    #[test]
    fn sanitizes_snapshot_kinds_and_month_keys() {
        let mut monthly_snapshots = IndexMap::new();
        monthly_snapshots.insert(
            "2026-04".to_string(),
            FinanceMonthSnapshot {
                assets: vec![FinanceNamedAmountEntry {
                    name: " Card ".to_string(),
                    kind: FinanceNamedAmountKind::LiabilityBalance,
                    amount: 800,
                }],
                liabilities: vec![FinanceNamedAmountEntry {
                    name: " Cash ".to_string(),
                    kind: FinanceNamedAmountKind::CashReserve,
                    amount: 1_000,
                }],
                note: "\r\n month end ".to_string(),
            },
        );
        monthly_snapshots.insert(
            "2026-13".to_string(),
            FinanceMonthSnapshot {
                assets: vec![FinanceNamedAmountEntry {
                    name: "Bad".to_string(),
                    amount: 1,
                    ..default_named_amount_entry()
                }],
                ..FinanceMonthSnapshot::default()
            },
        );

        let sanitized = FinanceProfile {
            monthly_snapshots,
            ..empty_profile()
        }
        .sanitized();

        assert_eq!(1, sanitized.monthly_snapshots.len());
        let snapshot = sanitized.monthly_snapshots.get("2026-04").unwrap();
        assert_eq!(FinanceNamedAmountKind::OtherAsset, snapshot.assets[0].kind);
        assert_eq!(
            FinanceNamedAmountKind::OtherLiability,
            snapshot.liabilities[0].kind
        );
        assert_eq!("month end", snapshot.note);
    }

    #[test]
    fn upsert_day_ledger_json_sanitizes_and_removes_blank_ledger() {
        let profile = empty_profile();
        let profile_json = serde_json::to_string(&profile).unwrap();
        let ledger_json = r#"{
            "incomes":[{"name":" Salary ","kind":"ACTIVE","amount":1200,"note":" base   pay "}],
            "expenses":[{"name":"Food","bucket":"FOOD","amount":300,"note":""}],
            "note":" day note "
        }"#;

        let updated_json =
            upsert_finance_day_ledger_json(&profile_json, "2026-04-08", ledger_json).unwrap();
        let updated = serde_json::from_str::<FinanceProfile>(&updated_json).unwrap();
        let ledger = updated.daily_ledgers.get("2026-04-08").unwrap();
        assert_eq!("Salary", ledger.incomes[0].name);
        assert_eq!(1200, ledger.incomes[0].amount);
        assert_eq!("day note", ledger.note);

        let removed_json =
            upsert_finance_day_ledger_json(&updated_json, "2026-04-08", "{}").unwrap();
        let removed = serde_json::from_str::<FinanceProfile>(&removed_json).unwrap();
        assert!(!removed.daily_ledgers.contains_key("2026-04-08"));
    }

    #[test]
    fn upsert_month_snapshot_and_profile_aggregate_use_json_payloads() {
        let mut daily_ledgers = IndexMap::new();
        daily_ledgers.insert(
            "2026-04-08".to_string(),
            FinanceDayLedger {
                incomes: vec![FinanceIncomeEntry {
                    name: "Salary".to_string(),
                    amount: 2_000,
                    ..default_income_entry()
                }],
                expenses: vec![FinanceExpenseEntry {
                    name: "Food".to_string(),
                    bucket: FinanceExpenseBucket::Food,
                    amount: 500,
                    ..default_expense_entry()
                }],
                ..FinanceDayLedger::default()
            },
        );
        let profile = FinanceProfile {
            daily_ledgers,
            ..empty_profile()
        };
        let profile_json = serde_json::to_string(&profile).unwrap();
        let snapshot_json = r#"{
            "assets":[{"name":"Cash","kind":"CASH_RESERVE","amount":3000}],
            "liabilities":[{"name":"Card","kind":"LIABILITY_BALANCE","amount":800}]
        }"#;

        let updated_json =
            upsert_finance_month_snapshot_json(&profile_json, "2026-04", snapshot_json).unwrap();
        let updated = serde_json::from_str::<FinanceProfile>(&updated_json).unwrap();
        assert!(updated.monthly_snapshots.contains_key("2026-04"));

        let aggregate =
            aggregate_finance_ledger_values(&updated_json, PERIOD_MONTH, 2026, 4, 21).unwrap();
        assert_eq!(2_000, aggregate.income_total());
        assert_eq!(500, aggregate.expense_total());
        assert_eq!(1, aggregate.days_with_entries);
    }

    #[test]
    fn json_entrypoint_round_trips_sanitized_payload() {
        let raw = r#"{
            "activeIncomeMonthly":9223372036854775807,
            "assetIncomeMonthly":-5,
            "dailyLedgers":{
                "2026-04-08":{
                    "incomes":[{"name":" Salary ","kind":"ACTIVE","amount":1200,"note":" base   pay "}],
                    "expenses":[{"name":"","bucket":"OTHER","amount":0,"note":""}],
                    "note":" ok "
                },
                "2026-04-31":{"incomes":[{"name":"Bad","amount":1}]}
            }
        }"#;

        let sanitized = sanitize_finance_profile_json(raw).unwrap();
        let profile = serde_json::from_str::<FinanceProfile>(&sanitized).unwrap();

        assert_eq!(MAX_FINANCE_AMOUNT, profile.active_income_monthly);
        assert_eq!(0, profile.asset_income_monthly);
        assert!(profile.daily_ledgers.contains_key("2026-04-08"));
        assert!(!profile.daily_ledgers.contains_key("2026-04-31"));
    }

    #[test]
    fn default_finance_template_json_exports_canonical_rows() {
        let income_json = default_finance_income_entries_json().unwrap();
        let income_rows = serde_json::from_str::<Vec<FinanceIncomeEntry>>(&income_json).unwrap();
        assert_eq!(4, income_rows.len());
        assert_eq!("\u{5de5}\u{8d44}\u{6536}\u{5165}", income_rows[0].name);
        assert_eq!(FinanceIncomeKind::Asset, income_rows[2].kind);

        let expense_json = default_finance_expense_entries_json().unwrap();
        let expense_rows = serde_json::from_str::<Vec<FinanceExpenseEntry>>(&expense_json).unwrap();
        assert_eq!(5, expense_rows.len());
        assert_eq!(FinanceExpenseBucket::Debt, expense_rows[0].bucket);
        assert_eq!(FinanceExpenseBucket::Learning, expense_rows[4].bucket);

        let asset_json = default_finance_asset_entries_json().unwrap();
        let asset_rows = serde_json::from_str::<Vec<FinanceNamedAmountEntry>>(&asset_json).unwrap();
        assert_eq!(4, asset_rows.len());
        assert_eq!(FinanceNamedAmountKind::CashReserve, asset_rows[0].kind);

        let liability_json = default_finance_liability_entries_json().unwrap();
        let liability_rows =
            serde_json::from_str::<Vec<FinanceNamedAmountEntry>>(&liability_json).unwrap();
        assert_eq!(4, liability_rows.len());
        assert_eq!(
            FinanceNamedAmountKind::OtherLiability,
            liability_rows[3].kind
        );
    }

    #[test]
    fn append_missing_template_json_skips_present_blank_and_duplicate_rows() {
        let rows = vec![FinanceIncomeEntry {
            name: " Salary ".to_string(),
            kind: FinanceIncomeKind::Active,
            amount: 1_200,
            note: "paid".to_string(),
        }];
        let templates = vec![
            FinanceIncomeEntry {
                name: "salary".to_string(),
                kind: FinanceIncomeKind::Active,
                amount: 0,
                note: String::new(),
            },
            FinanceIncomeEntry {
                name: "Bonus".to_string(),
                kind: FinanceIncomeKind::Other,
                amount: 0,
                note: "annual".to_string(),
            },
            FinanceIncomeEntry {
                name: "bonus".to_string(),
                kind: FinanceIncomeKind::Other,
                amount: 0,
                note: "duplicate".to_string(),
            },
            FinanceIncomeEntry {
                name: "   ".to_string(),
                kind: FinanceIncomeKind::Asset,
                amount: 0,
                note: String::new(),
            },
        ];
        let updated = append_missing_income_templates_json(
            &serde_json::to_string(&rows).unwrap(),
            &serde_json::to_string(&templates).unwrap(),
        )
        .unwrap();
        let decoded = serde_json::from_str::<Vec<FinanceIncomeEntry>>(&updated).unwrap();
        assert_eq!(2, decoded.len());
        assert_eq!("Bonus", decoded[1].name);
        assert_eq!("annual", decoded[1].note);
    }

    #[test]
    fn finance_row_edit_json_applies_append_replace_and_remove_by_row_kind() {
        let income_rows = vec![FinanceIncomeEntry {
            name: "Salary".to_string(),
            kind: FinanceIncomeKind::Active,
            amount: 1_200,
            note: String::new(),
        }];
        let bonus = FinanceIncomeEntry {
            name: "Bonus".to_string(),
            kind: FinanceIncomeKind::Other,
            amount: 300,
            note: "annual".to_string(),
        };
        let appended = finance_row_edit_json(
            0,
            0,
            &serde_json::to_string(&income_rows).unwrap(),
            -1,
            &serde_json::to_string(&bonus).unwrap(),
        )
        .unwrap();
        let appended_rows = serde_json::from_str::<Vec<FinanceIncomeEntry>>(&appended).unwrap();
        assert_eq!(2, appended_rows.len());
        assert_eq!("Bonus", appended_rows[1].name);

        let expense_rows = vec![FinanceExpenseEntry {
            name: "Food".to_string(),
            bucket: FinanceExpenseBucket::Food,
            amount: 80,
            note: String::new(),
        }];
        let debt = FinanceExpenseEntry {
            name: "Debt".to_string(),
            bucket: FinanceExpenseBucket::Debt,
            amount: 100,
            note: "card".to_string(),
        };
        let replaced = finance_row_edit_json(
            1,
            1,
            &serde_json::to_string(&expense_rows).unwrap(),
            0,
            &serde_json::to_string(&debt).unwrap(),
        )
        .unwrap();
        let replaced_rows = serde_json::from_str::<Vec<FinanceExpenseEntry>>(&replaced).unwrap();
        assert_eq!(FinanceExpenseBucket::Debt, replaced_rows[0].bucket);
        assert!(finance_row_edit_json(
            1,
            1,
            &serde_json::to_string(&expense_rows).unwrap(),
            4,
            &serde_json::to_string(&debt).unwrap(),
        )
        .is_none());

        let named_rows = vec![
            FinanceNamedAmountEntry {
                name: "Cash".to_string(),
                kind: FinanceNamedAmountKind::CashReserve,
                amount: 1_000,
            },
            FinanceNamedAmountEntry {
                name: "Loan".to_string(),
                kind: FinanceNamedAmountKind::LiabilityBalance,
                amount: 500,
            },
        ];
        let removed =
            finance_row_edit_json(2, 2, &serde_json::to_string(&named_rows).unwrap(), 0, "")
                .unwrap();
        let removed_rows = serde_json::from_str::<Vec<FinanceNamedAmountEntry>>(&removed).unwrap();
        assert_eq!(1, removed_rows.len());
        assert_eq!("Loan", removed_rows[0].name);
    }

    #[test]
    fn encode_finance_backup_json_sanitizes_profile_and_metadata() {
        let profile = FinanceProfile {
            active_income_monthly: i64::MAX,
            acquisition_focus: "  Build    assets ".to_string(),
            ..empty_profile()
        };
        let payload_json =
            encode_finance_backup_json(&serde_json::to_string(&profile).unwrap(), " 2.10.6 ", -5)
                .unwrap();
        let payload = serde_json::from_str::<FinanceBackupPayload>(&payload_json).unwrap();
        assert_eq!(FINANCE_BACKUP_SCHEMA_VERSION, payload.schema_version);
        assert_eq!(0, payload.exported_at_epoch_millis);
        assert_eq!("2.10.6", payload.app_version_name);
        assert_eq!(
            MAX_FINANCE_AMOUNT,
            payload.finance_profile.active_income_monthly
        );
        assert_eq!("Build assets", payload.finance_profile.acquisition_focus);
    }

    #[test]
    fn day_ledger_and_month_snapshot_default_json_keep_lookup_rules_native() {
        let mut daily_ledgers = IndexMap::new();
        daily_ledgers.insert(
            "2026-04-08".to_string(),
            FinanceDayLedger {
                incomes: vec![FinanceIncomeEntry {
                    name: "Salary".to_string(),
                    kind: FinanceIncomeKind::Active,
                    amount: 800,
                    note: String::new(),
                }],
                ..FinanceDayLedger::default()
            },
        );
        let mut monthly_snapshots = IndexMap::new();
        monthly_snapshots.insert(
            "2026-04".to_string(),
            FinanceMonthSnapshot {
                assets: vec![FinanceNamedAmountEntry {
                    name: "Cash".to_string(),
                    kind: FinanceNamedAmountKind::CashReserve,
                    amount: 2_000,
                }],
                ..FinanceMonthSnapshot::default()
            },
        );
        let profile_json = serde_json::to_string(&FinanceProfile {
            daily_ledgers,
            monthly_snapshots,
            ..empty_profile()
        })
        .unwrap();

        let ledger = finance_day_ledger_or_default_json(&profile_json, "2026-04-08").unwrap();
        let parsed_ledger = serde_json::from_str::<FinanceDayLedger>(&ledger).unwrap();
        assert_eq!(800, parsed_ledger.incomes[0].amount);

        let empty_ledger = finance_day_ledger_or_default_json(&profile_json, "2026-04-09").unwrap();
        assert!(serde_json::from_str::<FinanceDayLedger>(&empty_ledger)
            .unwrap()
            .incomes
            .is_empty());
        assert!(finance_day_ledger_or_default_json(&profile_json, "2026-04-31").is_none());

        let snapshot = finance_month_snapshot_or_default_json(&profile_json, "2026-04").unwrap();
        let parsed_snapshot = serde_json::from_str::<FinanceMonthSnapshot>(&snapshot).unwrap();
        assert_eq!(2_000, parsed_snapshot.assets[0].amount);

        let empty_snapshot =
            finance_month_snapshot_or_default_json(&profile_json, "2026-05").unwrap();
        assert!(
            serde_json::from_str::<FinanceMonthSnapshot>(&empty_snapshot)
                .unwrap()
                .assets
                .is_empty()
        );
        assert!(finance_month_snapshot_or_default_json(&profile_json, "2026-13").is_none());
    }

    #[test]
    fn settings_config_json_and_replacement_follow_sanitized_bucket_order() {
        let settings_json = r#"{
            "expenseCategories": [
                {"bucket":"FOOD","label":" Food Budget ","targetShareOfIncome":2.0},
                {"bucket":"DEBT","label":"","targetShareOfIncome":0.25}
            ]
        }"#;

        let food_config = finance_settings_config_for_json(settings_json, 1).unwrap();
        let food = serde_json::from_str::<FinanceExpenseCategoryConfig>(&food_config).unwrap();
        assert_eq!(FinanceExpenseBucket::Food, food.bucket);
        assert_eq!("Food Budget", food.label);
        assert_eq!(Some(1.0), food.target_share_of_income);

        let replacement_json =
            r#"{"bucket":"OTHER","label":"Long Long Learning Label","targetShareOfIncome":-0.5}"#;
        let updated =
            replace_finance_expense_category_config_json(settings_json, 4, replacement_json)
                .unwrap();
        let updated_settings = serde_json::from_str::<FinanceSettings>(&updated).unwrap();
        assert_eq!(6, updated_settings.expense_categories.len());
        let learning = updated_settings
            .expense_categories
            .iter()
            .find(|config| config.bucket == FinanceExpenseBucket::Learning)
            .unwrap();
        assert_eq!(FinanceExpenseBucket::Learning, learning.bucket);
        assert_eq!("Long Long Le", learning.label);
        assert_eq!(Some(0.0), learning.target_share_of_income);

        let defaults = default_finance_expense_category_configs_json().unwrap();
        assert_eq!(
            6,
            serde_json::from_str::<Vec<FinanceExpenseCategoryConfig>>(&defaults)
                .unwrap()
                .len()
        );
    }

    fn empty_profile() -> FinanceProfile {
        FinanceProfile {
            active_income_monthly: 0,
            asset_income_monthly: 0,
            living_expense_monthly: 0,
            liability_payment_monthly: 0,
            cash_reserve: 0,
            productive_asset_value: 0,
            liability_balance: 0,
            acquisition_focus: String::new(),
            liability_focus: String::new(),
            settings: FinanceSettings::default(),
            daily_ledgers: IndexMap::new(),
            monthly_snapshots: IndexMap::new(),
        }
    }

    fn default_income_entry() -> FinanceIncomeEntry {
        FinanceIncomeEntry {
            name: String::new(),
            kind: FinanceIncomeKind::Active,
            amount: 0,
            note: String::new(),
        }
    }

    fn default_expense_entry() -> FinanceExpenseEntry {
        FinanceExpenseEntry {
            name: String::new(),
            bucket: FinanceExpenseBucket::Other,
            amount: 0,
            note: String::new(),
        }
    }

    fn default_named_amount_entry() -> FinanceNamedAmountEntry {
        FinanceNamedAmountEntry {
            name: String::new(),
            kind: FinanceNamedAmountKind::OtherAsset,
            amount: 0,
        }
    }
}
