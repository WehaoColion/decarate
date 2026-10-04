// v1.0.3 Windows - Build the desktop overview from one decoded finance profile.
use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

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
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub legacy_amount_minor: IndexMap<String, i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calculation_money_unit: Option<String>,
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
    pub id: String,
    #[serde(default, deserialize_with = "deserialize_nullable_finance_millis")]
    pub updated_at_epoch_millis: i64,
    #[serde(default, deserialize_with = "deserialize_nullable_finance_millis")]
    pub deleted_at_epoch_millis: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_income_kind")]
    pub kind: FinanceIncomeKind,
    #[serde(default)]
    pub amount: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount_minor: Option<i64>,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FinanceExpenseEntry {
    #[serde(default)]
    pub id: String,
    #[serde(default, deserialize_with = "deserialize_nullable_finance_millis")]
    pub updated_at_epoch_millis: i64,
    #[serde(default, deserialize_with = "deserialize_nullable_finance_millis")]
    pub deleted_at_epoch_millis: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_expense_bucket")]
    pub bucket: FinanceExpenseBucket,
    #[serde(default)]
    pub amount: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount_minor: Option<i64>,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FinanceNamedAmountEntry {
    #[serde(default)]
    pub id: String,
    #[serde(default, deserialize_with = "deserialize_nullable_finance_millis")]
    pub updated_at_epoch_millis: i64,
    #[serde(default, deserialize_with = "deserialize_nullable_finance_millis")]
    pub deleted_at_epoch_millis: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_named_amount_kind")]
    pub kind: FinanceNamedAmountKind,
    #[serde(default)]
    pub amount: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount_minor: Option<i64>,
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
    #[serde(default, deserialize_with = "deserialize_nullable_finance_millis")]
    pub confirmed_at_epoch_millis: i64,
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
    #[serde(default, deserialize_with = "deserialize_nullable_finance_millis")]
    pub confirmed_at_epoch_millis: i64,
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
    let profile = persisted_finance_profile(raw)?;
    serde_json::to_string(&profile.sanitized()).ok()
}

fn persisted_finance_profile(raw: &str) -> Option<FinanceProfile> {
    let value = serde_json::from_str::<serde_json::Value>(raw).ok()?;
    crate::finance_precision_guard::validate_profile(&value).ok()?;
    serde_json::from_value(value).ok()
}

pub fn decode_finance_backup_profile_json(raw: &str) -> Option<String> {
    let root = serde_json::from_str::<serde_json::Value>(raw).ok()?;
    let looks_like_backup_payload = root.as_object().is_some_and(|object| {
        object.contains_key("financeProfile")
            || object.contains_key("schemaVersion")
            || object.contains_key("appVersionName")
    });

    if looks_like_backup_payload {
        crate::finance_precision_guard::validate_profile(root.get("financeProfile")?).ok()?;
        let payload = serde_json::from_value::<FinanceBackupPayload>(root).ok()?;
        if payload.schema_version < 1 || payload.schema_version > FINANCE_BACKUP_SCHEMA_VERSION {
            return None;
        }
        serde_json::to_string(&payload.finance_profile.sanitized()).ok()
    } else {
        crate::finance_precision_guard::validate_profile(&root).ok()?;
        let profile = serde_json::from_str::<FinanceProfile>(raw).ok()?;
        serde_json::to_string(&profile.sanitized()).ok()
    }
}

pub fn encode_finance_backup_json(
    profile_json: &str,
    app_version_name: &str,
    exported_at_epoch_millis: i64,
) -> Option<String> {
    let profile = persisted_finance_profile(profile_json)?.sanitized();
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
    let mut profile = persisted_finance_profile(profile_json)?;
    let ledger_value = serde_json::from_str::<serde_json::Value>(ledger_json).ok()?;
    crate::finance_precision_guard::validate_profile(&serde_json::json!({
        "dailyLedgers": {day_key: ledger_value}
    }))
    .ok()?;
    let ledger = serde_json::from_str::<FinanceDayLedger>(ledger_json)
        .ok()?
        .sanitized();
    if ledger.has_persisted_entries() {
        profile.daily_ledgers.insert(day_key.to_string(), ledger);
    } else {
        profile.daily_ledgers.shift_remove(day_key);
    }
    serde_json::to_string(&profile.sanitized()).ok()
}

pub fn upsert_finance_month_snapshot_json(
    profile_json: &str,
    month_key: &str,
    snapshot_json: &str,
) -> Option<String> {
    if !is_valid_finance_month_key(month_key) {
        return None;
    }
    let mut profile = persisted_finance_profile(profile_json)?;
    let snapshot_value = serde_json::from_str::<serde_json::Value>(snapshot_json).ok()?;
    crate::finance_precision_guard::validate_profile(&serde_json::json!({
        "monthlySnapshots": {month_key: snapshot_value}
    }))
    .ok()?;
    let snapshot = serde_json::from_str::<FinanceMonthSnapshot>(snapshot_json)
        .ok()?
        .sanitized();
    if snapshot.has_persisted_entries() {
        profile
            .monthly_snapshots
            .insert(month_key.to_string(), snapshot);
    } else {
        profile.monthly_snapshots.shift_remove(month_key);
    }
    serde_json::to_string(&profile.sanitized()).ok()
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
    let templates = serde_json::from_str::<Vec<FinanceIncomeEntry>>(templates_json)
        .ok()?
        .into_iter()
        .filter(|entry| !entry.is_deleted())
        .map(|mut entry| {
            entry.id.clear();
            entry.updated_at_epoch_millis = 0;
            entry.deleted_at_epoch_millis = 0;
            entry
        })
        .collect();
    let appended = append_missing_templates(rows, templates, |entry| {
        (!entry.is_deleted())
            .then(|| template_key(income_kind_code(&entry.kind), &entry.name))
            .flatten()
    });
    serde_json::to_string(&appended).ok()
}

pub fn append_missing_expense_templates_json(
    rows_json: &str,
    templates_json: &str,
) -> Option<String> {
    let rows = serde_json::from_str::<Vec<FinanceExpenseEntry>>(rows_json).ok()?;
    let templates = serde_json::from_str::<Vec<FinanceExpenseEntry>>(templates_json)
        .ok()?
        .into_iter()
        .filter(|entry| !entry.is_deleted())
        .map(|mut entry| {
            entry.id.clear();
            entry.updated_at_epoch_millis = 0;
            entry.deleted_at_epoch_millis = 0;
            entry
        })
        .collect();
    let appended = append_missing_templates(rows, templates, |entry| {
        (!entry.is_deleted())
            .then(|| template_key(bucket_code(&entry.bucket), &entry.name))
            .flatten()
    });
    serde_json::to_string(&appended).ok()
}

pub fn append_missing_named_amount_templates_json(
    rows_json: &str,
    templates_json: &str,
) -> Option<String> {
    let rows = serde_json::from_str::<Vec<FinanceNamedAmountEntry>>(rows_json).ok()?;
    let templates = serde_json::from_str::<Vec<FinanceNamedAmountEntry>>(templates_json)
        .ok()?
        .into_iter()
        .filter(|entry| !entry.is_deleted())
        .map(|mut entry| {
            entry.id.clear();
            entry.updated_at_epoch_millis = 0;
            entry.deleted_at_epoch_millis = 0;
            entry
        })
        .collect();
    let appended = append_missing_templates(rows, templates, |entry| {
        (!entry.is_deleted())
            .then(|| template_key(named_amount_kind_code(&entry.kind), &entry.name))
            .flatten()
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
    pub cashflow_comparison_available: bool,
    pub net_worth_comparison_available: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct FinanceYearNetWorthValues {
    opening_net_worth: i64,
    closing_net_worth: i64,
    opening_month_code: i64,
    closing_month_code: i64,
    has_calendar_year_opening: bool,
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
pub struct FinanceHealthScoreValues {
    pub score: i32,
    pub risk_level_code: i32,
    pub cashflow_score: f32,
    pub defensive_score: f32,
    pub liability_score: f32,
    pub passive_score: f32,
    pub wage_score: f32,
    pub record_score: f32,
    pub discipline_score: f32,
    pub momentum_score: f32,
    pub warning_count: i32,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FinanceRiskState {
    #[default]
    Unknown,
    Stable,
    Watch,
    Tight,
    Critical,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FinanceRiskCockpitSnapshot {
    pub risk_state: FinanceRiskState,
    pub reason_code: String,
    pub policy_version: u32,
    pub data_confidence: i32,
    pub uses_detailed_cashflow: bool,
    pub forecast_available: bool,
    pub balance_sheet_available: bool,
    pub has_fresh_snapshot: bool,
    pub safe_to_spend: i64,
    pub protected_allocation: i64,
    pub pending_recurring_reserve: i64,
    pub projected_monthly_net: i64,
    pub projected_balance30_days: i64,
    pub projected_balance90_days: i64,
    pub stressed_balance30_days: i64,
    pub cash_reserve: i64,
    pub emergency_target: i64,
    pub emergency_gap: i64,
    pub emergency_months: Option<f32>,
    pub debt_balance: i64,
    pub monthly_debt_payment: i64,
    pub debt_free_months: Option<f32>,
    pub recurring_monthly_estimate: i64,
    pub anomaly_day_key: String,
    pub anomaly_amount: i64,
    pub anomaly_ratio: f32,
    pub budget_envelopes: Vec<FinanceBudgetEnvelope>,
    pub recurring_candidates: Vec<FinanceRecurringCandidate>,
    pub actions: Vec<FinanceRiskAction>,
}

// Android keeps these receipts in workspace-local storage. Fingerprints bind a
// review to the exact month and lane; this type is intentionally absent from
// FinanceProfile so that it cannot enter the existing sync protocol.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FinanceReviewReceipt {
    pub month_key: String,
    #[serde(default)]
    pub income_expense_fingerprint: String,
    #[serde(default)]
    pub cash_fingerprint: String,
    #[serde(default)]
    pub assets_fingerprint: String,
    #[serde(default)]
    pub liabilities_fingerprint: String,
    #[serde(default)]
    pub income_expense_verified: bool,
    #[serde(default)]
    pub cash_verified: bool,
    #[serde(default)]
    pub assets_verified: bool,
    #[serde(default)]
    pub liabilities_verified: bool,
    #[serde(default)]
    pub zero_cash_confirmed: bool,
    #[serde(default)]
    pub zero_assets_confirmed: bool,
    #[serde(default)]
    pub zero_liabilities_confirmed: bool,
    #[serde(default)]
    pub confirmed_recurring_keys: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FinanceReviewFingerprints {
    pub month_key: String,
    pub income_expense_fingerprint: String,
    pub cash_fingerprint: String,
    pub assets_fingerprint: String,
    pub liabilities_fingerprint: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AndroidFinanceRecurringExpense {
    pub key: String,
    pub name: String,
    pub bucket_code: i32,
    pub monthly_amount: i64,
    pub confirmed: bool,
    pub paid_this_month: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AndroidFinanceDuplicatePayment {
    pub day_key: String,
    pub name: String,
    pub amount: i64,
    pub entry_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AndroidFinanceRiskV2 {
    pub policy_version: u32,
    pub month_key: String,
    pub recorded_income: i64,
    pub recorded_outflow: i64,
    pub recorded_net_cashflow: i64,
    pub recorded_days: i64,
    pub recorded_cash: Option<i64>,
    pub recorded_assets: Option<i64>,
    pub recorded_liabilities: Option<i64>,
    pub income_expense_verified: bool,
    pub cash_verified: bool,
    pub assets_verified: bool,
    pub liabilities_verified: bool,
    pub verified_cash: Option<i64>,
    pub verified_assets: Option<i64>,
    pub verified_liabilities: Option<i64>,
    pub baseline_months: Vec<String>,
    pub forecast_available: bool,
    pub monthly_income_baseline: Option<i64>,
    pub monthly_outflow_baseline: Option<i64>,
    pub monthly_net_baseline: Option<i64>,
    pub pending_essential_reserve: Option<i64>,
    pub protected_allocation: Option<i64>,
    pub safe_to_spend: Option<i64>,
    pub projected_balance30_days: Option<i64>,
    pub projected_balance90_days: Option<i64>,
    pub risk_state: FinanceRiskState,
    pub reason_code: String,
    pub recurring_expenses: Vec<AndroidFinanceRecurringExpense>,
    pub duplicate_payments: Vec<AndroidFinanceDuplicatePayment>,
    pub anomaly_day_key: Option<String>,
    pub anomaly_amount: Option<i64>,
    pub anomaly_ratio: Option<f32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FinanceBudgetEnvelope {
    pub bucket_code: i32,
    pub label: String,
    pub target_amount: i64,
    pub actual_amount: i64,
    pub remaining_amount: i64,
    pub status_code: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FinanceRecurringCandidate {
    pub name: String,
    pub occurrences: i32,
    pub average_amount: i64,
    pub monthly_estimate: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FinanceRiskAction {
    pub severity_code: i32,
    pub kind_code: i32,
    pub bucket_code: i32,
    pub amount: i64,
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
const ALERT_KIND_BALANCE_SHEET_RISK: i32 = 7;
const ALERT_KIND_STALE_MONTH_SNAPSHOT: i32 = 8;
const ALERT_KIND_TARGET_OVERCOMMITTED: i32 = 9;
const ALERT_KIND_CASHFLOW_MISSING: i32 = 10;
const ALERT_KIND_LEGACY_CASHFLOW_ESTIMATE: i32 = 11;

const RISK_LEVEL_STEADY: i32 = 0;
const RISK_LEVEL_WATCH: i32 = 1;
const RISK_LEVEL_TIGHT: i32 = 2;
const RISK_LEVEL_CRITICAL: i32 = 3;

const FINANCE_RISK_POLICY_VERSION: u32 = 1;
const REASON_CASHFLOW_BASELINE_MISSING: &str = "CASHFLOW_BASELINE_MISSING";
const REASON_CASHFLOW_COVERAGE_INSUFFICIENT: &str = "CASHFLOW_COVERAGE_INSUFFICIENT";
const REASON_CURRENT_BALANCE_SHEET_MISSING: &str = "CURRENT_BALANCE_SHEET_MISSING";
const REASON_OBSERVED_NEGATIVE_CASHFLOW: &str = "OBSERVED_NEGATIVE_CASHFLOW";
const REASON_OBSERVED_NEGATIVE_NET_WORTH: &str = "OBSERVED_NEGATIVE_NET_WORTH";
const REASON_POLICY_STABLE: &str = "POLICY_STABLE";
const REASON_POLICY_WATCH: &str = "POLICY_WATCH";
const REASON_POLICY_TIGHT: &str = "POLICY_TIGHT";
const REASON_POLICY_CRITICAL: &str = "POLICY_CRITICAL";

// A monthly balance-sheet snapshot older than this no longer represents the
// user's current defensive buffer or leverage reliably enough for risk scoring.
const MAX_HEALTH_SNAPSHOT_AGE_MONTHS: i32 = 3;
const TARGET_TOTAL_EPSILON: f32 = 0.0001;
const TARGET_DISCIPLINE_PENALTY_SPAN: f32 = 0.20;
const TARGET_OVERCOMMIT_SCORE_CAP: f32 = 0.80;

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

const BUDGET_STATUS_NORMAL: i32 = 0;
const BUDGET_STATUS_NEAR_CAP: i32 = 1;
const BUDGET_STATUS_OVER_CAP: i32 = 2;
const BUDGET_STATUS_BELOW_FLOOR: i32 = 3;
const BUDGET_STATUS_NO_TARGET: i32 = 4;

const ACTION_SEVERITY_INFO: i32 = 0;
const ACTION_SEVERITY_WATCH: i32 = 1;
const ACTION_SEVERITY_URGENT: i32 = 2;

const ACTION_KIND_COMPLETE_DATA: i32 = 0;
const ACTION_KIND_STOP_NEGATIVE_CASHFLOW: i32 = 1;
const ACTION_KIND_BUILD_EMERGENCY_FUND: i32 = 2;
const ACTION_KIND_REFRESH_MONTH_SNAPSHOT: i32 = 3;
const ACTION_KIND_REDUCE_OVER_BUDGET: i32 = 4;
const ACTION_KIND_FILL_LONG_TERM_ALLOCATION: i32 = 5;
const ACTION_KIND_ACCELERATE_DEBT: i32 = 6;
const ACTION_KIND_REVIEW_ANOMALY_DAY: i32 = 7;
const ACTION_KIND_REVIEW_RECURRING_EXPENSE: i32 = 8;
const ACTION_KIND_STRESS_TEST_PASSED: i32 = 9;
const ACTION_KIND_CURRENTLY_STABLE: i32 = 10;

const FORECAST_MIN_COVERAGE: f32 = 0.60;
const FORECAST_MIN_RECORDED_DAYS: i64 = 3;
const RECENT_RISK_WINDOW_DAYS: i64 = 90;

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

    fn essential_outflow_total(self) -> i64 {
        self.debt_total
            .saturating_add(self.food_total)
            .saturating_add(self.living_total)
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

/// Read-only desktop aggregate; Android continues to use its existing facades.
#[cfg(not(target_os = "android"))]
#[derive(Debug, PartialEq)]
pub struct DesktopFinanceOverviewValues {
    pub report: FinanceSnapshotValues,
    pub trend: FinanceTrendValues,
    pub health: Option<FinanceHealthScoreValues>,
    pub risk_json: String,
    pub latest_month: Option<String>,
}

#[cfg(not(target_os = "android"))]
pub fn build_desktop_finance_overview_values(
    raw: &str,
    period_code: i32,
    reference_year: i32,
    reference_month: i32,
    reference_day: i32,
) -> Option<DesktopFinanceOverviewValues> {
    if !matches!(
        period_code,
        PERIOD_DAY | PERIOD_MONTH | PERIOD_QUARTER | PERIOD_YEAR
    ) || !is_valid_ymd(
        reference_year.max(0) as u32,
        reference_month.max(0) as u32,
        reference_day.max(0) as u32,
    ) {
        return None;
    }
    let profile = serde_json::from_str::<FinanceProfile>(raw).ok()?;
    // These three legacy facades read the original profile. Calculate them
    // before consuming it for the sanitized health and risk computations.
    let report = profile.detailed_finance_report_values(
        period_code,
        reference_year,
        reference_month,
        reference_day,
    );
    let trend =
        profile.finance_trend_values(period_code, reference_year, reference_month, reference_day);
    let month_key = format!("{reference_year:04}-{reference_month:02}");
    let latest_month = Some(if is_valid_finance_month_key(&month_key) {
        profile
            .latest_snapshot_month_key_up_to(&month_key)
            .unwrap_or_default()
    } else {
        String::new()
    });
    let profile = profile.sanitized();
    let health = (profile.uses_detailed_cashflow_for_period(
        period_code,
        reference_year,
        reference_month,
        reference_day,
    ) || profile.has_legacy_cashflow_estimates())
    .then(|| {
        profile.finance_health_score_values(
            period_code,
            reference_year,
            reference_month,
            reference_day,
        )
    });
    let risk_json = serde_json::to_string(&profile.finance_risk_cockpit_snapshot(
        period_code,
        reference_year,
        reference_month,
        reference_day,
    ))
    .ok()?;
    Some(DesktopFinanceOverviewValues {
        report,
        trend,
        health,
        risk_json,
        latest_month,
    })
}

#[cfg(all(test, not(target_os = "android")))]
#[path = "finance_overview_performance_tests.rs"]
mod desktop_overview_performance_tests;

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
    let aggregate = profile.report_aggregate_for_period(
        period_code,
        reference_year,
        reference_month,
        reference_day,
    );
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

pub fn build_finance_health_score_values(
    raw: &str,
    period_code: i32,
    reference_year: i32,
    reference_month: i32,
    reference_day: i32,
) -> Option<FinanceHealthScoreValues> {
    let profile = serde_json::from_str::<FinanceProfile>(raw)
        .ok()?
        .sanitized();
    if !profile.uses_detailed_cashflow_for_period(
        period_code,
        reference_year,
        reference_month,
        reference_day,
    ) && !profile.has_legacy_cashflow_estimates()
    {
        return None;
    }
    Some(profile.finance_health_score_values(
        period_code,
        reference_year,
        reference_month,
        reference_day,
    ))
}

pub fn build_finance_risk_cockpit_json(
    raw: &str,
    period_code: i32,
    reference_year: i32,
    reference_month: i32,
    reference_day: i32,
) -> Option<String> {
    if !matches!(
        period_code,
        PERIOD_DAY | PERIOD_MONTH | PERIOD_QUARTER | PERIOD_YEAR
    ) || !is_valid_ymd(
        reference_year.max(0) as u32,
        reference_month.max(0) as u32,
        reference_day.max(0) as u32,
    ) {
        return None;
    }
    let profile = serde_json::from_str::<FinanceProfile>(raw)
        .ok()?
        .sanitized();
    let snapshot = profile.finance_risk_cockpit_snapshot(
        period_code,
        reference_year,
        reference_month,
        reference_day,
    );
    serde_json::to_string(&snapshot).ok()
}

pub fn build_android_finance_review_fingerprints_json(
    profile_json: &str,
    month_key: &str,
) -> Option<String> {
    if !is_valid_finance_month_key(month_key) {
        return None;
    }
    let profile = serde_json::from_str::<FinanceProfile>(profile_json)
        .ok()?
        .sanitized();
    serde_json::to_string(&android_finance_review_fingerprints(&profile, month_key)?).ok()
}

pub fn build_android_finance_risk_v2_json(
    profile_json: &str,
    review_receipts_json: &str,
    reference_year: i32,
    reference_month: i32,
    reference_day: i32,
) -> Option<String> {
    if !is_valid_ymd(
        reference_year.max(0) as u32,
        reference_month.max(0) as u32,
        reference_day.max(0) as u32,
    ) {
        return None;
    }
    let profile = serde_json::from_str::<FinanceProfile>(profile_json)
        .ok()?
        .sanitized();
    let receipts = serde_json::from_str::<Vec<FinanceReviewReceipt>>(review_receipts_json).ok()?;
    let result = android_finance_risk_v2(
        &profile,
        &receipts,
        reference_year,
        reference_month,
        reference_day,
    )?;
    serde_json::to_string(&result).ok()
}

fn android_finance_review_fingerprints(
    profile: &FinanceProfile,
    month_key: &str,
) -> Option<FinanceReviewFingerprints> {
    let day_prefix = format!("{month_key}-");
    let ledgers = profile
        .daily_ledgers
        .iter()
        .filter(|(day_key, _)| day_key.starts_with(&day_prefix))
        .collect::<Vec<_>>();
    let snapshot = profile.monthly_snapshots.get(month_key);
    let cash_rows = snapshot
        .map(|value| {
            value
                .assets
                .iter()
                .filter(|row| row.kind == FinanceNamedAmountKind::CashReserve)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let note = snapshot.map(|value| value.note.as_str()).unwrap_or("");
    let confirmed_at = snapshot
        .map(|value| value.confirmed_at_epoch_millis)
        .unwrap_or(0);
    Some(FinanceReviewFingerprints {
        month_key: month_key.to_string(),
        income_expense_fingerprint: finance_review_fingerprint(month_key, "cashflow", &ledgers)?,
        cash_fingerprint: finance_review_fingerprint(
            month_key,
            "cash",
            &(snapshot.is_some(), note, confirmed_at, cash_rows),
        )?,
        assets_fingerprint: finance_review_fingerprint(
            month_key,
            "assets",
            &(
                snapshot.is_some(),
                note,
                confirmed_at,
                snapshot.map(|value| &value.assets),
            ),
        )?,
        liabilities_fingerprint: finance_review_fingerprint(
            month_key,
            "liabilities",
            &(
                snapshot.is_some(),
                note,
                confirmed_at,
                snapshot.map(|value| &value.liabilities),
            ),
        )?,
    })
}

fn finance_review_fingerprint<T: Serialize>(
    month_key: &str,
    lane: &str,
    data: &T,
) -> Option<String> {
    let bytes = serde_json::to_vec(&(month_key, lane, data)).ok()?;
    Some(format!("{:x}", Sha256::digest(&bytes)))
}

fn matching_review_receipt<'a>(
    receipts: &'a [FinanceReviewReceipt],
    month_key: &str,
) -> Option<&'a FinanceReviewReceipt> {
    receipts
        .iter()
        .rev()
        .find(|receipt| receipt.month_key == month_key)
}

fn income_expense_review_matches(
    receipt: &FinanceReviewReceipt,
    fingerprint: &str,
    recorded_days: i64,
    reference_day: i32,
    full_month_required: bool,
) -> bool {
    if !receipt.income_expense_verified {
        return false;
    }
    let full_month_fingerprint = if recorded_days == 0 {
        format!("zero-full:{fingerprint}")
    } else {
        format!("full:{fingerprint}")
    };
    receipt.income_expense_fingerprint == full_month_fingerprint
        || (!full_month_required
            && receipt.income_expense_fingerprint
                == format!("through:{reference_day}:{fingerprint}"))
}

fn android_finance_risk_v2(
    profile: &FinanceProfile,
    receipts: &[FinanceReviewReceipt],
    reference_year: i32,
    reference_month: i32,
    reference_day: i32,
) -> Option<AndroidFinanceRiskV2> {
    let month_key = format!("{reference_year:04}-{reference_month:02}");
    let current_fp = android_finance_review_fingerprints(profile, &month_key)?;
    let current_receipt = matching_review_receipt(receipts, &month_key);
    let cash_verified = current_receipt.is_some_and(|receipt| {
        receipt.cash_verified && receipt.cash_fingerprint == current_fp.cash_fingerprint
    });
    let assets_verified = current_receipt.is_some_and(|receipt| {
        receipt.assets_verified && receipt.assets_fingerprint == current_fp.assets_fingerprint
    });
    let liabilities_verified = current_receipt.is_some_and(|receipt| {
        receipt.liabilities_verified
            && receipt.liabilities_fingerprint == current_fp.liabilities_fingerprint
    });
    let current =
        profile.aggregate_for_period(PERIOD_MONTH, reference_year, reference_month, reference_day);
    let income_expense_verified = current_receipt.is_some_and(|receipt| {
        income_expense_review_matches(
            receipt,
            &current_fp.income_expense_fingerprint,
            current.days_with_entries,
            reference_day,
            false,
        )
    });
    let snapshot = profile.monthly_snapshots.get(&month_key);
    let summary = snapshot.map(FinanceMonthSnapshot::to_summary_totals);
    let has_cash_row = snapshot.is_some_and(|value| {
        value
            .assets
            .iter()
            .any(|row| !row.is_deleted() && row.kind == FinanceNamedAmountKind::CashReserve)
    });
    let has_asset_row =
        snapshot.is_some_and(|value| value.assets.iter().any(|row| !row.is_deleted()));
    let has_liability_row =
        snapshot.is_some_and(|value| value.liabilities.iter().any(|row| !row.is_deleted()));
    let recorded_cash = has_cash_row.then(|| summary.unwrap().cash_reserve_total);
    let recorded_assets = has_asset_row.then(|| summary.unwrap().asset_total);
    let recorded_liabilities = has_liability_row.then(|| summary.unwrap().liability_total);
    let verified_cash = cash_verified
        .then(|| {
            recorded_cash.or_else(|| {
                current_receipt
                    .filter(|receipt| receipt.zero_cash_confirmed)
                    .map(|_| 0)
            })
        })
        .flatten();
    let verified_assets = assets_verified
        .then(|| {
            recorded_assets.or_else(|| {
                current_receipt
                    .filter(|receipt| receipt.zero_assets_confirmed)
                    .map(|_| 0)
            })
        })
        .flatten();
    let verified_liabilities = liabilities_verified
        .then(|| {
            recorded_liabilities.or_else(|| {
                current_receipt
                    .filter(|receipt| receipt.zero_liabilities_confirmed)
                    .map(|_| 0)
            })
        })
        .flatten();

    // A complete month is established by the user's local receipt, never by
    // the mere presence of one wage payment or a few days with entries.
    let mut baseline = Vec::<(String, FinanceLedgerTotals)>::new();
    for offset in 1..=6 {
        let (year, month, _) = add_months(reference_year, reference_month, 1, -offset);
        let key = format!("{year:04}-{month:02}");
        let Some(receipt) = matching_review_receipt(receipts, &key) else {
            continue;
        };
        let fingerprints = android_finance_review_fingerprints(profile, &key)?;
        let aggregate =
            profile.aggregate_for_period(PERIOD_MONTH, year, month, days_in_month(year, month));
        if !income_expense_review_matches(
            receipt,
            &fingerprints.income_expense_fingerprint,
            aggregate.days_with_entries,
            days_in_month(year, month),
            true,
        ) {
            continue;
        }
        baseline.push((key, aggregate));
        if baseline.len() == 3 {
            break;
        }
    }
    let forecast_available = baseline.len() == 3;
    let monthly_income_baseline = forecast_available
        .then(|| median_finance_amount(baseline.iter().map(|(_, value)| value.income_total())));
    let monthly_outflow_baseline = forecast_available
        .then(|| median_finance_amount(baseline.iter().map(|(_, value)| value.expense_total())));
    let monthly_net_baseline = forecast_available
        .then(|| median_finance_amount(baseline.iter().map(|(_, value)| value.net_cashflow())));
    let recurring_expenses = if forecast_available {
        android_stable_recurring_expenses(
            profile,
            &baseline,
            current_receipt.filter(|_| income_expense_verified),
            reference_year,
            reference_month,
            reference_day,
        )
    } else {
        Vec::new()
    };
    let duplicate_payments = android_duplicate_payments(profile, &month_key, reference_day);
    let pending_essential_reserve = (forecast_available && income_expense_verified).then(|| {
        let baseline_pending = median_finance_amount(
            baseline
                .iter()
                .map(|(_, value)| value.essential_outflow_total()),
        )
        .saturating_sub(current.essential_outflow_total())
        .max(0);
        // An unrelated food or living payment never settles a separately
        // confirmed unpaid rent, debt instalment, or other essential bill.
        let unpaid_confirmed_essential = recurring_expenses
            .iter()
            .filter(|item| {
                item.confirmed
                    && !item.paid_this_month
                    && matches!(
                        item.bucket_code,
                        BUCKET_CODE_DEBT | BUCKET_CODE_FOOD | BUCKET_CODE_LIVING
                    )
            })
            .map(|item| item.monthly_amount)
            .fold(0_i64, i64::saturating_add);
        baseline_pending.max(unpaid_confirmed_essential)
    });
    let protected_allocation = (forecast_available && income_expense_verified).then(|| {
        let baseline_income = monthly_income_baseline.unwrap_or(0);
        profile
            .settings
            .expense_categories
            .iter()
            .filter(|config| is_minimum_allocation_bucket(&config.bucket))
            .map(|config| {
                let share = config.target_share_of_income.unwrap_or(0.0);
                let target = if share.is_finite() && (0.0..=1.0).contains(&share) {
                    scale_finance_amount(baseline_income, share as f64)
                } else {
                    0
                };
                target
                    .saturating_sub(current.bucket_total(&config.bucket))
                    .max(0)
            })
            .fold(0_i64, i64::saturating_add)
    });
    let complete_current_review = income_expense_verified
        && verified_cash.is_some()
        && verified_assets.is_some()
        && verified_liabilities.is_some();
    let safe_to_spend = if complete_current_review {
        verified_cash
            .zip(pending_essential_reserve)
            .zip(protected_allocation)
            .map(|((cash, essential), protected)| {
                cash.saturating_sub(essential).saturating_sub(protected)
            })
    } else {
        None
    };
    let projected_balance30_days = if complete_current_review {
        verified_cash
            .zip(monthly_net_baseline)
            .map(|(cash, net)| cash.saturating_add(net))
    } else {
        None
    };
    let projected_balance90_days = if complete_current_review {
        verified_cash
            .zip(monthly_net_baseline)
            .map(|(cash, net)| cash.saturating_add(net.saturating_mul(3)))
    } else {
        None
    };
    let (risk_state, reason_code) = match (
        forecast_available,
        income_expense_verified,
        verified_cash,
        verified_assets,
        verified_liabilities,
        safe_to_spend,
        projected_balance30_days,
        projected_balance90_days,
    ) {
        (
            true,
            true,
            Some(_),
            Some(assets),
            Some(liabilities),
            Some(safe),
            Some(day30),
            Some(day90),
        ) => {
            if assets < liabilities {
                (FinanceRiskState::Critical, "VERIFIED_NEGATIVE_NET_WORTH")
            } else if safe < 0 || day30 < 0 {
                (FinanceRiskState::Tight, "VERIFIED_CASH_GAP")
            } else if day90 < 0 {
                (FinanceRiskState::Watch, "VERIFIED_90_DAY_GAP")
            } else {
                (FinanceRiskState::Stable, "VERIFIED_BASELINE_STABLE")
            }
        }
        _ if !forecast_available => (FinanceRiskState::Unknown, "THREE_REVIEWED_MONTHS_REQUIRED"),
        _ => (FinanceRiskState::Unknown, "CURRENT_REVIEW_INCOMPLETE"),
    };
    let anomaly = if forecast_available && income_expense_verified {
        android_expense_anomaly(
            profile,
            &baseline,
            &month_key,
            reference_day,
            &recurring_expenses,
        )
    } else {
        None
    };
    Some(AndroidFinanceRiskV2 {
        policy_version: 2,
        month_key,
        recorded_income: current.income_total(),
        recorded_outflow: current.expense_total(),
        recorded_net_cashflow: current.net_cashflow(),
        recorded_days: current.days_with_entries,
        recorded_cash,
        recorded_assets,
        recorded_liabilities,
        income_expense_verified,
        cash_verified,
        assets_verified,
        liabilities_verified,
        verified_cash,
        verified_assets,
        verified_liabilities,
        baseline_months: baseline.into_iter().map(|(key, _)| key).collect(),
        forecast_available,
        monthly_income_baseline,
        monthly_outflow_baseline,
        monthly_net_baseline,
        pending_essential_reserve,
        protected_allocation,
        safe_to_spend,
        projected_balance30_days,
        projected_balance90_days,
        risk_state,
        reason_code: reason_code.to_string(),
        recurring_expenses,
        duplicate_payments,
        anomaly_day_key: anomaly.as_ref().map(|value| value.0.clone()),
        anomaly_amount: anomaly.as_ref().map(|value| value.1),
        anomaly_ratio: anomaly.map(|value| value.2),
    })
}

fn median_finance_amount(values: impl Iterator<Item = i64>) -> i64 {
    let mut values = values.collect::<Vec<_>>();
    values.sort_unstable();
    values[values.len() / 2]
}

fn android_expense_key(name: &str, bucket: &FinanceExpenseBucket) -> Option<String> {
    let normalized = compact_whitespace(name).to_lowercase();
    (!normalized.is_empty()).then(|| format!("{}:{normalized}", bucket_code(bucket)))
}

fn android_stable_recurring_expenses(
    profile: &FinanceProfile,
    baseline: &[(String, FinanceLedgerTotals)],
    current_receipt: Option<&FinanceReviewReceipt>,
    reference_year: i32,
    reference_month: i32,
    reference_day: i32,
) -> Vec<AndroidFinanceRecurringExpense> {
    if baseline.len() != 3 {
        return Vec::new();
    }
    let mut month_indices = baseline
        .iter()
        .filter_map(|(key, _)| {
            parse_finance_month_key(key).map(|(year, month)| month_index(year, month))
        })
        .collect::<Vec<_>>();
    month_indices.sort_unstable();
    if month_indices.len() != 3 || month_indices[2] - month_indices[0] != 2 {
        return Vec::new();
    }
    let baseline_keys = baseline
        .iter()
        .map(|(key, _)| key.as_str())
        .collect::<HashSet<_>>();
    let mut grouped = HashMap::<String, Vec<(i32, i32, i64, String, i32)>>::new();
    for (day_key, ledger) in &profile.daily_ledgers {
        let Some((year, month, day)) = parse_finance_day_key(day_key) else {
            continue;
        };
        if !baseline_keys.contains(format!("{year:04}-{month:02}").as_str()) {
            continue;
        }
        for expense in &ledger.expenses {
            if expense.is_deleted() || expense.amount <= 0 {
                continue;
            }
            let Some(key) = android_expense_key(&expense.name, &expense.bucket) else {
                continue;
            };
            grouped.entry(key).or_default().push((
                month_index(year, month),
                day,
                expense.amount,
                compact_whitespace(&expense.name),
                bucket_code(&expense.bucket),
            ));
        }
    }
    let current_key = format!("{reference_year:04}-{reference_month:02}");
    let confirmed_keys = current_receipt
        .map(|receipt| {
            receipt
                .confirmed_recurring_keys
                .iter()
                .map(String::as_str)
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    let mut result = Vec::new();
    for (key, entries) in grouped {
        if entries.len() != 3
            || entries
                .iter()
                .map(|entry| entry.0)
                .collect::<HashSet<_>>()
                .len()
                != 3
        {
            continue;
        }
        let min_day = entries.iter().map(|entry| entry.1).min().unwrap_or(0);
        let max_day = entries.iter().map(|entry| entry.1).max().unwrap_or(0);
        let min_amount = entries.iter().map(|entry| entry.2).min().unwrap_or(0);
        let max_amount = entries.iter().map(|entry| entry.2).max().unwrap_or(0);
        if max_day - min_day > 3
            || max_amount - min_amount
                > (min_amount / 10).max(
                    if profile.calculation_money_unit.as_deref() == Some("rmb-cent-v1") {
                        100
                    } else {
                        1
                    },
                )
        {
            continue;
        }
        let paid_this_month = profile.daily_ledgers.iter().any(|(day_key, ledger)| {
            day_key.starts_with(&format!("{current_key}-"))
                && parse_finance_day_key(day_key).is_some_and(|(_, _, day)| day <= reference_day)
                && ledger.expenses.iter().any(|expense| {
                    !expense.is_deleted()
                        && expense.amount > 0
                        && android_expense_key(&expense.name, &expense.bucket).as_deref()
                            == Some(key.as_str())
                })
        });
        result.push(AndroidFinanceRecurringExpense {
            key: key.clone(),
            name: entries[0].3.clone(),
            bucket_code: entries[0].4,
            monthly_amount: median_finance_amount(entries.iter().map(|entry| entry.2)),
            confirmed: confirmed_keys.contains(key.as_str()),
            paid_this_month,
        });
    }
    result.sort_by(|left, right| left.key.cmp(&right.key));
    result
}

fn android_duplicate_payments(
    profile: &FinanceProfile,
    month_key: &str,
    reference_day: i32,
) -> Vec<AndroidFinanceDuplicatePayment> {
    let mut result = Vec::new();
    for (day_key, ledger) in &profile.daily_ledgers {
        if !day_key.starts_with(&format!("{month_key}-"))
            || !parse_finance_day_key(day_key).is_some_and(|(_, _, day)| day <= reference_day)
        {
            continue;
        }
        let mut grouped = HashMap::<(String, i64), Vec<&FinanceExpenseEntry>>::new();
        for expense in &ledger.expenses {
            if expense.is_deleted() || expense.amount <= 0 {
                continue;
            }
            let name = compact_whitespace(&expense.name).to_lowercase();
            if !name.is_empty() {
                grouped
                    .entry((name, expense.amount))
                    .or_default()
                    .push(expense);
            }
        }
        for ((_, amount), rows) in grouped {
            if rows.len() < 2 {
                continue;
            }
            result.push(AndroidFinanceDuplicatePayment {
                day_key: day_key.clone(),
                name: compact_whitespace(&rows[0].name),
                amount,
                entry_ids: rows.iter().map(|row| row.id.clone()).collect(),
            });
        }
    }
    result.sort_by(|left, right| {
        left.day_key
            .cmp(&right.day_key)
            .then_with(|| left.name.cmp(&right.name))
    });
    result
}

fn android_expense_anomaly(
    profile: &FinanceProfile,
    baseline: &[(String, FinanceLedgerTotals)],
    current_month_key: &str,
    reference_day: i32,
    recurring: &[AndroidFinanceRecurringExpense],
) -> Option<(String, i64, f32)> {
    let excluded = recurring
        .iter()
        .filter(|item| item.confirmed)
        .map(|item| item.key.as_str())
        .collect::<HashSet<_>>();
    let baseline_months = baseline
        .iter()
        .map(|(key, _)| key.as_str())
        .collect::<HashSet<_>>();
    let mut historical = Vec::<i64>::new();
    let mut current = Vec::<(String, i64)>::new();
    for (day_key, ledger) in &profile.daily_ledgers {
        let Some((year, month, day)) = parse_finance_day_key(day_key) else {
            continue;
        };
        let month_key = format!("{year:04}-{month:02}");
        let is_baseline = baseline_months.contains(month_key.as_str());
        let is_current = month_key == current_month_key && day <= reference_day;
        if !is_baseline && !is_current {
            continue;
        }
        let total = ledger
            .expenses
            .iter()
            .filter(|expense| {
                !expense.is_deleted()
                    && expense.amount > 0
                    && !android_expense_key(&expense.name, &expense.bucket)
                        .is_some_and(|key| excluded.contains(key.as_str()))
            })
            .map(|expense| expense.amount)
            .fold(0_i64, i64::saturating_add);
        if total <= 0 {
            continue;
        }
        if is_baseline {
            historical.push(total);
        } else {
            current.push((day_key.clone(), total));
        }
    }
    if historical.len() < 6 {
        return None;
    }
    let median = median_finance_amount(historical.into_iter());
    if median <= 0 {
        return None;
    }
    current
        .into_iter()
        .filter_map(|(day_key, amount)| {
            let ratio = amount as f32 / median as f32;
            (ratio.is_finite() && ratio >= 2.0).then_some((day_key, amount, ratio))
        })
        .max_by(|left, right| {
            left.2
                .total_cmp(&right.2)
                .then_with(|| left.0.cmp(&right.0))
        })
}

pub fn year_net_worth_summary_values(raw: &str, year: i32, through_month: i32) -> Option<[i64; 5]> {
    let profile = serde_json::from_str::<FinanceProfile>(raw)
        .ok()?
        .sanitized();
    let summary = profile.year_net_worth_summary(year, through_month);
    Some([
        summary.opening_net_worth,
        summary.closing_net_worth,
        summary.opening_month_code,
        summary.closing_month_code,
        i64::from(summary.has_calendar_year_opening),
    ])
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
        let mut daily_ledgers = sanitize_finance_day_ledgers(self.daily_ledgers);
        let mut monthly_snapshots = sanitize_finance_month_snapshots(self.monthly_snapshots);
        assign_finance_profile_entry_ids(&mut daily_ledgers, &mut monthly_snapshots);
        Self {
            active_income_monthly: self.active_income_monthly,
            asset_income_monthly: self.asset_income_monthly,
            living_expense_monthly: self.living_expense_monthly,
            liability_payment_monthly: self.liability_payment_monthly,
            cash_reserve: self.cash_reserve,
            productive_asset_value: self.productive_asset_value,
            liability_balance: self.liability_balance,
            legacy_amount_minor: self.legacy_amount_minor,
            calculation_money_unit: self.calculation_money_unit,
            acquisition_focus: self.acquisition_focus,
            liability_focus: self.liability_focus,
            settings: self.settings.sanitized(),
            daily_ledgers,
            monthly_snapshots,
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
            || self.legacy_amount_minor.values().any(|value| *value > 0)
            || !self.acquisition_focus.trim().is_empty()
            || !self.liability_focus.trim().is_empty()
            || self
                .daily_ledgers
                .iter()
                .any(|(key, ledger)| is_valid_finance_day_key(key) && ledger.has_entries())
            || self
                .monthly_snapshots
                .iter()
                .any(|(key, snapshot)| is_valid_finance_month_key(key) && snapshot.has_entries())
    }

    fn has_detailed_finance_data(&self) -> bool {
        self.has_detailed_cashflow_data() || self.has_detailed_balance_sheet_data()
    }

    fn has_detailed_cashflow_data(&self) -> bool {
        self.daily_ledgers
            .iter()
            .any(|(key, ledger)| is_valid_finance_day_key(key) && ledger.has_recorded_finance_day())
    }

    fn has_legacy_cashflow_estimates(&self) -> bool {
        self.active_income_monthly > 0
            || self.asset_income_monthly > 0
            || self.living_expense_monthly > 0
            || self.liability_payment_monthly > 0
            || [
                "activeIncomeMonthly",
                "assetIncomeMonthly",
                "livingExpenseMonthly",
                "liabilityPaymentMonthly",
            ]
            .iter()
            .any(|field| {
                self.legacy_amount_minor
                    .get(*field)
                    .is_some_and(|value| *value > 0)
            })
    }

    fn has_detailed_balance_sheet_data(&self) -> bool {
        self.monthly_snapshots.iter().any(|(key, snapshot)| {
            is_valid_finance_month_key(key) && snapshot.has_financial_entries()
        })
    }

    fn report_aggregate_for_period(
        &self,
        period_code: i32,
        reference_year: i32,
        reference_month: i32,
        reference_day: i32,
    ) -> FinanceLedgerTotals {
        let detailed =
            self.aggregate_for_period(period_code, reference_year, reference_month, reference_day);
        // Source selection is period-local. A future or unrelated detailed row
        // must not silently zero an older report that still relies on the legacy
        // monthly estimate. A positive row switches this period immediately;
        // zero-only confirmations switch it only after meaningful coverage.
        if self.detailed_cashflow_is_authoritative(
            detailed,
            period_code,
            reference_year,
            reference_month,
            reference_day,
        ) {
            detailed
        } else {
            self.legacy_cashflow_aggregate(period_code)
        }
    }

    fn uses_detailed_cashflow_for_period(
        &self,
        period_code: i32,
        reference_year: i32,
        reference_month: i32,
        reference_day: i32,
    ) -> bool {
        let detailed =
            self.aggregate_for_period(period_code, reference_year, reference_month, reference_day);
        self.detailed_cashflow_is_authoritative(
            detailed,
            period_code,
            reference_year,
            reference_month,
            reference_day,
        )
    }

    fn detailed_cashflow_is_authoritative(
        &self,
        detailed: FinanceLedgerTotals,
        period_code: i32,
        reference_year: i32,
        reference_month: i32,
        reference_day: i32,
    ) -> bool {
        let has_financial_value = detailed.income_total() > 0 || detailed.expense_total() > 0;
        let expected_days =
            expected_recorded_days(period_code, reference_year, reference_month, reference_day)
                .max(1) as i64;
        has_financial_value || detailed.days_with_entries.saturating_mul(5) >= expected_days * 3
    }

    fn legacy_cashflow_aggregate(&self, period_code: i32) -> FinanceLedgerTotals {
        FinanceLedgerTotals {
            active_income_total: scale_monthly_finance_amount(
                self.active_income_monthly.max(0),
                period_code,
            ),
            asset_income_total: scale_monthly_finance_amount(
                self.asset_income_monthly.max(0),
                period_code,
            ),
            other_income_total: 0,
            debt_total: scale_monthly_finance_amount(
                self.liability_payment_monthly.max(0),
                period_code,
            ),
            food_total: 0,
            btc_total: 0,
            living_total: scale_monthly_finance_amount(
                self.living_expense_monthly.max(0),
                period_code,
            ),
            learning_total: 0,
            other_expense_total: 0,
            days_with_entries: 0,
        }
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
                cashflow_comparison_available: false,
                net_worth_comparison_available: false,
            };
        }

        let (
            (current_year, current_month, current_day),
            (previous_year, previous_month, previous_day),
        ) = comparable_period_reference_dates(
            period_code,
            reference_year,
            reference_month,
            reference_day,
        );
        let current_aggregate =
            self.aggregate_for_period(period_code, current_year, current_month, current_day);
        let previous_aggregate =
            self.aggregate_for_period(period_code, previous_year, previous_month, previous_day);
        let current_expected_days =
            expected_recorded_days(period_code, current_year, current_month, current_day);
        let previous_expected_days =
            expected_recorded_days(period_code, previous_year, previous_month, previous_day);
        let cashflow_comparison_available = trend_samples_are_comparable(
            current_aggregate.days_with_entries,
            current_expected_days,
            previous_aggregate.days_with_entries,
            previous_expected_days,
        );
        let current_snapshot_key =
            self.latest_snapshot_month_key_up_to(&format!("{current_year:04}-{current_month:02}"));
        let previous_snapshot_key = self
            .latest_snapshot_month_key_up_to(&format!("{previous_year:04}-{previous_month:02}"));
        let net_worth_comparison_available = current_snapshot_key.is_some()
            && previous_snapshot_key.is_some()
            && current_snapshot_key != previous_snapshot_key;
        let current_summary = current_snapshot_key
            .as_ref()
            .and_then(|key| self.monthly_snapshots.get(key))
            .map(FinanceMonthSnapshot::to_summary_totals)
            .unwrap_or_else(FinanceMonthSnapshotTotals::empty);
        let previous_summary = previous_snapshot_key
            .as_ref()
            .and_then(|key| self.monthly_snapshots.get(key))
            .map(FinanceMonthSnapshot::to_summary_totals)
            .unwrap_or_else(FinanceMonthSnapshotTotals::empty);
        let target_drift = (current_aggregate.days_with_entries > 0)
            .then(|| self.most_problematic_target_drift(current_aggregate))
            .flatten();

        FinanceTrendValues {
            income_delta: if cashflow_comparison_available {
                current_aggregate
                    .income_total()
                    .saturating_sub(previous_aggregate.income_total())
            } else {
                0
            },
            outflow_delta: if cashflow_comparison_available {
                current_aggregate
                    .expense_total()
                    .saturating_sub(previous_aggregate.expense_total())
            } else {
                0
            },
            net_cashflow_delta: if cashflow_comparison_available {
                current_aggregate
                    .net_cashflow()
                    .saturating_sub(previous_aggregate.net_cashflow())
            } else {
                0
            },
            net_worth_delta: if net_worth_comparison_available {
                current_summary
                    .net_worth()
                    .saturating_sub(previous_summary.net_worth())
            } else {
                0
            },
            current_recorded_days: current_aggregate.days_with_entries,
            previous_recorded_days: previous_aggregate.days_with_entries,
            most_off_target_bucket_code: target_drift
                .map(|(bucket, _)| bucket_code(&bucket))
                .unwrap_or(BUCKET_CODE_NONE),
            most_off_target_ratio_delta: target_drift.map(|(_, drift)| drift).unwrap_or(0.0),
            cashflow_comparison_available,
            net_worth_comparison_available,
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

        let aggregate = self.report_aggregate_for_period(
            period_code,
            reference_year,
            reference_month,
            reference_day,
        );
        let uses_detailed_cashflow = self.uses_detailed_cashflow_for_period(
            period_code,
            reference_year,
            reference_month,
            reference_day,
        );
        let has_cashflow_evidence = uses_detailed_cashflow || self.has_legacy_cashflow_estimates();
        let latest_snapshot =
            self.latest_snapshot_summary_with_age(reference_year, reference_month);
        let has_fresh_snapshot = latest_snapshot
            .is_some_and(|(_, age_months)| age_months <= MAX_HEALTH_SNAPSHOT_AGE_MONTHS);
        let summary = latest_snapshot
            .filter(|(_, age_months)| *age_months <= MAX_HEALTH_SNAPSHOT_AGE_MONTHS)
            .map(|(summary, _)| summary)
            .unwrap_or_else(FinanceMonthSnapshotTotals::empty);
        let expected_days =
            expected_recorded_days(period_code, reference_year, reference_month, reference_day)
                .max(1);
        let defensive_base = aggregate
            .essential_outflow_total()
            .saturating_sub(aggregate.asset_income_total);
        let defensive_coverage = if defensive_base <= 0 {
            None
        } else if summary.cash_reserve_total <= 0 {
            Some(0.0)
        } else {
            Some(summary.cash_reserve_total as f32 / defensive_base as f32)
        };
        let defensive_coverage_months =
            defensive_coverage_in_months(defensive_coverage, expected_days);
        let negative_net_worth = has_fresh_snapshot && summary.net_worth() < 0;
        let high_balance_sheet_leverage = has_fresh_snapshot
            && summary.liability_total > 0
            && (summary.asset_total <= 0
                || summary.liability_total as f64 / summary.asset_total as f64 >= 0.80);

        let mut warnings = Vec::new();
        let mut infos = Vec::new();
        if has_cashflow_evidence && aggregate.net_cashflow() < 0 {
            warnings.push(FinanceAlertPlanItem {
                kind_code: ALERT_KIND_NEGATIVE_CASHFLOW,
                bucket_code: BUCKET_CODE_NONE,
                drift: 0.0,
            });
        }
        if negative_net_worth || high_balance_sheet_leverage {
            warnings.push(FinanceAlertPlanItem {
                kind_code: ALERT_KIND_BALANCE_SHEET_RISK,
                bucket_code: BUCKET_CODE_NONE,
                drift: 0.0,
            });
        }
        if !has_fresh_snapshot {
            warnings.push(FinanceAlertPlanItem {
                kind_code: ALERT_KIND_STALE_MONTH_SNAPSHOT,
                bucket_code: BUCKET_CODE_NONE,
                drift: 0.0,
            });
        }
        if has_fresh_snapshot && defensive_coverage_months.is_some_and(|value| value < 2.0) {
            warnings.push(FinanceAlertPlanItem {
                kind_code: ALERT_KIND_LOW_DEFENSIVE_COVERAGE,
                bucket_code: BUCKET_CODE_NONE,
                drift: 0.0,
            });
        }
        if uses_detailed_cashflow
            && expected_days > 0
            && aggregate.days_with_entries * 2 < expected_days as i64
        {
            warnings.push(FinanceAlertPlanItem {
                kind_code: ALERT_KIND_LOW_RECORD_DENSITY,
                bucket_code: BUCKET_CODE_NONE,
                drift: 0.0,
            });
        }
        let target_overcommit = self.settings.target_overcommit();
        if target_overcommit > TARGET_TOTAL_EPSILON {
            warnings.push(FinanceAlertPlanItem {
                kind_code: ALERT_KIND_TARGET_OVERCOMMITTED,
                bucket_code: BUCKET_CODE_NONE,
                drift: target_overcommit,
            });
        }
        if uses_detailed_cashflow {
            if let Some((bucket, drift)) = self.most_problematic_target_drift(aggregate) {
                warnings.push(FinanceAlertPlanItem {
                    kind_code: ALERT_KIND_TARGET_DRIFT,
                    bucket_code: bucket_code(&bucket),
                    drift,
                });
            }
        }
        if !has_cashflow_evidence {
            warnings.push(FinanceAlertPlanItem {
                kind_code: ALERT_KIND_CASHFLOW_MISSING,
                bucket_code: BUCKET_CODE_NONE,
                drift: 0.0,
            });
        } else if !uses_detailed_cashflow {
            warnings.push(FinanceAlertPlanItem {
                kind_code: ALERT_KIND_LEGACY_CASHFLOW_ESTIMATE,
                bucket_code: BUCKET_CODE_NONE,
                drift: 0.0,
            });
        }
        if has_fresh_snapshot && summary.asset_total <= 0 && summary.liability_total <= 0 {
            infos.push(FinanceAlertPlanItem {
                kind_code: ALERT_KIND_EMPTY_MONTH_SNAPSHOT,
                bucket_code: BUCKET_CODE_NONE,
                drift: 0.0,
            });
        }

        let mut plan = Vec::with_capacity(warnings.len().saturating_add(1));
        plan.extend(warnings);
        // Never hide a warning behind an arbitrary display cap. One lower-
        // priority informational item is enough to keep the page concise.
        plan.extend(infos.into_iter().take(1));
        if plan.is_empty() {
            plan.push(FinanceAlertPlanItem {
                kind_code: ALERT_KIND_CLEAN,
                bucket_code: BUCKET_CODE_NONE,
                drift: 0.0,
            });
        }
        plan
    }

    fn detailed_finance_snapshot_values(
        &self,
        reference_year: i32,
        reference_month: i32,
    ) -> FinanceSnapshotValues {
        let aggregate = self.report_aggregate_for_period(
            PERIOD_MONTH,
            reference_year,
            reference_month,
            days_in_month(reference_year, reference_month),
        );
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
        let aggregate = self.report_aggregate_for_period(
            period_code,
            reference_year,
            reference_month,
            reference_day,
        );
        let summary = self.latest_snapshot_summary_up_to(reference_year, reference_month);
        snapshot_values_from_totals(aggregate, summary, aggregate.days_with_entries)
    }

    fn finance_health_score_values(
        &self,
        period_code: i32,
        reference_year: i32,
        reference_month: i32,
        reference_day: i32,
    ) -> FinanceHealthScoreValues {
        let aggregate = self.report_aggregate_for_period(
            period_code,
            reference_year,
            reference_month,
            reference_day,
        );
        let uses_detailed_cashflow = self.uses_detailed_cashflow_for_period(
            period_code,
            reference_year,
            reference_month,
            reference_day,
        );
        let latest_snapshot =
            self.latest_snapshot_summary_with_age(reference_year, reference_month);
        let has_fresh_snapshot = latest_snapshot
            .is_some_and(|(_, age_months)| age_months <= MAX_HEALTH_SNAPSHOT_AGE_MONTHS);
        let summary = latest_snapshot
            .filter(|(_, age_months)| *age_months <= MAX_HEALTH_SNAPSHOT_AGE_MONTHS)
            .map(|(summary, _)| summary)
            .unwrap_or_else(FinanceMonthSnapshotTotals::empty);
        let snapshot = snapshot_values_from_totals(aggregate, summary, aggregate.days_with_entries);
        let trend =
            self.finance_trend_values(period_code, reference_year, reference_month, reference_day);
        let target_drift = if uses_detailed_cashflow {
            self.most_problematic_target_drift(aggregate)
        } else {
            None
        };
        let discipline_drift = if uses_detailed_cashflow {
            self.most_target_discipline_drift(aggregate)
        } else {
            None
        };
        let target_overcommit = self.settings.target_overcommit();
        let expected_days_count =
            expected_recorded_days(period_code, reference_year, reference_month, reference_day)
                .max(1);
        let expected_days = expected_days_count as f32;
        let record_coverage = if uses_detailed_cashflow {
            (aggregate.days_with_entries as f32 / expected_days).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let low_record_density = uses_detailed_cashflow && record_coverage < 0.50;
        let legacy_cashflow_estimate = !uses_detailed_cashflow;

        let cashflow_retention = if snapshot.total_income <= 0 {
            0.0
        } else {
            snapshot.net_cashflow as f32 / snapshot.total_income as f32
        };
        let cashflow_score = normalized_finance_score(cashflow_retention, -0.05, 0.20);
        let defensive_coverage_months =
            defensive_coverage_in_months(snapshot.defensive_coverage, expected_days as i32);
        let defensive_score = match defensive_coverage_months {
            None => 1.0,
            Some(value) => normalized_finance_score(value, 0.0, 6.0),
        };
        let cashflow_liability_score =
            (1.0 - (snapshot.liability_pressure_ratio / 0.35)).clamp(0.0, 1.0);
        let liability_score = if has_fresh_snapshot {
            cashflow_liability_score.min(balance_sheet_liability_score(summary))
        } else {
            // Missing or stale balance-sheet data must not be interpreted as zero debt.
            cashflow_liability_score.min(0.35)
        };
        let passive_score = (snapshot.passive_coverage_ratio / 0.50).clamp(0.0, 1.0);
        let wage_score = if snapshot.total_income <= 0 {
            0.0
        } else {
            (1.0 - snapshot.wage_dependence_ratio).clamp(0.0, 1.0)
        };
        let record_score = if uses_detailed_cashflow {
            record_coverage
        } else {
            // Legacy monthly inputs can support a rough directional view, but
            // they cannot prove day-level completeness.
            0.50
        };
        let observed_discipline_score = discipline_drift
            .map(|(_, drift)| {
                (1.0 - (drift.abs() / TARGET_DISCIPLINE_PENALTY_SPAN)).clamp(0.0, 1.0)
            })
            .unwrap_or(1.0);
        let configuration_discipline_score = target_overcommit_discipline_score(target_overcommit);
        let discipline_score = observed_discipline_score.min(configuration_discipline_score);
        let momentum_score = finance_momentum_score(aggregate, trend);

        let negative_net_worth = has_fresh_snapshot && summary.net_worth() < 0;
        let high_balance_sheet_leverage = has_fresh_snapshot
            && summary.liability_total > 0
            && (summary.asset_total <= 0
                || summary.liability_total as f64 / summary.asset_total as f64 >= 0.80);
        let warning_count = [
            aggregate.net_cashflow() < 0,
            has_fresh_snapshot && defensive_coverage_months.is_some_and(|coverage| coverage < 2.0),
            target_drift.is_some(),
            target_overcommit > TARGET_TOTAL_EPSILON,
            !has_fresh_snapshot,
            negative_net_worth || high_balance_sheet_leverage,
            low_record_density,
            legacy_cashflow_estimate,
        ]
        .into_iter()
        .filter(|flag| *flag)
        .count() as i32;

        let weighted_score = cashflow_score * 0.22
            + defensive_score * 0.18
            + liability_score * 0.15
            + passive_score * 0.12
            + wage_score * 0.08
            + record_score * 0.10
            + discipline_score * 0.10
            + momentum_score * 0.05;
        let raw_score = ((weighted_score * 100.0).round() as i32).clamp(0, 99);
        // Confidence changes continuously with confirmed/recorded coverage so
        // there is no artificial cliff at a single threshold. Legacy monthly
        // estimates remain explicitly provisional.
        let confidence_cap = if uses_detailed_cashflow {
            (55.0 + record_coverage * 44.0).round() as i32
        } else {
            69
        };
        let score = raw_score.min(confidence_cap.clamp(55, 99));
        let risk_level_code = finance_risk_level_code(
            score,
            warning_count,
            aggregate,
            snapshot,
            defensive_coverage_months,
        );

        FinanceHealthScoreValues {
            score,
            risk_level_code,
            cashflow_score,
            defensive_score,
            liability_score,
            passive_score,
            wage_score,
            record_score,
            discipline_score,
            momentum_score,
            warning_count,
        }
    }

    fn finance_risk_cockpit_snapshot(
        &self,
        period_code: i32,
        reference_year: i32,
        reference_month: i32,
        reference_day: i32,
    ) -> FinanceRiskCockpitSnapshot {
        let detailed =
            self.aggregate_for_period(period_code, reference_year, reference_month, reference_day);
        let uses_detailed_cashflow = self.detailed_cashflow_is_authoritative(
            detailed,
            period_code,
            reference_year,
            reference_month,
            reference_day,
        );
        let aggregate = if uses_detailed_cashflow {
            detailed
        } else {
            self.legacy_cashflow_aggregate(period_code)
        };
        let expected_days =
            expected_recorded_days(period_code, reference_year, reference_month, reference_day)
                .max(1);
        let coverage = if uses_detailed_cashflow {
            (detailed.days_with_entries as f32 / expected_days as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let forecast_available = uses_detailed_cashflow
            && detailed.days_with_entries >= FORECAST_MIN_RECORDED_DAYS
            && coverage >= FORECAST_MIN_COVERAGE;

        let latest_snapshot =
            self.latest_snapshot_summary_with_age(reference_year, reference_month);
        let balance_sheet_available = latest_snapshot.is_some();
        // The cockpit's cash-balance and debt-cycle projections require a
        // snapshot for the selected reference month. Health scoring below can
        // still treat a snapshot up to three months old as recent evidence.
        let has_fresh_snapshot = latest_snapshot.is_some_and(|(_, age_months)| age_months == 0);
        // An older snapshot can prove that balance-sheet history exists, but
        // it must never be presented as the selected month's cash or debt.
        let current_summary = latest_snapshot
            .filter(|(_, age_months)| *age_months == 0)
            .map(|(summary, _)| summary)
            .unwrap_or_else(FinanceMonthSnapshotTotals::empty);

        let cashflow_confidence = if uses_detailed_cashflow {
            let coverage_confidence = (coverage * 75.0).round() as i32;
            if forecast_available {
                coverage_confidence
            } else {
                coverage_confidence.min(45)
            }
        } else if self.has_legacy_cashflow_estimates() {
            20
        } else {
            0
        };
        let balance_sheet_confidence = match latest_snapshot.map(|(_, age_months)| age_months) {
            Some(0) => 25,
            Some(1..=MAX_HEALTH_SNAPSHOT_AGE_MONTHS) => 10,
            Some(_) => 5,
            None => 0,
        };
        let data_confidence = cashflow_confidence
            .saturating_add(balance_sheet_confidence)
            .clamp(0, 100);

        let has_cashflow_evidence = uses_detailed_cashflow || self.has_legacy_cashflow_estimates();
        let (risk_state, reason_code) = if has_fresh_snapshot && current_summary.net_worth() < 0 {
            (
                FinanceRiskState::Critical,
                REASON_OBSERVED_NEGATIVE_NET_WORTH,
            )
        } else if has_cashflow_evidence && aggregate.net_cashflow() < 0 {
            (FinanceRiskState::Tight, REASON_OBSERVED_NEGATIVE_CASHFLOW)
        } else if !has_cashflow_evidence {
            (FinanceRiskState::Unknown, REASON_CASHFLOW_BASELINE_MISSING)
        } else if !forecast_available {
            (
                FinanceRiskState::Unknown,
                REASON_CASHFLOW_COVERAGE_INSUFFICIENT,
            )
        } else if !has_fresh_snapshot {
            (
                FinanceRiskState::Unknown,
                REASON_CURRENT_BALANCE_SHEET_MISSING,
            )
        } else {
            match self
                .finance_health_score_values(
                    period_code,
                    reference_year,
                    reference_month,
                    reference_day,
                )
                .risk_level_code
            {
                RISK_LEVEL_CRITICAL => (FinanceRiskState::Critical, REASON_POLICY_CRITICAL),
                RISK_LEVEL_TIGHT => (FinanceRiskState::Tight, REASON_POLICY_TIGHT),
                RISK_LEVEL_WATCH => (FinanceRiskState::Watch, REASON_POLICY_WATCH),
                _ => (FinanceRiskState::Stable, REASON_POLICY_STABLE),
            }
        };

        let budget_envelopes =
            finance_budget_envelopes(aggregate, &self.settings.clone().sanitized());
        let protected_allocation = budget_envelopes
            .iter()
            .filter(|envelope| {
                matches!(
                    envelope.bucket_code,
                    BUCKET_CODE_BTC | BUCKET_CODE_LEARNING | BUCKET_CODE_OTHER
                )
            })
            .map(|envelope| envelope.remaining_amount.max(0))
            .fold(0_i64, i64::saturating_add);
        let pending_recurring_by_bucket = self.pending_recurring_by_bucket(
            period_code,
            reference_year,
            reference_month,
            reference_day,
        );
        let pending_recurring_reserve =
            finance_pending_recurring_reserve(&pending_recurring_by_bucket, &budget_envelopes);
        let safe_to_spend = aggregate
            .net_cashflow()
            .saturating_sub(protected_allocation)
            .saturating_sub(pending_recurring_reserve)
            .max(0);

        let projected_monthly_income = if forecast_available {
            scale_elapsed_amount_to_30_days(detailed.income_total(), expected_days)
        } else {
            0
        };
        let projected_monthly_outflow = if forecast_available {
            scale_elapsed_amount_to_30_days(detailed.expense_total(), expected_days)
        } else {
            0
        };
        let projected_monthly_essential = if forecast_available {
            scale_elapsed_amount_to_30_days(detailed.essential_outflow_total(), expected_days)
        } else if self.has_legacy_cashflow_estimates() {
            self.legacy_cashflow_aggregate(PERIOD_MONTH)
                .essential_outflow_total()
        } else {
            0
        };
        let projected_monthly_net = if forecast_available {
            projected_monthly_income.saturating_sub(projected_monthly_outflow)
        } else {
            0
        };
        let stressed_monthly_income = if forecast_available {
            scale_finance_amount(projected_monthly_income, 0.80)
        } else {
            0
        };
        let stressed_monthly_outflow = if forecast_available {
            scale_finance_amount(projected_monthly_outflow, 1.15)
        } else {
            0
        };
        let stressed_monthly_net = if forecast_available {
            stressed_monthly_income.saturating_sub(stressed_monthly_outflow)
        } else {
            0
        };

        let cash_reserve = current_summary.cash_reserve_total;
        let debt_balance = current_summary.liability_total;
        let projected_balance30_days = if forecast_available && has_fresh_snapshot {
            cash_reserve.saturating_add(projected_monthly_net)
        } else {
            0
        };
        let projected_balance90_days = if forecast_available && has_fresh_snapshot {
            cash_reserve.saturating_add(projected_monthly_net.saturating_mul(3))
        } else {
            0
        };
        let stressed_balance30_days = if forecast_available && has_fresh_snapshot {
            cash_reserve.saturating_add(stressed_monthly_net)
        } else {
            0
        };

        let emergency_target = projected_monthly_essential.saturating_mul(6);
        let emergency_gap = if has_fresh_snapshot {
            emergency_target.saturating_sub(cash_reserve).max(0)
        } else {
            0
        };
        let emergency_months = if has_fresh_snapshot && projected_monthly_essential > 0 {
            Some(cash_reserve as f32 / projected_monthly_essential as f32)
        } else {
            None
        };
        let monthly_debt_payment = if forecast_available {
            scale_elapsed_amount_to_30_days(detailed.debt_total, expected_days)
        } else if self.has_legacy_cashflow_estimates() {
            self.liability_payment_monthly.max(0)
        } else {
            0
        };
        // This is principal divided by observed monthly payment. No interest
        // rate is available, so no amortisation schedule is invented here.
        let debt_free_months = if has_fresh_snapshot && debt_balance > 0 && monthly_debt_payment > 0
        {
            Some(debt_balance as f32 / monthly_debt_payment as f32)
        } else {
            None
        };

        let recurring_candidates =
            self.recent_recurring_candidates(reference_year, reference_month, reference_day);
        let recurring_monthly_estimate = recurring_candidates
            .iter()
            .map(|candidate| candidate.monthly_estimate)
            .fold(0_i64, i64::saturating_add);
        let (anomaly_day_key, anomaly_amount, anomaly_ratio) =
            self.recent_expense_anomaly(reference_year, reference_month, reference_day);

        let actions = finance_risk_actions(
            uses_detailed_cashflow,
            forecast_available,
            balance_sheet_available,
            has_fresh_snapshot,
            aggregate,
            projected_monthly_net,
            stressed_monthly_net,
            emergency_gap,
            emergency_months,
            debt_balance,
            monthly_debt_payment,
            debt_free_months,
            &budget_envelopes,
            recurring_monthly_estimate,
            anomaly_ratio,
            anomaly_amount,
        );

        FinanceRiskCockpitSnapshot {
            risk_state,
            reason_code: reason_code.to_string(),
            policy_version: FINANCE_RISK_POLICY_VERSION,
            data_confidence,
            uses_detailed_cashflow,
            forecast_available,
            balance_sheet_available,
            has_fresh_snapshot,
            safe_to_spend,
            protected_allocation,
            pending_recurring_reserve,
            projected_monthly_net,
            projected_balance30_days,
            projected_balance90_days,
            stressed_balance30_days,
            cash_reserve,
            emergency_target,
            emergency_gap,
            emergency_months,
            debt_balance,
            monthly_debt_payment,
            debt_free_months,
            recurring_monthly_estimate,
            anomaly_day_key,
            anomaly_amount,
            anomaly_ratio,
            budget_envelopes,
            recurring_candidates,
            actions,
        }
    }

    fn recent_recurring_candidates(
        &self,
        reference_year: i32,
        reference_month: i32,
        reference_day: i32,
    ) -> Vec<FinanceRecurringCandidate> {
        let reference_ordinal = days_from_civil(
            reference_year as i64,
            reference_month as i64,
            reference_day as i64,
        );
        let mut grouped = HashMap::<String, RecurringExpenseAccumulator>::new();
        for (day_key, ledger) in &self.daily_ledgers {
            let Some((year, month, day)) = parse_finance_day_key(day_key) else {
                continue;
            };
            let ordinal = days_from_civil(year as i64, month as i64, day as i64);
            let age = reference_ordinal.saturating_sub(ordinal);
            if !(0..RECENT_RISK_WINDOW_DAYS).contains(&age) {
                continue;
            }
            let mut same_day = HashMap::<String, (String, i64)>::new();
            for expense in &ledger.expenses {
                if expense.is_deleted() || expense.amount <= 0 {
                    continue;
                }
                let display_name = compact_whitespace(&expense.name);
                let normalized_name = display_name.to_lowercase();
                if normalized_name.is_empty() {
                    continue;
                }
                let day_value = same_day
                    .entry(normalized_name)
                    .or_insert_with(|| (display_name, 0));
                day_value.1 = day_value.1.saturating_add(expense.amount.max(0));
            }
            for (normalized_name, (display_name, amount)) in same_day {
                let accumulator =
                    grouped
                        .entry(normalized_name)
                        .or_insert_with(|| RecurringExpenseAccumulator {
                            name: display_name,
                            day_amounts: Vec::new(),
                        });
                accumulator.day_amounts.push((ordinal, amount));
            }
        }

        let mut candidates = grouped
            .into_values()
            .filter_map(recurring_candidate_from_accumulator)
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            right
                .monthly_estimate
                .cmp(&left.monthly_estimate)
                .then_with(|| right.occurrences.cmp(&left.occurrences))
                .then_with(|| left.name.cmp(&right.name))
        });
        candidates.truncate(5);
        candidates
    }

    fn pending_recurring_by_bucket(
        &self,
        period_code: i32,
        reference_year: i32,
        reference_month: i32,
        reference_day: i32,
    ) -> [i64; 6] {
        let reference_month_index = month_index(reference_year, reference_month);
        let mut histories = HashMap::<(String, i32), PendingMonthlyExpenseHistory>::new();
        for (day_key, ledger) in &self.daily_ledgers {
            let Some((year, month, day)) = parse_finance_day_key(day_key) else {
                continue;
            };
            let age_months = reference_month_index.saturating_sub(month_index(year, month));
            if !(0..=3).contains(&age_months) || (age_months == 0 && day > reference_day) {
                continue;
            }
            let mut same_day = HashMap::<(String, i32), i64>::new();
            for expense in &ledger.expenses {
                if expense.is_deleted() || expense.amount <= 0 {
                    continue;
                }
                let normalized_name = compact_whitespace(&expense.name).to_lowercase();
                if normalized_name.is_empty() {
                    continue;
                }
                let key = (normalized_name, bucket_code(&expense.bucket));
                same_day
                    .entry(key)
                    .and_modify(|amount| *amount = amount.saturating_add(expense.amount.max(0)))
                    .or_insert(expense.amount.max(0));
            }
            for (key, amount) in same_day {
                let history = histories.entry(key).or_default();
                if age_months == 0 {
                    history.seen_in_current_month = true;
                } else {
                    history.previous_months[(age_months - 1) as usize].push((day, amount));
                }
            }
        }

        let mut by_bucket = [0_i64; 6];
        for ((_, bucket_code), history) in histories {
            if history.seen_in_current_month
                || history
                    .previous_months
                    .iter()
                    .any(|month_entries| month_entries.len() != 1)
            {
                continue;
            }
            let mut days = history
                .previous_months
                .iter()
                .map(|entries| entries[0].0)
                .collect::<Vec<_>>();
            let amounts = history
                .previous_months
                .iter()
                .map(|entries| entries[0].1)
                .collect::<Vec<_>>();
            let Some(min_day) = days.iter().min().copied() else {
                continue;
            };
            let Some(max_day) = days.iter().max().copied() else {
                continue;
            };
            if max_day.saturating_sub(min_day) > 3 {
                continue;
            }
            let Some(min_amount) = amounts.iter().min().copied() else {
                continue;
            };
            let Some(max_amount) = amounts.iter().max().copied() else {
                continue;
            };
            let amount_tolerance = (min_amount / 10).max(
                if self.calculation_money_unit.as_deref() == Some("rmb-cent-v1") {
                    100
                } else {
                    1
                },
            );
            if max_amount.saturating_sub(min_amount) > amount_tolerance {
                continue;
            }
            days.sort_unstable();
            let expected_day =
                days[days.len() / 2].min(days_in_month(reference_year, reference_month));
            if period_code == PERIOD_DAY && expected_day != reference_day {
                continue;
            }
            let Ok(bucket_index) = usize::try_from(bucket_code) else {
                continue;
            };
            if let Some(total) = by_bucket.get_mut(bucket_index) {
                *total = total.saturating_add(max_amount);
            }
        }
        by_bucket
    }

    fn recent_expense_anomaly(
        &self,
        reference_year: i32,
        reference_month: i32,
        reference_day: i32,
    ) -> (String, i64, f32) {
        let reference_ordinal = days_from_civil(
            reference_year as i64,
            reference_month as i64,
            reference_day as i64,
        );
        let mut samples = self
            .daily_ledgers
            .iter()
            .filter_map(|(day_key, ledger)| {
                let (year, month, day) = parse_finance_day_key(day_key)?;
                let ordinal = days_from_civil(year as i64, month as i64, day as i64);
                let age = reference_ordinal.saturating_sub(ordinal);
                if !(0..RECENT_RISK_WINDOW_DAYS).contains(&age) {
                    return None;
                }
                let amount = ledger.to_aggregate_totals().expense_total();
                (amount > 0).then(|| (ordinal, day_key.clone(), amount))
            })
            .collect::<Vec<_>>();
        samples.sort_by_key(|sample| sample.0);

        let mut best: Option<(String, i64, f32)> = None;
        for index in 3..samples.len() {
            let mut history = samples[..index]
                .iter()
                .map(|sample| sample.2)
                .collect::<Vec<_>>();
            history.sort_unstable();
            let median = history[history.len() / 2];
            if median <= 0 {
                continue;
            }
            let amount = samples[index].2;
            let ratio = amount as f32 / median as f32;
            if !ratio.is_finite() || ratio < 2.0 {
                continue;
            }
            let candidate = (samples[index].1.clone(), amount, ratio);
            if best.as_ref().is_none_or(|current| {
                candidate.2 > current.2
                    || ((candidate.2 - current.2).abs() <= f32::EPSILON && candidate.1 > current.1)
            }) {
                best = Some(candidate);
            }
        }
        best.unwrap_or_else(|| (String::new(), 0, 0.0))
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
            // Detailed cashflow entries replace the legacy monthly estimates,
            // but they must not erase the user's existing balance-sheet baseline.
            // The baseline remains display-only until a dated month snapshot is
            // recorded; health scoring still treats the missing snapshot as risk.
            return self
                .legacy_balance_sheet_totals()
                .unwrap_or_else(FinanceMonthSnapshotTotals::empty);
        };
        self.monthly_snapshots
            .get(&key)
            .map(FinanceMonthSnapshot::to_summary_totals)
            .unwrap_or_else(FinanceMonthSnapshotTotals::empty)
    }

    fn legacy_balance_sheet_totals(&self) -> Option<FinanceMonthSnapshotTotals> {
        let cash_reserve_total = self.cash_reserve.max(0);
        let productive_asset_total = self.productive_asset_value.max(0);
        let liability_total = self.liability_balance.max(0);
        if cash_reserve_total == 0 && productive_asset_total == 0 && liability_total == 0 {
            return None;
        }
        Some(FinanceMonthSnapshotTotals {
            asset_total: cash_reserve_total.saturating_add(productive_asset_total),
            liability_total,
            cash_reserve_total,
            productive_asset_total,
        })
    }

    fn latest_snapshot_summary_with_age(
        &self,
        reference_year: i32,
        reference_month: i32,
    ) -> Option<(FinanceMonthSnapshotTotals, i32)> {
        if reference_year <= 0 || !(1..=12).contains(&reference_month) {
            return None;
        }
        let reference_key = format!("{reference_year:04}-{reference_month:02}");
        let key = self.latest_snapshot_month_key_up_to(&reference_key)?;
        let (snapshot_year, snapshot_month) = parse_finance_month_key(&key)?;
        let age_months = month_index(reference_year, reference_month)
            .saturating_sub(month_index(snapshot_year, snapshot_month));
        let summary = self
            .monthly_snapshots
            .get(&key)
            .map(FinanceMonthSnapshot::to_summary_totals)?;
        Some((summary, age_months))
    }

    fn latest_snapshot_month_key_up_to(&self, reference_month_key: &str) -> Option<String> {
        self.monthly_snapshots
            .iter()
            .filter(|(key, snapshot)| {
                is_valid_finance_month_key(key)
                    && key.as_str() <= reference_month_key
                    && snapshot.has_financial_entries()
            })
            .map(|(key, _)| key)
            .max()
            .cloned()
    }

    fn previous_recorded_day_key(&self, before_day_key: &str) -> Option<String> {
        self.daily_ledgers
            .iter()
            .filter(|(key, ledger)| {
                is_valid_finance_day_key(key)
                    && key.as_str() < before_day_key
                    && ledger.has_recorded_finance_day()
            })
            .map(|(key, _)| key)
            .max()
            .cloned()
    }

    fn previous_recorded_month_key(&self, before_month_key: &str) -> Option<String> {
        self.monthly_snapshots
            .iter()
            .filter(|(key, snapshot)| {
                is_valid_finance_month_key(key)
                    && key.as_str() < before_month_key
                    && snapshot.has_financial_entries()
            })
            .map(|(key, _)| key)
            .max()
            .cloned()
    }

    fn year_net_worth_summary(&self, year: i32, through_month: i32) -> FinanceYearNetWorthValues {
        if year <= 0 || !(1..=12).contains(&through_month) {
            return FinanceYearNetWorthValues::default();
        }
        let mut keys = self
            .monthly_snapshots
            .iter()
            .filter(|(key, snapshot)| {
                parse_finance_month_key(key).is_some_and(|(snapshot_year, snapshot_month)| {
                    snapshot_year == year
                        && snapshot_month <= through_month
                        && snapshot.has_financial_entries()
                })
            })
            .map(|(key, _)| key)
            .cloned()
            .collect::<Vec<_>>();
        keys.sort();
        let Some(first_key) = keys.first() else {
            return FinanceYearNetWorthValues::default();
        };
        let Some(last_key) = keys.last() else {
            return FinanceYearNetWorthValues::default();
        };
        let prior_year_end_key = format!("{:04}-12", year - 1);
        let prior_opening_key = self.latest_snapshot_month_key_up_to(&prior_year_end_key);
        let has_calendar_year_opening = prior_opening_key.is_some();
        let opening_key = prior_opening_key.unwrap_or_else(|| first_key.clone());
        let opening = self
            .monthly_snapshots
            .get(&opening_key)
            .map(FinanceMonthSnapshot::to_summary_totals)
            .unwrap_or_else(FinanceMonthSnapshotTotals::empty)
            .net_worth();
        let closing = self
            .monthly_snapshots
            .get(last_key)
            .map(FinanceMonthSnapshot::to_summary_totals)
            .unwrap_or_else(FinanceMonthSnapshotTotals::empty)
            .net_worth();
        let opening_month_code = parse_finance_month_key(&opening_key)
            .map(|(snapshot_year, snapshot_month)| {
                i64::from(snapshot_year) * 100 + i64::from(snapshot_month)
            })
            .unwrap_or_default();
        let closing_month_code = parse_finance_month_key(last_key)
            .map(|(snapshot_year, snapshot_month)| {
                i64::from(snapshot_year) * 100 + i64::from(snapshot_month)
            })
            .unwrap_or_default();
        FinanceYearNetWorthValues {
            opening_net_worth: opening,
            closing_net_worth: closing,
            opening_month_code,
            closing_month_code,
            has_calendar_year_opening,
        }
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

    fn most_target_discipline_drift(
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
                is_target_discipline_violation(&bucket, drift).then_some((bucket, drift))
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
        Self {
            expense_categories: self
                .expense_categories
                .into_iter()
                .map(FinanceExpenseCategoryConfig::sanitized)
                .collect(),
        }
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

    fn configured_minimum_target_total(&self) -> f32 {
        [
            FinanceExpenseBucket::Btc,
            FinanceExpenseBucket::Learning,
            FinanceExpenseBucket::Other,
        ]
        .into_iter()
        .filter_map(|bucket| self.target_for(&bucket))
        .filter(|target| target.is_finite())
        .sum()
    }

    fn target_overcommit(&self) -> f32 {
        (self.configured_minimum_target_total() - 1.0).max(0.0)
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
    fn sanitized(mut self) -> Self {
        self.target_share_of_income = self
            .target_share_of_income
            .map(|value| value.clamp(0.0, 1.0));
        self
    }
}

impl FinanceIncomeEntry {
    fn sanitized(mut self) -> Self {
        self.id = self.id.trim().to_string();
        self.updated_at_epoch_millis = self.updated_at_epoch_millis.max(0);
        self.deleted_at_epoch_millis = self.deleted_at_epoch_millis.max(0);
        self
    }

    fn is_deleted(&self) -> bool {
        self.deleted_at_epoch_millis > 0
    }

    fn has_entries(&self) -> bool {
        !self.is_deleted()
            && ((self.amount > 0 || self.amount_minor.is_some_and(|value| value > 0))
                || !self.name.trim().is_empty()
                || !self.note.trim().is_empty())
    }

    fn should_persist(&self) -> bool {
        self.has_entries() || (self.is_deleted() && !self.id.trim().is_empty())
    }
}

impl FinanceExpenseEntry {
    fn sanitized(mut self) -> Self {
        self.id = self.id.trim().to_string();
        self.updated_at_epoch_millis = self.updated_at_epoch_millis.max(0);
        self.deleted_at_epoch_millis = self.deleted_at_epoch_millis.max(0);
        self
    }

    fn is_deleted(&self) -> bool {
        self.deleted_at_epoch_millis > 0
    }

    fn has_entries(&self) -> bool {
        !self.is_deleted()
            && ((self.amount > 0 || self.amount_minor.is_some_and(|value| value > 0))
                || !self.name.trim().is_empty()
                || !self.note.trim().is_empty())
    }

    fn should_persist(&self) -> bool {
        self.has_entries() || (self.is_deleted() && !self.id.trim().is_empty())
    }
}

impl FinanceNamedAmountEntry {
    fn sanitized_asset(mut self) -> Self {
        self.id = self.id.trim().to_string();
        self.updated_at_epoch_millis = self.updated_at_epoch_millis.max(0);
        self.deleted_at_epoch_millis = self.deleted_at_epoch_millis.max(0);
        self
    }

    fn sanitized_liability(mut self) -> Self {
        self.id = self.id.trim().to_string();
        self.updated_at_epoch_millis = self.updated_at_epoch_millis.max(0);
        self.deleted_at_epoch_millis = self.deleted_at_epoch_millis.max(0);
        self
    }

    fn is_deleted(&self) -> bool {
        self.deleted_at_epoch_millis > 0
    }

    fn has_entries(&self) -> bool {
        !self.is_deleted()
            && ((self.amount > 0 || self.amount_minor.is_some_and(|value| value > 0))
                || !self.name.trim().is_empty())
    }

    fn should_persist(&self) -> bool {
        self.has_entries() || (self.is_deleted() && !self.id.trim().is_empty())
    }
}

impl FinanceDayLedger {
    fn sanitized(self) -> Self {
        Self {
            incomes: self
                .incomes
                .into_iter()
                .map(FinanceIncomeEntry::sanitized)
                .filter(FinanceIncomeEntry::should_persist)
                .collect(),
            expenses: self
                .expenses
                .into_iter()
                .map(FinanceExpenseEntry::sanitized)
                .filter(FinanceExpenseEntry::should_persist)
                .collect(),
            note: self.note,
            confirmed_at_epoch_millis: self.confirmed_at_epoch_millis.max(0),
        }
    }

    fn has_entries(&self) -> bool {
        self.incomes.iter().any(FinanceIncomeEntry::has_entries)
            || self.expenses.iter().any(FinanceExpenseEntry::has_entries)
            || !self.note.trim().is_empty()
            || self.confirmed_at_epoch_millis > 0
    }

    fn has_persisted_entries(&self) -> bool {
        self.incomes.iter().any(FinanceIncomeEntry::should_persist)
            || self
                .expenses
                .iter()
                .any(FinanceExpenseEntry::should_persist)
            || !self.note.trim().is_empty()
            || self.confirmed_at_epoch_millis > 0
    }

    fn has_financial_entries(&self) -> bool {
        self.incomes.iter().any(|entry| {
            !entry.is_deleted()
                && (entry.amount > 0 || entry.amount_minor.is_some_and(|value| value > 0))
        }) || self.expenses.iter().any(|entry| {
            !entry.is_deleted()
                && (entry.amount > 0 || entry.amount_minor.is_some_and(|value| value > 0))
        })
    }

    fn has_recorded_finance_day(&self) -> bool {
        self.has_financial_entries() || self.confirmed_at_epoch_millis > 0
    }

    fn to_aggregate_totals(&self) -> FinanceLedgerTotals {
        let mut aggregate = FinanceLedgerTotals::empty();
        for entry in &self.incomes {
            if entry.is_deleted() {
                continue;
            }
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
            if entry.is_deleted() {
                continue;
            }
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
        aggregate.days_with_entries = if self.has_recorded_finance_day() {
            1
        } else {
            0
        };
        aggregate
    }
}

impl FinanceMonthSnapshot {
    fn sanitized(self) -> Self {
        let mut assets = Vec::new();
        let mut liabilities = Vec::new();
        for entry in self.assets {
            let entry = entry.sanitized_asset();
            if !entry.should_persist() {
                continue;
            }
            if is_asset_kind(&entry.kind) {
                assets.push(entry);
            } else {
                liabilities.push(entry);
            }
        }
        for entry in self.liabilities {
            let entry = entry.sanitized_liability();
            if !entry.should_persist() {
                continue;
            }
            if is_liability_kind(&entry.kind) {
                liabilities.push(entry);
            } else {
                assets.push(entry);
            }
        }
        Self {
            assets,
            liabilities,
            note: self.note,
            confirmed_at_epoch_millis: self.confirmed_at_epoch_millis.max(0),
        }
    }

    fn has_entries(&self) -> bool {
        self.assets.iter().any(FinanceNamedAmountEntry::has_entries)
            || self
                .liabilities
                .iter()
                .any(FinanceNamedAmountEntry::has_entries)
            || !self.note.trim().is_empty()
            || self.confirmed_at_epoch_millis > 0
    }

    fn has_persisted_entries(&self) -> bool {
        self.assets
            .iter()
            .any(FinanceNamedAmountEntry::should_persist)
            || self
                .liabilities
                .iter()
                .any(FinanceNamedAmountEntry::should_persist)
            || !self.note.trim().is_empty()
            || self.confirmed_at_epoch_millis > 0
    }

    fn has_financial_entries(&self) -> bool {
        self.assets.iter().any(|entry| {
            !entry.is_deleted()
                && (entry.amount > 0 || entry.amount_minor.is_some_and(|value| value > 0))
        }) || self.liabilities.iter().any(|entry| {
            !entry.is_deleted()
                && (entry.amount > 0 || entry.amount_minor.is_some_and(|value| value > 0))
        }) || self.confirmed_at_epoch_millis > 0
    }

    fn to_summary_totals(&self) -> FinanceMonthSnapshotTotals {
        let mut summary = FinanceMonthSnapshotTotals::empty();
        for entry in &self.assets {
            if entry.is_deleted() {
                continue;
            }
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
            if entry.is_deleted() {
                continue;
            }
            summary.liability_total = summary.liability_total.saturating_add(entry.amount.max(0));
        }
        summary
    }
}

fn sanitize_finance_day_ledgers(
    values: IndexMap<String, FinanceDayLedger>,
) -> IndexMap<String, FinanceDayLedger> {
    let mut sanitized = IndexMap::new();
    for (key, value) in values {
        if !is_valid_finance_day_key(&key) {
            continue;
        }
        let value = value.sanitized();
        if value.has_persisted_entries() {
            sanitized.insert(key, value);
        }
    }
    sanitized
}

fn sanitize_finance_month_snapshots(
    values: IndexMap<String, FinanceMonthSnapshot>,
) -> IndexMap<String, FinanceMonthSnapshot> {
    let mut sanitized = IndexMap::new();
    for (key, value) in values {
        if !is_valid_finance_month_key(&key) {
            continue;
        }
        let value = value.sanitized();
        if value.has_persisted_entries() {
            sanitized.insert(key, value);
        }
    }
    sanitized
}

fn assign_finance_profile_entry_ids(
    daily_ledgers: &mut IndexMap<String, FinanceDayLedger>,
    monthly_snapshots: &mut IndexMap<String, FinanceMonthSnapshot>,
) {
    let explicit_id_owners = collect_finance_entry_id_owners(daily_ledgers, monthly_snapshots);
    let reserved_explicit_ids = explicit_id_owners.keys().cloned().collect::<HashSet<_>>();
    let mut used_entry_ids = HashSet::new();
    for (key, ledger) in daily_ledgers {
        assign_income_entry_ids(
            &mut ledger.incomes,
            key,
            &explicit_id_owners,
            &reserved_explicit_ids,
            &mut used_entry_ids,
        );
        assign_expense_entry_ids(
            &mut ledger.expenses,
            key,
            &explicit_id_owners,
            &reserved_explicit_ids,
            &mut used_entry_ids,
        );
    }
    for (key, snapshot) in monthly_snapshots {
        assign_named_amount_entry_ids(
            &mut snapshot.assets,
            key,
            "asset",
            &explicit_id_owners,
            &reserved_explicit_ids,
            &mut used_entry_ids,
        );
        assign_named_amount_entry_ids(
            &mut snapshot.liabilities,
            key,
            "liability",
            &explicit_id_owners,
            &reserved_explicit_ids,
            &mut used_entry_ids,
        );
    }
}

fn collect_finance_entry_id_owners(
    daily_ledgers: &IndexMap<String, FinanceDayLedger>,
    monthly_snapshots: &IndexMap<String, FinanceMonthSnapshot>,
) -> HashMap<String, String> {
    let mut owners = HashMap::new();
    for (key, ledger) in daily_ledgers {
        collect_income_entry_id_owners(&ledger.incomes, key, &mut owners);
        collect_expense_entry_id_owners(&ledger.expenses, key, &mut owners);
    }
    for (key, snapshot) in monthly_snapshots {
        collect_named_amount_entry_id_owners(&snapshot.assets, key, "asset", &mut owners);
        collect_named_amount_entry_id_owners(&snapshot.liabilities, key, "liability", &mut owners);
    }
    owners
}

fn record_finance_entry_id_owner(owners: &mut HashMap<String, String>, id: &str, identity: String) {
    if id.is_empty() {
        return;
    }
    owners
        .entry(id.to_string())
        .and_modify(|owner| {
            if identity < *owner {
                *owner = identity.clone();
            }
        })
        .or_insert(identity);
}

fn collect_income_entry_id_owners(
    entries: &[FinanceIncomeEntry],
    container_key: &str,
    owners: &mut HashMap<String, String>,
) {
    let mut occurrences = IndexMap::<String, usize>::new();
    for entry in entries {
        let signature = income_entry_signature(entry);
        let occurrence = next_finance_entry_occurrence(
            &mut occurrences,
            &finance_entry_occurrence_key(&signature, &entry.id),
        );
        record_finance_entry_id_owner(
            owners,
            &entry.id,
            finance_entry_identity(container_key, "income", &signature, &entry.id, occurrence),
        );
    }
}

fn collect_expense_entry_id_owners(
    entries: &[FinanceExpenseEntry],
    container_key: &str,
    owners: &mut HashMap<String, String>,
) {
    let mut occurrences = IndexMap::<String, usize>::new();
    for entry in entries {
        let signature = expense_entry_signature(entry);
        let occurrence = next_finance_entry_occurrence(
            &mut occurrences,
            &finance_entry_occurrence_key(&signature, &entry.id),
        );
        record_finance_entry_id_owner(
            owners,
            &entry.id,
            finance_entry_identity(container_key, "expense", &signature, &entry.id, occurrence),
        );
    }
}

fn collect_named_amount_entry_id_owners(
    entries: &[FinanceNamedAmountEntry],
    container_key: &str,
    lane: &str,
    owners: &mut HashMap<String, String>,
) {
    let mut occurrences = IndexMap::<String, usize>::new();
    for entry in entries {
        let signature = named_amount_entry_signature(entry);
        let occurrence = next_finance_entry_occurrence(
            &mut occurrences,
            &finance_entry_occurrence_key(&signature, &entry.id),
        );
        record_finance_entry_id_owner(
            owners,
            &entry.id,
            finance_entry_identity(container_key, lane, &signature, &entry.id, occurrence),
        );
    }
}

fn assign_income_entry_ids(
    entries: &mut [FinanceIncomeEntry],
    container_key: &str,
    explicit_id_owners: &HashMap<String, String>,
    reserved_explicit_ids: &HashSet<String>,
    used_entry_ids: &mut HashSet<String>,
) {
    let mut occurrences = IndexMap::<String, usize>::new();
    for entry in entries {
        let signature = income_entry_signature(entry);
        let original_id = entry.id.clone();
        let occurrence = next_finance_entry_occurrence(
            &mut occurrences,
            &finance_entry_occurrence_key(&signature, &original_id),
        );
        ensure_unique_finance_entry_id(
            &mut entry.id,
            container_key,
            "income",
            &signature,
            &original_id,
            occurrence,
            explicit_id_owners,
            reserved_explicit_ids,
            used_entry_ids,
        );
    }
}

fn assign_expense_entry_ids(
    entries: &mut [FinanceExpenseEntry],
    container_key: &str,
    explicit_id_owners: &HashMap<String, String>,
    reserved_explicit_ids: &HashSet<String>,
    used_entry_ids: &mut HashSet<String>,
) {
    let mut occurrences = IndexMap::<String, usize>::new();
    for entry in entries {
        let signature = expense_entry_signature(entry);
        let original_id = entry.id.clone();
        let occurrence = next_finance_entry_occurrence(
            &mut occurrences,
            &finance_entry_occurrence_key(&signature, &original_id),
        );
        ensure_unique_finance_entry_id(
            &mut entry.id,
            container_key,
            "expense",
            &signature,
            &original_id,
            occurrence,
            explicit_id_owners,
            reserved_explicit_ids,
            used_entry_ids,
        );
    }
}

fn assign_named_amount_entry_ids(
    entries: &mut [FinanceNamedAmountEntry],
    container_key: &str,
    lane: &str,
    explicit_id_owners: &HashMap<String, String>,
    reserved_explicit_ids: &HashSet<String>,
    used_entry_ids: &mut HashSet<String>,
) {
    let mut occurrences = IndexMap::<String, usize>::new();
    for entry in entries {
        let signature = named_amount_entry_signature(entry);
        let original_id = entry.id.clone();
        let occurrence = next_finance_entry_occurrence(
            &mut occurrences,
            &finance_entry_occurrence_key(&signature, &original_id),
        );
        ensure_unique_finance_entry_id(
            &mut entry.id,
            container_key,
            lane,
            &signature,
            &original_id,
            occurrence,
            explicit_id_owners,
            reserved_explicit_ids,
            used_entry_ids,
        );
    }
}

fn income_entry_signature(entry: &FinanceIncomeEntry) -> String {
    serde_json::to_string(&(
        entry.name.as_str(),
        income_kind_code(&entry.kind),
        entry.amount,
        entry.note.as_str(),
        entry.updated_at_epoch_millis,
        entry.deleted_at_epoch_millis,
    ))
    .unwrap_or_default()
}

fn expense_entry_signature(entry: &FinanceExpenseEntry) -> String {
    serde_json::to_string(&(
        entry.name.as_str(),
        bucket_code(&entry.bucket),
        entry.amount,
        entry.note.as_str(),
        entry.updated_at_epoch_millis,
        entry.deleted_at_epoch_millis,
    ))
    .unwrap_or_default()
}

fn named_amount_entry_signature(entry: &FinanceNamedAmountEntry) -> String {
    serde_json::to_string(&(
        entry.name.as_str(),
        named_amount_kind_code(&entry.kind),
        entry.amount,
        entry.updated_at_epoch_millis,
        entry.deleted_at_epoch_millis,
    ))
    .unwrap_or_default()
}

fn finance_entry_occurrence_key(signature: &str, original_id: &str) -> String {
    serde_json::to_string(&(signature, original_id)).unwrap_or_default()
}

fn finance_entry_identity(
    container_key: &str,
    lane: &str,
    signature: &str,
    original_id: &str,
    occurrence: usize,
) -> String {
    serde_json::to_string(&(container_key, lane, signature, original_id, occurrence))
        .unwrap_or_default()
}

fn next_finance_entry_occurrence(
    occurrences: &mut IndexMap<String, usize>,
    signature: &str,
) -> usize {
    let occurrence = occurrences.entry(signature.to_string()).or_insert(0);
    let current = *occurrence;
    *occurrence += 1;
    current
}

fn ensure_unique_finance_entry_id(
    id: &mut String,
    container_key: &str,
    lane: &str,
    signature: &str,
    original_id: &str,
    occurrence: usize,
    explicit_id_owners: &HashMap<String, String>,
    reserved_explicit_ids: &HashSet<String>,
    used_entry_ids: &mut HashSet<String>,
) {
    let identity = finance_entry_identity(container_key, lane, signature, original_id, occurrence);
    if !original_id.is_empty()
        && explicit_id_owners.get(original_id) == Some(&identity)
        && used_entry_ids.insert(original_id.to_string())
    {
        *id = original_id.to_string();
        return;
    }

    let mut collision = 0usize;
    loop {
        let generated = deterministic_finance_entry_id(
            container_key,
            lane,
            signature,
            original_id,
            occurrence,
            collision,
        );
        if !reserved_explicit_ids.contains(&generated) && used_entry_ids.insert(generated.clone()) {
            *id = generated;
            return;
        }
        collision += 1;
    }
}

fn deterministic_finance_entry_id(
    container_key: &str,
    lane: &str,
    signature: &str,
    original_id: &str,
    occurrence: usize,
    collision: usize,
) -> String {
    let source = format!(
        "{container_key}\u{1f}{lane}\u{1f}{signature}\u{1f}{original_id}\u{1f}{occurrence}\u{1f}{collision}"
    );
    let first = stable_finance_hash(0xcbf29ce484222325, source.as_bytes());
    let second = stable_finance_hash(0x84222325cbf29ce4, source.as_bytes());
    format!("fin-legacy-{first:016x}{second:016x}")
}

fn stable_finance_hash(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn deserialize_nullable_finance_millis<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<i64>::deserialize(deserializer)?.unwrap_or(0))
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
            id: String::new(),
            updated_at_epoch_millis: 0,
            deleted_at_epoch_millis: 0,
            name: "\u{503a}\u{52a1}\u{538b}\u{964d}".to_string(),
            bucket: FinanceExpenseBucket::Debt,
            amount: 0,
            amount_minor: None,
            note: String::new(),
        },
        FinanceExpenseEntry {
            id: String::new(),
            updated_at_epoch_millis: 0,
            deleted_at_epoch_millis: 0,
            name: "\u{65e5}\u{5e38}\u{6d88}\u{8d39}".to_string(),
            bucket: FinanceExpenseBucket::Food,
            amount: 0,
            amount_minor: None,
            note: String::new(),
        },
        FinanceExpenseEntry {
            id: String::new(),
            updated_at_epoch_millis: 0,
            deleted_at_epoch_millis: 0,
            name: "\u{56fa}\u{5b9a}\u{652f}\u{51fa}".to_string(),
            bucket: FinanceExpenseBucket::Living,
            amount: 0,
            amount_minor: None,
            note: String::new(),
        },
        FinanceExpenseEntry {
            id: String::new(),
            updated_at_epoch_millis: 0,
            deleted_at_epoch_millis: 0,
            name: "\u{957f}\u{671f}\u{914d}\u{7f6e}".to_string(),
            bucket: FinanceExpenseBucket::Btc,
            amount: 0,
            amount_minor: None,
            note: String::new(),
        },
        FinanceExpenseEntry {
            id: String::new(),
            updated_at_epoch_millis: 0,
            deleted_at_epoch_millis: 0,
            name: "\u{6210}\u{957f}\u{6295}\u{5165}".to_string(),
            bucket: FinanceExpenseBucket::Learning,
            amount: 0,
            amount_minor: None,
            note: String::new(),
        },
    ]
}

fn default_finance_income_entries() -> Vec<FinanceIncomeEntry> {
    vec![
        FinanceIncomeEntry {
            id: String::new(),
            updated_at_epoch_millis: 0,
            deleted_at_epoch_millis: 0,
            name: "\u{5de5}\u{8d44}\u{6536}\u{5165}".to_string(),
            kind: FinanceIncomeKind::Active,
            amount: 0,
            amount_minor: None,
            note: String::new(),
        },
        FinanceIncomeEntry {
            id: String::new(),
            updated_at_epoch_millis: 0,
            deleted_at_epoch_millis: 0,
            name: "\u{526f}\u{4e1a}\u{6536}\u{5165}".to_string(),
            kind: FinanceIncomeKind::Active,
            amount: 0,
            amount_minor: None,
            note: String::new(),
        },
        FinanceIncomeEntry {
            id: String::new(),
            updated_at_epoch_millis: 0,
            deleted_at_epoch_millis: 0,
            name: "\u{88ab}\u{52a8}\u{6536}\u{5165}".to_string(),
            kind: FinanceIncomeKind::Asset,
            amount: 0,
            amount_minor: None,
            note: String::new(),
        },
        FinanceIncomeEntry {
            id: String::new(),
            updated_at_epoch_millis: 0,
            deleted_at_epoch_millis: 0,
            name: "\u{5176}\u{4ed6}\u{6536}\u{5165}".to_string(),
            kind: FinanceIncomeKind::Other,
            amount: 0,
            amount_minor: None,
            note: String::new(),
        },
    ]
}

fn default_finance_asset_entries() -> Vec<FinanceNamedAmountEntry> {
    vec![
        FinanceNamedAmountEntry {
            id: String::new(),
            updated_at_epoch_millis: 0,
            deleted_at_epoch_millis: 0,
            name: "\u{73b0}\u{91d1}\u{4e0e}\u{6d3b}\u{671f}\u{5b58}\u{6b3e}".to_string(),
            kind: FinanceNamedAmountKind::CashReserve,
            amount: 0,
            amount_minor: None,
        },
        FinanceNamedAmountEntry {
            id: String::new(),
            updated_at_epoch_millis: 0,
            deleted_at_epoch_millis: 0,
            name: "\u{80a1}\u{7968} / \u{57fa}\u{91d1} / \u{52a0}\u{5bc6}\u{8d44}\u{4ea7}"
                .to_string(),
            kind: FinanceNamedAmountKind::ProductiveAsset,
            amount: 0,
            amount_minor: None,
        },
        FinanceNamedAmountEntry {
            id: String::new(),
            updated_at_epoch_millis: 0,
            deleted_at_epoch_millis: 0,
            name: "\u{7ecf}\u{8425}\u{6027}\u{8d44}\u{4ea7}\u{51c0}\u{503c}".to_string(),
            kind: FinanceNamedAmountKind::ProductiveAsset,
            amount: 0,
            amount_minor: None,
        },
        FinanceNamedAmountEntry {
            id: String::new(),
            updated_at_epoch_millis: 0,
            deleted_at_epoch_millis: 0,
            name: "\u{5176}\u{4ed6}\u{8d44}\u{4ea7}".to_string(),
            kind: FinanceNamedAmountKind::OtherAsset,
            amount: 0,
            amount_minor: None,
        },
    ]
}

fn default_finance_liability_entries() -> Vec<FinanceNamedAmountEntry> {
    vec![
        FinanceNamedAmountEntry {
            id: String::new(),
            updated_at_epoch_millis: 0,
            deleted_at_epoch_millis: 0,
            name: "\u{623f}\u{8d37} / \u{79df}\u{8d41}\u{8d1f}\u{503a}".to_string(),
            kind: FinanceNamedAmountKind::LiabilityBalance,
            amount: 0,
            amount_minor: None,
        },
        FinanceNamedAmountEntry {
            id: String::new(),
            updated_at_epoch_millis: 0,
            deleted_at_epoch_millis: 0,
            name: "\u{4fe1}\u{7528}\u{5361}\u{6b20}\u{6b3e}".to_string(),
            kind: FinanceNamedAmountKind::LiabilityBalance,
            amount: 0,
            amount_minor: None,
        },
        FinanceNamedAmountEntry {
            id: String::new(),
            updated_at_epoch_millis: 0,
            deleted_at_epoch_millis: 0,
            name: "\u{6d88}\u{8d39}\u{8d37}\u{6b3e} / \u{5206}\u{671f}\u{4ed8}\u{6b3e}".to_string(),
            kind: FinanceNamedAmountKind::LiabilityBalance,
            amount: 0,
            amount_minor: None,
        },
        FinanceNamedAmountEntry {
            id: String::new(),
            updated_at_epoch_millis: 0,
            deleted_at_epoch_millis: 0,
            name: "\u{4e2a}\u{4eba}\u{503a}\u{52a1}".to_string(),
            kind: FinanceNamedAmountKind::OtherLiability,
            amount: 0,
            amount_minor: None,
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

fn compact_whitespace(value: &str) -> String {
    value
        .split_whitespace()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Clone, Debug)]
struct RecurringExpenseAccumulator {
    name: String,
    day_amounts: Vec<(i64, i64)>,
}

#[derive(Clone, Debug, Default)]
struct PendingMonthlyExpenseHistory {
    seen_in_current_month: bool,
    previous_months: [Vec<(i32, i64)>; 3],
}

fn finance_budget_envelopes(
    aggregate: FinanceLedgerTotals,
    settings: &FinanceSettings,
) -> Vec<FinanceBudgetEnvelope> {
    let income = aggregate.income_total();
    primary_bucket_order()
        .into_iter()
        .map(|bucket| {
            let target = settings.target_for(&bucket);
            let target_amount = target
                .map(|share| scale_finance_amount(income, share as f64))
                .unwrap_or(0);
            let actual_amount = aggregate.bucket_total(&bucket);
            let remaining_amount = target_amount.saturating_sub(actual_amount);
            let status_code = match target {
                None => BUDGET_STATUS_NO_TARGET,
                Some(_) if is_minimum_allocation_bucket(&bucket) => {
                    if actual_amount < target_amount {
                        BUDGET_STATUS_BELOW_FLOOR
                    } else {
                        BUDGET_STATUS_NORMAL
                    }
                }
                Some(_) if actual_amount > target_amount => BUDGET_STATUS_OVER_CAP,
                Some(_)
                    if target_amount > 0 && actual_amount as f64 / target_amount as f64 >= 0.80 =>
                {
                    BUDGET_STATUS_NEAR_CAP
                }
                Some(_) => BUDGET_STATUS_NORMAL,
            };
            FinanceBudgetEnvelope {
                bucket_code: bucket_code(&bucket),
                label: settings.label_for(&bucket),
                target_amount,
                actual_amount,
                remaining_amount,
                status_code,
            }
        })
        .collect()
}

fn finance_pending_recurring_reserve(
    pending_by_bucket: &[i64; 6],
    budget_envelopes: &[FinanceBudgetEnvelope],
) -> i64 {
    pending_by_bucket
        .iter()
        .enumerate()
        .map(|(bucket_index, pending_amount)| {
            let pending_amount = (*pending_amount).max(0);
            let bucket_code = bucket_index as i32;
            let protected_overlap = bucket_from_code(bucket_code)
                .filter(is_minimum_allocation_bucket)
                .and_then(|_| {
                    budget_envelopes
                        .iter()
                        .find(|envelope| envelope.bucket_code == bucket_code)
                })
                .map(|envelope| envelope.remaining_amount.max(0))
                .unwrap_or(0)
                .min(pending_amount);
            pending_amount.saturating_sub(protected_overlap)
        })
        .fold(0_i64, i64::saturating_add)
}

fn is_minimum_allocation_bucket(bucket: &FinanceExpenseBucket) -> bool {
    matches!(
        bucket,
        FinanceExpenseBucket::Btc | FinanceExpenseBucket::Learning | FinanceExpenseBucket::Other
    )
}

fn scale_elapsed_amount_to_30_days(amount: i64, elapsed_days: i32) -> i64 {
    if elapsed_days <= 0 {
        0
    } else {
        scale_finance_amount(amount, 30.0 / elapsed_days as f64)
    }
}

fn scale_finance_amount(amount: i64, factor: f64) -> i64 {
    if amount <= 0 || !factor.is_finite() || factor <= 0.0 {
        return 0;
    }
    let scaled = amount as f64 * factor;
    if scaled >= i64::MAX as f64 {
        i64::MAX
    } else {
        scaled.round() as i64
    }
}

fn recurring_candidate_from_accumulator(
    mut accumulator: RecurringExpenseAccumulator,
) -> Option<FinanceRecurringCandidate> {
    accumulator.day_amounts.sort_by_key(|entry| entry.0);
    if accumulator.day_amounts.len() < 2 {
        return None;
    }
    let occurrences = accumulator.day_amounts.len() as i32;
    let total = accumulator
        .day_amounts
        .iter()
        .map(|entry| entry.1)
        .fold(0_i64, i64::saturating_add);
    let average_amount = scale_finance_amount(total, 1.0 / occurrences as f64);
    let first_day = accumulator.day_amounts.first()?.0;
    let last_day = accumulator.day_amounts.last()?.0;
    let span_days = last_day.saturating_sub(first_day);
    if span_days <= 0 {
        return None;
    }
    let average_gap_days = span_days as f64 / f64::from(occurrences - 1);
    let monthly_frequency = (30.0 / average_gap_days).clamp(1.0 / 3.0, 30.0);
    let monthly_estimate = scale_finance_amount(average_amount, monthly_frequency);
    Some(FinanceRecurringCandidate {
        name: accumulator.name,
        occurrences,
        average_amount,
        monthly_estimate,
    })
}

#[allow(clippy::too_many_arguments)]
fn finance_risk_actions(
    uses_detailed_cashflow: bool,
    forecast_available: bool,
    balance_sheet_available: bool,
    has_fresh_snapshot: bool,
    aggregate: FinanceLedgerTotals,
    projected_monthly_net: i64,
    stressed_monthly_net: i64,
    emergency_gap: i64,
    emergency_months: Option<f32>,
    debt_balance: i64,
    monthly_debt_payment: i64,
    debt_free_months: Option<f32>,
    budget_envelopes: &[FinanceBudgetEnvelope],
    recurring_monthly_estimate: i64,
    anomaly_ratio: f32,
    anomaly_amount: i64,
) -> Vec<FinanceRiskAction> {
    let mut candidates = Vec::<(i32, FinanceRiskAction)>::new();
    let has_cashflow_evidence =
        uses_detailed_cashflow || aggregate.income_total() > 0 || aggregate.expense_total() > 0;
    if !forecast_available {
        candidates.push((
            1,
            FinanceRiskAction {
                severity_code: if has_cashflow_evidence {
                    ACTION_SEVERITY_WATCH
                } else {
                    ACTION_SEVERITY_URGENT
                },
                kind_code: ACTION_KIND_COMPLETE_DATA,
                bucket_code: BUCKET_CODE_NONE,
                amount: 0,
            },
        ));
    }

    let observed_negative = aggregate.net_cashflow() < 0;
    if projected_monthly_net < 0 || observed_negative {
        let deficit = if projected_monthly_net < 0 {
            projected_monthly_net.saturating_abs()
        } else {
            aggregate.net_cashflow().saturating_abs()
        };
        candidates.push((
            0,
            FinanceRiskAction {
                severity_code: ACTION_SEVERITY_URGENT,
                kind_code: ACTION_KIND_STOP_NEGATIVE_CASHFLOW,
                bucket_code: BUCKET_CODE_NONE,
                amount: deficit,
            },
        ));
    } else if forecast_available && stressed_monthly_net < 0 {
        candidates.push((
            0,
            FinanceRiskAction {
                severity_code: ACTION_SEVERITY_WATCH,
                kind_code: ACTION_KIND_STOP_NEGATIVE_CASHFLOW,
                bucket_code: BUCKET_CODE_NONE,
                amount: stressed_monthly_net.saturating_abs(),
            },
        ));
    }

    if !has_fresh_snapshot {
        candidates.push((
            2,
            FinanceRiskAction {
                severity_code: if balance_sheet_available {
                    ACTION_SEVERITY_URGENT
                } else {
                    ACTION_SEVERITY_WATCH
                },
                kind_code: ACTION_KIND_REFRESH_MONTH_SNAPSHOT,
                bucket_code: BUCKET_CODE_NONE,
                amount: 0,
            },
        ));
    }
    if has_fresh_snapshot && emergency_gap > 0 {
        candidates.push((
            3,
            FinanceRiskAction {
                severity_code: if emergency_months.is_some_and(|months| months < 1.0) {
                    ACTION_SEVERITY_URGENT
                } else {
                    ACTION_SEVERITY_WATCH
                },
                kind_code: ACTION_KIND_BUILD_EMERGENCY_FUND,
                bucket_code: BUCKET_CODE_NONE,
                amount: emergency_gap,
            },
        ));
    }

    if let Some(envelope) = budget_envelopes
        .iter()
        .filter(|envelope| envelope.status_code == BUDGET_STATUS_OVER_CAP)
        .max_by_key(|envelope| envelope.remaining_amount.saturating_abs())
    {
        let overspend = envelope.remaining_amount.saturating_abs();
        candidates.push((
            4,
            FinanceRiskAction {
                severity_code: if envelope.target_amount <= 0
                    || overspend.saturating_mul(5) >= envelope.target_amount
                {
                    ACTION_SEVERITY_URGENT
                } else {
                    ACTION_SEVERITY_WATCH
                },
                kind_code: ACTION_KIND_REDUCE_OVER_BUDGET,
                bucket_code: envelope.bucket_code,
                amount: overspend,
            },
        ));
    }
    if let Some(envelope) = budget_envelopes
        .iter()
        .filter(|envelope| {
            envelope.status_code == BUDGET_STATUS_BELOW_FLOOR && envelope.remaining_amount > 0
        })
        .max_by_key(|envelope| envelope.remaining_amount)
    {
        candidates.push((
            5,
            FinanceRiskAction {
                severity_code: ACTION_SEVERITY_WATCH,
                kind_code: ACTION_KIND_FILL_LONG_TERM_ALLOCATION,
                bucket_code: envelope.bucket_code,
                amount: envelope.remaining_amount,
            },
        ));
    }
    if has_fresh_snapshot && debt_balance > 0 {
        let debt_action_severity = if monthly_debt_payment <= 0 {
            Some(ACTION_SEVERITY_URGENT)
        } else if debt_free_months.is_some_and(|months| months > 60.0) {
            Some(ACTION_SEVERITY_WATCH)
        } else {
            None
        };
        if let Some(severity_code) = debt_action_severity {
            candidates.push((
                6,
                FinanceRiskAction {
                    severity_code,
                    kind_code: ACTION_KIND_ACCELERATE_DEBT,
                    bucket_code: BUCKET_CODE_DEBT,
                    amount: debt_balance,
                },
            ));
        }
    }
    if anomaly_ratio >= 2.0 && anomaly_amount > 0 {
        candidates.push((
            7,
            FinanceRiskAction {
                severity_code: if anomaly_ratio >= 3.0 {
                    ACTION_SEVERITY_URGENT
                } else {
                    ACTION_SEVERITY_WATCH
                },
                kind_code: ACTION_KIND_REVIEW_ANOMALY_DAY,
                bucket_code: BUCKET_CODE_NONE,
                amount: anomaly_amount,
            },
        ));
    }
    if recurring_monthly_estimate > 0 {
        candidates.push((
            8,
            FinanceRiskAction {
                severity_code: ACTION_SEVERITY_WATCH,
                kind_code: ACTION_KIND_REVIEW_RECURRING_EXPENSE,
                bucket_code: BUCKET_CODE_NONE,
                amount: recurring_monthly_estimate,
            },
        ));
    }

    candidates.sort_by(|(left_priority, left), (right_priority, right)| {
        right
            .severity_code
            .cmp(&left.severity_code)
            .then_with(|| left_priority.cmp(right_priority))
            .then_with(|| left.kind_code.cmp(&right.kind_code))
    });
    let mut actions = candidates
        .into_iter()
        .map(|(_, action)| action)
        .take(3)
        .collect::<Vec<_>>();
    if actions.is_empty() {
        actions.push(FinanceRiskAction {
            severity_code: ACTION_SEVERITY_INFO,
            kind_code: if forecast_available && stressed_monthly_net >= 0 {
                ACTION_KIND_STRESS_TEST_PASSED
            } else {
                ACTION_KIND_CURRENTLY_STABLE
            },
            bucket_code: BUCKET_CODE_NONE,
            amount: if forecast_available {
                stressed_monthly_net.max(0)
            } else {
                0
            },
        });
    }
    actions
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
    if !is_valid_ymd(
        reference_year.max(0) as u32,
        reference_month.max(0) as u32,
        reference_day.max(0) as u32,
    ) || days_from_civil(year as i64, month as i64, day as i64)
        > days_from_civil(
            reference_year as i64,
            reference_month as i64,
            reference_day as i64,
        )
    {
        return false;
    }

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

fn comparable_period_reference_dates(
    period_code: i32,
    year: i32,
    month: i32,
    day: i32,
) -> ((i32, i32, i32), (i32, i32, i32)) {
    if !is_valid_ymd(year.max(0) as u32, month.max(0) as u32, day.max(0) as u32) {
        return ((year, month, day), (year, month, day));
    }

    let current_start = match period_code {
        PERIOD_DAY => (year, month, day),
        PERIOD_MONTH => (year, month, 1),
        PERIOD_QUARTER => (year, ((month - 1) / 3) * 3 + 1, 1),
        PERIOD_YEAR => (year, 1, 1),
        _ => return ((year, month, day), (year, month, day)),
    };
    let previous_start = match period_code {
        PERIOD_DAY => add_days(year, month, day, -1),
        PERIOD_MONTH => add_months(current_start.0, current_start.1, 1, -1),
        PERIOD_QUARTER => add_months(current_start.0, current_start.1, 1, -3),
        PERIOD_YEAR => (year - 1, 1, 1),
        _ => current_start,
    };
    let previous_end = add_days(current_start.0, current_start.1, current_start.2, -1);
    let current_elapsed = days_from_civil(year as i64, month as i64, day as i64)
        - days_from_civil(
            current_start.0 as i64,
            current_start.1 as i64,
            current_start.2 as i64,
        )
        + 1;
    let previous_length = days_from_civil(
        previous_end.0 as i64,
        previous_end.1 as i64,
        previous_end.2 as i64,
    ) - days_from_civil(
        previous_start.0 as i64,
        previous_start.1 as i64,
        previous_start.2 as i64,
    ) + 1;
    let comparable_days = current_elapsed.min(previous_length).max(1);
    (
        add_days(
            current_start.0,
            current_start.1,
            current_start.2,
            comparable_days - 1,
        ),
        add_days(
            previous_start.0,
            previous_start.1,
            previous_start.2,
            comparable_days - 1,
        ),
    )
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

fn trend_samples_are_comparable(
    current_recorded_days: i64,
    current_expected_days: i32,
    previous_recorded_days: i64,
    previous_expected_days: i32,
) -> bool {
    if current_expected_days <= 0
        || previous_expected_days <= 0
        || current_recorded_days <= 0
        || previous_recorded_days <= 0
    {
        return false;
    }
    let current_coverage =
        (current_recorded_days as f32 / current_expected_days as f32).clamp(0.0, 1.0);
    let previous_coverage =
        (previous_recorded_days as f32 / previous_expected_days as f32).clamp(0.0, 1.0);
    current_coverage >= 0.60
        && previous_coverage >= 0.60
        && (current_coverage - previous_coverage).abs() <= 0.20
}

fn defensive_coverage_in_months(
    coverage_for_elapsed_period: Option<f32>,
    elapsed_days: i32,
) -> Option<f32> {
    const AVERAGE_DAYS_PER_MONTH: f32 = 365.2425 / 12.0;
    coverage_for_elapsed_period.map(|coverage| {
        if !coverage.is_finite() {
            coverage
        } else {
            coverage.max(0.0) * elapsed_days.max(1) as f32 / AVERAGE_DAYS_PER_MONTH
        }
    })
}

fn snapshot_values_from_totals(
    aggregate: FinanceLedgerTotals,
    summary: FinanceMonthSnapshotTotals,
    recorded_days: i64,
) -> FinanceSnapshotValues {
    let essential_outflow = aggregate.essential_outflow_total();
    let freedom_gap = essential_outflow
        .saturating_sub(aggregate.asset_income_total)
        .max(0);
    let defensive_base = essential_outflow.saturating_sub(aggregate.asset_income_total);
    let defensive_coverage = if defensive_base <= 0 {
        None
    } else if summary.cash_reserve_total <= 0 {
        Some(0.0)
    } else {
        Some(summary.cash_reserve_total as f32 / defensive_base as f32)
    };

    FinanceSnapshotValues {
        total_income: aggregate.income_total(),
        total_outflow: aggregate.expense_total(),
        net_cashflow: aggregate.net_cashflow(),
        freedom_gap,
        passive_coverage_ratio: positive_numerator_ratio(
            aggregate.asset_income_total,
            essential_outflow,
        ),
        wage_dependence_ratio: safe_finance_ratio(
            aggregate.active_income_total,
            aggregate.income_total(),
        ),
        liability_pressure_ratio: positive_numerator_ratio(
            aggregate.debt_total,
            aggregate.income_total(),
        ),
        net_worth: summary.net_worth(),
        defensive_coverage,
        asset_yield_ratio: positive_numerator_ratio(
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

fn positive_numerator_ratio(numerator: i64, denominator: i64) -> f32 {
    if numerator <= 0 {
        0.0
    } else if denominator <= 0 {
        1.0
    } else {
        numerator as f32 / denominator as f32
    }
}

fn normalized_finance_score(value: f32, floor: f32, ceiling: f32) -> f32 {
    if !value.is_finite() || ceiling <= floor {
        return 0.0;
    }
    ((value - floor) / (ceiling - floor)).clamp(0.0, 1.0)
}

fn balance_sheet_liability_score(summary: FinanceMonthSnapshotTotals) -> f32 {
    if summary.liability_total <= 0 {
        return 1.0;
    }
    if summary.asset_total <= 0 || summary.net_worth() < 0 {
        return 0.0;
    }
    let leverage = summary.liability_total as f64 / summary.asset_total as f64;
    (1.0 - leverage.clamp(0.0, 1.0)) as f32
}

fn finance_momentum_score(aggregate: FinanceLedgerTotals, trend: FinanceTrendValues) -> f32 {
    if aggregate.days_with_entries <= 0 || !trend.cashflow_comparison_available {
        return 0.72;
    }
    if aggregate.net_cashflow() < 0 && trend.outflow_delta > 0 {
        0.15
    } else if trend.net_cashflow_delta < 0 && trend.outflow_delta > 0 {
        0.35
    } else if trend.net_cashflow_delta < 0 {
        0.60
    } else if trend.outflow_delta <= 0 || trend.income_delta >= 0 {
        1.0
    } else {
        0.82
    }
}

fn finance_risk_level_code(
    score: i32,
    warning_count: i32,
    aggregate: FinanceLedgerTotals,
    snapshot: FinanceSnapshotValues,
    defensive_coverage_months: Option<f32>,
) -> i32 {
    let defensive_coverage = defensive_coverage_months.unwrap_or(f32::INFINITY);
    if snapshot.net_worth < 0
        || score < 42
        || (aggregate.net_cashflow() < 0
            && (defensive_coverage < 1.0 || snapshot.liability_pressure_ratio > 0.45))
    {
        RISK_LEVEL_CRITICAL
    } else if score < 56 || warning_count >= 2 {
        RISK_LEVEL_TIGHT
    } else if score < 72 || warning_count > 0 {
        RISK_LEVEL_WATCH
    } else {
        RISK_LEVEL_STEADY
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

fn month_index(year: i32, month: i32) -> i32 {
    year.saturating_mul(12).saturating_add(month - 1)
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

fn scale_monthly_finance_amount(amount: i64, period_code: i32) -> i64 {
    let safe = amount.max(0);
    match period_code {
        PERIOD_DAY => ((safe as f64) / 30.0).round() as i64,
        PERIOD_QUARTER => safe.saturating_mul(3),
        PERIOD_YEAR => safe.saturating_mul(12),
        _ => safe,
    }
}

fn format_model_ratio_percent(value: f32) -> String {
    format!("{:.0}%", value * 100.0)
}

fn target_overcommit_discipline_score(overcommit: f32) -> f32 {
    if overcommit <= TARGET_TOTAL_EPSILON {
        1.0
    } else {
        (1.0 - overcommit / TARGET_DISCIPLINE_PENALTY_SPAN).clamp(0.0, TARGET_OVERCOMMIT_SCORE_CAP)
    }
}

fn is_target_discipline_violation(bucket: &FinanceExpenseBucket, drift: f32) -> bool {
    match bucket {
        FinanceExpenseBucket::Debt | FinanceExpenseBucket::Food | FinanceExpenseBucket::Living => {
            drift > 0.0
        }
        FinanceExpenseBucket::Btc
        | FinanceExpenseBucket::Learning
        | FinanceExpenseBucket::Other => drift < 0.0,
    }
}

fn is_problematic_target_drift(bucket: &FinanceExpenseBucket, drift: f32) -> bool {
    is_target_discipline_violation(bucket, drift) && drift.abs() > 0.08
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
    use serde_json::json;

    #[test]
    fn cent_only_rows_survive_backup_upsert_and_exact_calculation() {
        let raw = json!({
            "dailyLedgers": {"2026-10-02": {
                "incomes": [{"id":"i1","amount":0,"amountMinor":1}, {"id":"i2","amount":0,"amountMinor":1}],
                "expenses": [{"id":"e1","amount":58,"amountMinor":5812}]
            }},
            "monthlySnapshots": {"2026-10": {
                "assets": [{"id":"a1","amount":0,"amountMinor":2}],
                "liabilities": [{"id":"l1","kind":"LIABILITY_BALANCE","amount":0,"amountMinor":1}]
            }},
            "cashReserve":1,"legacyAmountMinor":{"cashReserve":123}
        }).to_string();
        let sanitized = sanitize_finance_profile_json(&raw).unwrap();
        let backup = encode_finance_backup_json(&sanitized, "test", 1).unwrap();
        let restored = decode_finance_backup_profile_json(&backup).unwrap();
        let original: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert_eq!(
            2,
            original["dailyLedgers"]["2026-10-02"]["incomes"]
                .as_array()
                .unwrap()
                .len()
        );
        assert_eq!(
            0,
            original["dailyLedgers"]["2026-10-02"]["incomes"][0]["amount"]
        );
        assert_eq!(
            1,
            original["dailyLedgers"]["2026-10-02"]["incomes"][0]["amountMinor"]
        );
        assert_eq!(
            "l1",
            original["monthlySnapshots"]["2026-10"]["liabilities"][0]["id"]
        );
        assert_eq!(
            1,
            original["monthlySnapshots"]["2026-10"]["liabilities"][0]["amountMinor"]
        );
        let projected = crate::finance_money::project_profile_json(&restored).unwrap();
        let calculated: FinanceProfile = serde_json::from_str(&projected).unwrap();
        let totals = calculated.daily_ledgers["2026-10-02"].to_aggregate_totals();
        assert_eq!(2, totals.income_total());
        assert_eq!(5812, totals.expense_total());
        let balance = calculated.monthly_snapshots["2026-10"].to_summary_totals();
        assert_eq!(2, balance.asset_total);
        assert_eq!(1, balance.liability_total);
        assert_eq!(123, calculated.cash_reserve);
        assert!(sanitize_finance_profile_json(&projected).is_none());
        assert!(encode_finance_backup_json(&projected, "test", 1).is_none());
        let zero_cent_row = json!({"assets":[{"id":"a","amount":0,"amountMinor":1}]}).to_string();
        let updated = upsert_finance_month_snapshot_json("{}", "2026-10", &zero_cent_row).unwrap();
        assert!(updated.contains("\"amountMinor\":1"));
    }

    #[test]
    fn sanitizer_preserves_amounts_and_focus_text_byte_for_byte() {
        let profile = FinanceProfile {
            active_income_monthly: i64::MAX,
            asset_income_monthly: -1,
            acquisition_focus: "  Dividend     snowball  ".to_string(),
            liability_focus: "  Loan     trim  ".to_string(),
            ..empty_profile()
        }
        .sanitized();

        assert_eq!(i64::MAX, profile.active_income_monthly);
        assert_eq!(-1, profile.asset_income_monthly);
        assert_eq!("  Dividend     snowball  ", profile.acquisition_focus);
        assert_eq!("  Loan     trim  ", profile.liability_focus);
    }

    #[test]
    fn sanitizer_preserves_finance_history_and_all_valid_ledger_lines() {
        let income_lines = (0..30)
            .map(|index| FinanceIncomeEntry {
                name: format!("income-{index}"),
                amount: index + 1,
                ..default_income_entry()
            })
            .collect::<Vec<_>>();
        let mut daily_ledgers = IndexMap::new();
        for year in 2020..=2025 {
            for month in 1..=12 {
                for day in 1..=days_in_month(year, month) {
                    daily_ledgers.insert(
                        format!("{year:04}-{month:02}-{day:02}"),
                        FinanceDayLedger {
                            incomes: income_lines.clone(),
                            ..FinanceDayLedger::default()
                        },
                    );
                }
            }
        }
        let expected_days = daily_ledgers.len();

        let mut monthly_snapshots = IndexMap::new();
        for year in 2020..=2045 {
            for month in 1..=12 {
                monthly_snapshots.insert(
                    format!("{year:04}-{month:02}"),
                    FinanceMonthSnapshot {
                        assets: (0..30)
                            .map(|index| FinanceNamedAmountEntry {
                                id: String::new(),
                                updated_at_epoch_millis: 0,
                                deleted_at_epoch_millis: 0,
                                name: format!("asset-{index}"),
                                kind: FinanceNamedAmountKind::OtherAsset,
                                amount: index + 1,
                                amount_minor: None,
                            })
                            .collect(),
                        ..FinanceMonthSnapshot::default()
                    },
                );
            }
        }
        let expected_months = monthly_snapshots.len();

        let sanitized = FinanceProfile {
            daily_ledgers,
            monthly_snapshots,
            ..empty_profile()
        }
        .sanitized();

        assert_eq!(expected_days, sanitized.daily_ledgers.len());
        assert_eq!(expected_months, sanitized.monthly_snapshots.len());
        assert!(sanitized
            .daily_ledgers
            .values()
            .all(|ledger| ledger.incomes.len() == 30));
        assert!(sanitized
            .monthly_snapshots
            .values()
            .all(|snapshot| snapshot.assets.len() == 30));
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
    fn finance_health_score_penalizes_cashflow_pressure_and_drift() {
        let mut daily_ledgers = IndexMap::new();
        daily_ledgers.insert(
            "2026-04-18".to_string(),
            FinanceDayLedger {
                incomes: vec![FinanceIncomeEntry {
                    name: "Salary".to_string(),
                    amount: 1_000,
                    ..default_income_entry()
                }],
                expenses: vec![
                    FinanceExpenseEntry {
                        name: "Debt".to_string(),
                        bucket: FinanceExpenseBucket::Debt,
                        amount: 650,
                        ..default_expense_entry()
                    },
                    FinanceExpenseEntry {
                        name: "Food".to_string(),
                        bucket: FinanceExpenseBucket::Food,
                        amount: 500,
                        ..default_expense_entry()
                    },
                ],
                ..FinanceDayLedger::default()
            },
        );
        let mut monthly_snapshots = IndexMap::new();
        monthly_snapshots.insert(
            "2026-04".to_string(),
            FinanceMonthSnapshot {
                assets: vec![FinanceNamedAmountEntry {
                    id: String::new(),
                    updated_at_epoch_millis: 0,
                    deleted_at_epoch_millis: 0,
                    name: "Cash".to_string(),
                    kind: FinanceNamedAmountKind::CashReserve,
                    amount: 300,
                    amount_minor: None,
                }],
                liabilities: vec![FinanceNamedAmountEntry {
                    id: String::new(),
                    updated_at_epoch_millis: 0,
                    deleted_at_epoch_millis: 0,
                    name: "Card".to_string(),
                    kind: FinanceNamedAmountKind::LiabilityBalance,
                    amount: 2_000,
                    amount_minor: None,
                }],
                ..FinanceMonthSnapshot::default()
            },
        );
        let profile = FinanceProfile {
            daily_ledgers,
            monthly_snapshots,
            ..empty_profile()
        };
        let raw = serde_json::to_string(&profile).unwrap();

        let values = build_finance_health_score_values(&raw, PERIOD_MONTH, 2026, 4, 18).unwrap();

        assert!(values.score < 56);
        assert!(values.cashflow_score < 0.25);
        assert!(values.warning_count >= 2);
        assert!(values.risk_level_code >= RISK_LEVEL_TIGHT);
    }

    #[test]
    fn minimum_target_total_only_combines_lower_bound_commitments() {
        let defaults = FinanceSettings::default();
        assert!((defaults.configured_minimum_target_total() - 0.27).abs() < 0.0001);
        assert_eq!(0.0, defaults.target_overcommit());

        let settings = FinanceSettings {
            expense_categories: vec![
                FinanceExpenseCategoryConfig {
                    bucket: FinanceExpenseBucket::Debt,
                    label: "Debt cap".to_string(),
                    target_share_of_income: Some(1.0),
                },
                FinanceExpenseCategoryConfig {
                    bucket: FinanceExpenseBucket::Other,
                    label: "Reserve".to_string(),
                    target_share_of_income: Some(0.10),
                },
            ],
        };
        assert!((settings.configured_minimum_target_total() - 0.37).abs() < 0.0001);
        assert_eq!(0.0, settings.target_overcommit());
    }

    #[test]
    fn other_reserve_target_is_a_lower_bound_in_budget_and_discipline_rules() {
        let mut settings = FinanceSettings::default();
        settings
            .expense_categories
            .iter_mut()
            .find(|config| config.bucket == FinanceExpenseBucket::Other)
            .unwrap()
            .target_share_of_income = Some(0.10);
        let mut aggregate = FinanceLedgerTotals::empty();
        aggregate.active_income_total = 1_000;
        aggregate.other_expense_total = 200;

        let funded = finance_budget_envelopes(aggregate, &settings);
        let funded_other = funded
            .iter()
            .find(|envelope| envelope.bucket_code == BUCKET_CODE_OTHER)
            .unwrap();
        assert_eq!(BUDGET_STATUS_NORMAL, funded_other.status_code);
        assert!(!is_target_discipline_violation(
            &FinanceExpenseBucket::Other,
            0.10
        ));

        aggregate.other_expense_total = 50;
        let underfunded = finance_budget_envelopes(aggregate, &settings);
        let underfunded_other = underfunded
            .iter()
            .find(|envelope| envelope.bucket_code == BUCKET_CODE_OTHER)
            .unwrap();
        assert_eq!(BUDGET_STATUS_BELOW_FLOOR, underfunded_other.status_code);
        assert_eq!(50, underfunded_other.remaining_amount);
        assert!(is_target_discipline_violation(
            &FinanceExpenseBucket::Other,
            -0.05
        ));
    }

    #[test]
    fn overcommitted_targets_cap_discipline_and_emit_dedicated_alert() {
        let daily_ledgers = IndexMap::from([(
            "2026-04-18".to_string(),
            allocation_ledger(150, 180, 120, 300, 150, 100),
        )]);
        let monthly_snapshots =
            IndexMap::from([("2026-04".to_string(), net_worth_snapshot(100_000, 0))]);
        let valid_profile = FinanceProfile {
            daily_ledgers: daily_ledgers.clone(),
            monthly_snapshots: monthly_snapshots.clone(),
            ..empty_profile()
        };
        let valid_score = valid_profile.finance_health_score_values(PERIOD_DAY, 2026, 4, 18);
        assert!((valid_score.discipline_score - 1.0).abs() < 0.0001);

        let overcommitted_profile = FinanceProfile {
            settings: FinanceSettings {
                expense_categories: vec![
                    FinanceExpenseCategoryConfig {
                        bucket: FinanceExpenseBucket::Btc,
                        label: "Long-term".to_string(),
                        target_share_of_income: Some(0.60),
                    },
                    FinanceExpenseCategoryConfig {
                        bucket: FinanceExpenseBucket::Learning,
                        label: "Learning".to_string(),
                        target_share_of_income: Some(0.50),
                    },
                ],
            },
            daily_ledgers: IndexMap::from([(
                "2026-04-18".to_string(),
                allocation_ledger(0, 0, 600, 0, 500, 0),
            )]),
            monthly_snapshots,
            ..empty_profile()
        };
        let aggregate = overcommitted_profile.aggregate_for_period(PERIOD_DAY, 2026, 4, 18);
        assert!(overcommitted_profile
            .most_problematic_target_drift(aggregate)
            .is_none());

        let score = overcommitted_profile.finance_health_score_values(PERIOD_DAY, 2026, 4, 18);
        assert!((score.discipline_score - 0.50).abs() < 0.0001);
        assert!(score.warning_count >= 1);
        let plan = overcommitted_profile.finance_alert_plan(PERIOD_DAY, 2026, 4, 18);
        let overcommit_alert = plan
            .iter()
            .find(|item| item.kind_code == ALERT_KIND_TARGET_OVERCOMMITTED)
            .expect("overcommitted targets must emit a dedicated warning");
        assert_eq!(BUCKET_CODE_NONE, overcommit_alert.bucket_code);
        assert!((overcommit_alert.drift - 0.10).abs() < 0.0001);
        assert!(plan.iter().all(|item| item.kind_code != ALERT_KIND_CLEAN));
    }

    #[test]
    fn discipline_score_has_no_target_drift_dead_zone_but_alert_threshold_is_preserved() {
        let monthly_snapshots =
            IndexMap::from([("2026-04".to_string(), net_worth_snapshot(100_000, 0))]);
        let seven_point_profile = FinanceProfile {
            daily_ledgers: IndexMap::from([(
                "2026-04-18".to_string(),
                allocation_ledger(320, 110, 120, 300, 150, 0),
            )]),
            monthly_snapshots: monthly_snapshots.clone(),
            ..empty_profile()
        };
        let seven_point_aggregate =
            seven_point_profile.aggregate_for_period(PERIOD_DAY, 2026, 4, 18);
        assert!(seven_point_profile
            .most_problematic_target_drift(seven_point_aggregate)
            .is_none());
        let seven_point_score =
            seven_point_profile.finance_health_score_values(PERIOD_DAY, 2026, 4, 18);
        assert!((seven_point_score.discipline_score - 0.65).abs() < 0.0001);

        let nine_point_profile = FinanceProfile {
            daily_ledgers: IndexMap::from([(
                "2026-04-18".to_string(),
                allocation_ledger(340, 90, 120, 300, 150, 0),
            )]),
            monthly_snapshots,
            ..empty_profile()
        };
        let nine_point_aggregate = nine_point_profile.aggregate_for_period(PERIOD_DAY, 2026, 4, 18);
        let (bucket, drift) = nine_point_profile
            .most_problematic_target_drift(nine_point_aggregate)
            .expect("a nine-point overrun must cross the alert threshold");
        assert_eq!(FinanceExpenseBucket::Debt, bucket);
        assert!((drift - 0.09).abs() < 0.0001);
        let nine_point_score =
            nine_point_profile.finance_health_score_values(PERIOD_DAY, 2026, 4, 18);
        assert!((nine_point_score.discipline_score - 0.55).abs() < 0.0001);
    }

    #[test]
    fn trend_uses_equal_elapsed_days_and_excludes_future_rows_for_month_quarter_and_year() {
        let mut daily_ledgers = IndexMap::new();
        for (key, amount) in [
            ("2026-06-01", 40),
            ("2026-06-11", 60),
            ("2026-06-12", 1_000),
            ("2026-07-01", 100),
            ("2026-07-11", 100),
            ("2026-07-12", 1_000),
            ("2026-01-01", 40),
            ("2026-02-09", 60),
            ("2026-02-10", 1_000),
            ("2026-04-01", 100),
            ("2026-05-10", 100),
            ("2026-05-11", 1_000),
            ("2023-01-01", 40),
            ("2023-03-02", 60),
            ("2023-03-03", 1_000),
            ("2024-01-01", 100),
            ("2024-03-01", 100),
            ("2024-03-02", 1_000),
        ] {
            daily_ledgers.insert(key.to_string(), income_ledger(amount));
        }
        for month in [6, 7] {
            for day in 2..=7 {
                daily_ledgers.insert(
                    format!("2026-{month:02}-{day:02}"),
                    FinanceDayLedger {
                        confirmed_at_epoch_millis: 1,
                        ..FinanceDayLedger::default()
                    },
                );
            }
        }
        let profile = FinanceProfile {
            daily_ledgers,
            ..empty_profile()
        };

        let month = profile.finance_trend_values(PERIOD_MONTH, 2026, 7, 11);
        let quarter = profile.finance_trend_values(PERIOD_QUARTER, 2026, 5, 10);
        let year = profile.finance_trend_values(PERIOD_YEAR, 2024, 3, 1);

        assert!(month.cashflow_comparison_available);
        assert_eq!(100, month.income_delta);
        assert_eq!(8, month.current_recorded_days);
        assert_eq!(8, month.previous_recorded_days);
        for trend in [quarter, year] {
            assert!(!trend.cashflow_comparison_available);
            assert_eq!(0, trend.income_delta);
            assert_eq!(2, trend.current_recorded_days);
            assert_eq!(2, trend.previous_recorded_days);
        }
        assert_eq!(
            200,
            profile
                .aggregate_for_period(PERIOD_MONTH, 2026, 7, 11)
                .income_total()
        );
        assert_eq!(
            ((2026, 7, 30), (2026, 6, 30)),
            comparable_period_reference_dates(PERIOD_MONTH, 2026, 7, 31)
        );
        assert_eq!(
            ((2024, 3, 1), (2023, 3, 2)),
            comparable_period_reference_dates(PERIOD_YEAR, 2024, 3, 1)
        );
    }

    #[test]
    fn health_score_cannot_report_steady_when_net_worth_is_deeply_negative() {
        let mut daily_ledgers = IndexMap::new();
        daily_ledgers.insert("2026-07-11".to_string(), healthy_cashflow_ledger());
        let mut monthly_snapshots = IndexMap::new();
        monthly_snapshots.insert("2026-07".to_string(), net_worth_snapshot(1_000, 1_000_000));
        let raw = serde_json::to_string(&FinanceProfile {
            daily_ledgers,
            monthly_snapshots,
            ..empty_profile()
        })
        .unwrap();

        let values = build_finance_health_score_values(&raw, PERIOD_MONTH, 2026, 7, 11).unwrap();

        assert_eq!(0.0, values.liability_score);
        assert_eq!(RISK_LEVEL_CRITICAL, values.risk_level_code);
        assert!(values.warning_count >= 1);
        assert!(values.score < 86);
    }

    #[test]
    fn health_score_treats_balance_sheet_snapshot_older_than_three_months_as_stale() {
        let mut daily_ledgers = IndexMap::new();
        daily_ledgers.insert("2026-07-11".to_string(), healthy_cashflow_ledger());
        let mut fresh_snapshots = IndexMap::new();
        fresh_snapshots.insert("2026-07".to_string(), net_worth_snapshot(100_000, 0));
        let mut stale_snapshots = IndexMap::new();
        stale_snapshots.insert("2026-03".to_string(), net_worth_snapshot(100_000, 0));
        let fresh = FinanceProfile {
            daily_ledgers: daily_ledgers.clone(),
            monthly_snapshots: fresh_snapshots,
            ..empty_profile()
        }
        .finance_health_score_values(PERIOD_MONTH, 2026, 7, 11);
        let stale = FinanceProfile {
            daily_ledgers,
            monthly_snapshots: stale_snapshots,
            ..empty_profile()
        }
        .finance_health_score_values(PERIOD_MONTH, 2026, 7, 11);

        assert_eq!(1.0, fresh.defensive_score);
        assert_eq!(0.0, stale.defensive_score);
        assert_eq!(1.0, fresh.liability_score);
        assert_eq!(0.35, stale.liability_score);
        assert!(fresh.score >= stale.score + 20);
        assert!(stale.warning_count > fresh.warning_count);
    }

    #[test]
    fn sparse_ledger_caps_health_score_and_counts_as_a_warning() {
        let profile = FinanceProfile {
            daily_ledgers: IndexMap::from([("2026-07-11".to_string(), healthy_cashflow_ledger())]),
            monthly_snapshots: IndexMap::from([(
                "2026-07".to_string(),
                net_worth_snapshot(100_000, 0),
            )]),
            ..empty_profile()
        };

        let day = profile.finance_health_score_values(PERIOD_DAY, 2026, 7, 11);
        let month = profile.finance_health_score_values(PERIOD_MONTH, 2026, 7, 11);
        let month_alerts = profile.finance_alert_plan(PERIOD_MONTH, 2026, 7, 11);

        assert!(day.score > month.score);
        assert!(month.score <= 69);
        assert!(month.risk_level_code >= RISK_LEVEL_WATCH);
        assert!(month.warning_count >= 1);
        assert!(month_alerts
            .iter()
            .any(|item| item.kind_code == ALERT_KIND_LOW_RECORD_DENSITY));
    }

    #[test]
    fn alert_plan_surfaces_balance_sheet_risk_and_stale_snapshots() {
        let mut daily_ledgers = IndexMap::new();
        daily_ledgers.insert("2026-07-11".to_string(), healthy_cashflow_ledger());

        let mut risky_snapshots = IndexMap::new();
        risky_snapshots.insert("2026-07".to_string(), net_worth_snapshot(1_000, 5_000));
        let risky = FinanceProfile {
            daily_ledgers: daily_ledgers.clone(),
            monthly_snapshots: risky_snapshots,
            ..empty_profile()
        }
        .finance_alert_plan(PERIOD_MONTH, 2026, 7, 11);
        assert!(risky
            .iter()
            .any(|item| item.kind_code == ALERT_KIND_BALANCE_SHEET_RISK));

        let mut stale_snapshots = IndexMap::new();
        stale_snapshots.insert("2026-03".to_string(), net_worth_snapshot(100_000, 0));
        let stale = FinanceProfile {
            daily_ledgers,
            monthly_snapshots: stale_snapshots,
            ..empty_profile()
        }
        .finance_alert_plan(PERIOD_MONTH, 2026, 7, 11);
        assert!(stale
            .iter()
            .any(|item| item.kind_code == ALERT_KIND_STALE_MONTH_SNAPSHOT));
    }

    #[test]
    fn first_detailed_row_preserves_legacy_balance_sheet_without_treating_it_as_fresh() {
        let profile = FinanceProfile {
            cash_reserve: 60_000,
            productive_asset_value: 40_000,
            liability_balance: 10_000,
            daily_ledgers: IndexMap::from([(
                "2026-07-11".to_string(),
                FinanceDayLedger {
                    incomes: vec![FinanceIncomeEntry {
                        name: "Income".to_string(),
                        amount: 1_000,
                        ..default_income_entry()
                    }],
                    expenses: vec![FinanceExpenseEntry {
                        name: "Living".to_string(),
                        bucket: FinanceExpenseBucket::Living,
                        amount: 100,
                        ..default_expense_entry()
                    }],
                    ..FinanceDayLedger::default()
                },
            )]),
            ..empty_profile()
        };

        let report = profile.detailed_finance_report_values(PERIOD_MONTH, 2026, 7, 11);
        let health = profile.finance_health_score_values(PERIOD_MONTH, 2026, 7, 11);
        let alerts = profile.finance_alert_plan(PERIOD_MONTH, 2026, 7, 11);

        assert_eq!(90_000, report.net_worth);
        assert_eq!(Some(600.0), report.defensive_coverage);
        assert_eq!(0.0, health.defensive_score);
        assert_eq!(0.35, health.liability_score);
        assert!(alerts
            .iter()
            .any(|item| item.kind_code == ALERT_KIND_STALE_MONTH_SNAPSHOT));
    }

    #[test]
    fn balance_sheet_only_detail_keeps_legacy_cashflow_domain() {
        let profile = FinanceProfile {
            active_income_monthly: 30_000,
            asset_income_monthly: 3_000,
            living_expense_monthly: 10_000,
            liability_payment_monthly: 2_000,
            monthly_snapshots: IndexMap::from([(
                "2026-07".to_string(),
                FinanceMonthSnapshot {
                    confirmed_at_epoch_millis: 1_000,
                    ..FinanceMonthSnapshot::default()
                },
            )]),
            ..empty_profile()
        };

        assert!(!profile.has_detailed_cashflow_data());
        assert!(profile.has_detailed_balance_sheet_data());
        let report = profile.detailed_finance_report_values(PERIOD_MONTH, 2026, 7, 11);
        let health = profile.finance_health_score_values(PERIOD_MONTH, 2026, 7, 11);

        assert_eq!(33_000, report.total_income);
        assert_eq!(12_000, report.total_outflow);
        assert_eq!(21_000, report.net_cashflow);
        assert_eq!(0, report.recorded_days);
        assert!(health.score <= 69);
    }

    #[test]
    fn defensive_health_score_uses_months_across_day_and_month_views() {
        let first_day_months = defensive_coverage_in_months(Some(6.0), 1).unwrap();
        let full_month_months = defensive_coverage_in_months(Some(6.0), 30).unwrap();
        assert!((first_day_months - 0.197).abs() < 0.001);
        assert!((full_month_months - 5.914).abs() < 0.001);

        let mut daily_ledgers = IndexMap::new();
        for day in 1..=30 {
            daily_ledgers.insert(
                format!("2026-04-{day:02}"),
                FinanceDayLedger {
                    incomes: vec![FinanceIncomeEntry {
                        name: "Income".to_string(),
                        amount: 200,
                        ..default_income_entry()
                    }],
                    expenses: vec![FinanceExpenseEntry {
                        name: "Living".to_string(),
                        bucket: FinanceExpenseBucket::Living,
                        amount: 100,
                        ..default_expense_entry()
                    }],
                    ..FinanceDayLedger::default()
                },
            );
        }
        let mut monthly_snapshots = IndexMap::new();
        monthly_snapshots.insert("2026-04".to_string(), net_worth_snapshot(6_000, 0));
        let profile = FinanceProfile {
            daily_ledgers,
            monthly_snapshots,
            ..empty_profile()
        };

        let day = profile.finance_health_score_values(PERIOD_DAY, 2026, 4, 30);
        let month = profile.finance_health_score_values(PERIOD_MONTH, 2026, 4, 30);

        assert!((day.defensive_score - month.defensive_score).abs() < 0.001);
        assert!(day.defensive_score > 0.30 && day.defensive_score < 0.35);
    }

    #[test]
    fn note_only_periods_do_not_count_as_recorded_finance_days_or_trend_anchors() {
        let note_only_day = FinanceDayLedger {
            note: "memo only".to_string(),
            ..FinanceDayLedger::default()
        };
        let financial_day = FinanceDayLedger {
            incomes: vec![FinanceIncomeEntry {
                name: "Income".to_string(),
                amount: 100,
                ..default_income_entry()
            }],
            note: "memo with money".to_string(),
            ..FinanceDayLedger::default()
        };
        let mut note_only_month = FinanceMonthSnapshot::default();
        note_only_month.note = "memo only".to_string();
        let profile = FinanceProfile {
            daily_ledgers: IndexMap::from([
                ("2026-07-09".to_string(), note_only_day),
                ("2026-07-10".to_string(), financial_day),
            ]),
            monthly_snapshots: IndexMap::from([
                ("2026-05".to_string(), net_worth_snapshot(1_000, 0)),
                ("2026-06".to_string(), note_only_month),
            ]),
            ..empty_profile()
        };

        let aggregate = profile.aggregate_for_period(PERIOD_MONTH, 2026, 7, 11);
        assert_eq!(1, aggregate.days_with_entries);
        assert_eq!(
            Some("2026-07-10".to_string()),
            profile.previous_recorded_day_key("2026-07-11")
        );
        assert_eq!(None, profile.previous_recorded_day_key("2026-07-10"));
        assert_eq!(
            Some("2026-05".to_string()),
            profile.previous_recorded_month_key("2026-07")
        );
    }

    #[test]
    fn explicitly_confirmed_zero_balance_sheet_is_a_valid_fresh_snapshot() {
        let profile = FinanceProfile {
            daily_ledgers: IndexMap::from([("2026-07-11".to_string(), healthy_cashflow_ledger())]),
            monthly_snapshots: IndexMap::from([(
                "2026-07".to_string(),
                FinanceMonthSnapshot {
                    confirmed_at_epoch_millis: 1_000,
                    ..FinanceMonthSnapshot::default()
                },
            )]),
            ..empty_profile()
        };

        assert_eq!(
            Some("2026-07".to_string()),
            profile.latest_snapshot_month_key_up_to("2026-07")
        );
        let alerts = profile.finance_alert_plan(PERIOD_MONTH, 2026, 7, 11);
        assert!(alerts
            .iter()
            .any(|item| item.kind_code == ALERT_KIND_EMPTY_MONTH_SNAPSHOT));
        assert!(!alerts
            .iter()
            .any(|item| item.kind_code == ALERT_KIND_STALE_MONTH_SNAPSHOT));
    }

    #[test]
    fn year_net_worth_opening_prefers_latest_snapshot_before_year_start() {
        let mut monthly_snapshots = IndexMap::new();
        monthly_snapshots.insert("2025-12".to_string(), net_worth_snapshot(1_000, 100));
        monthly_snapshots.insert("2026-03".to_string(), net_worth_snapshot(1_400, 200));
        monthly_snapshots.insert(
            "2026-10".to_string(),
            FinanceMonthSnapshot {
                assets: vec![FinanceNamedAmountEntry {
                    id: "deleted-future".to_string(),
                    name: "Deleted".to_string(),
                    amount: 9_000,
                    deleted_at_epoch_millis: 10,
                    ..default_named_amount_entry()
                }],
                note: "note only".to_string(),
                ..FinanceMonthSnapshot::default()
            },
        );
        monthly_snapshots.insert("2026-11".to_string(), net_worth_snapshot(2_000, 300));
        let profile = FinanceProfile {
            monthly_snapshots,
            ..empty_profile()
        };

        assert_eq!(
            FinanceYearNetWorthValues {
                opening_net_worth: 900,
                closing_net_worth: 1_200,
                opening_month_code: 202512,
                closing_month_code: 202603,
                has_calendar_year_opening: true,
            },
            profile.year_net_worth_summary(2026, 7)
        );
        assert_eq!(
            profile.year_net_worth_summary(2026, 7),
            profile.year_net_worth_summary(2026, 10)
        );
        assert_eq!(
            FinanceYearNetWorthValues {
                opening_net_worth: 900,
                closing_net_worth: 1_700,
                opening_month_code: 202512,
                closing_month_code: 202611,
                has_calendar_year_opening: true,
            },
            profile.year_net_worth_summary(2026, 12)
        );
    }

    #[test]
    fn sanitizer_preserves_named_zero_legacy_rows_without_counting_them() {
        let mut daily_ledgers = IndexMap::new();
        daily_ledgers.insert(
            "2026-07-11".to_string(),
            FinanceDayLedger {
                incomes: vec![FinanceIncomeEntry {
                    id: "legacy-income".to_string(),
                    name: "Salary".to_string(),
                    amount: 0,
                    amount_minor: None,
                    ..default_income_entry()
                }],
                ..FinanceDayLedger::default()
            },
        );
        let mut monthly_snapshots = IndexMap::new();
        monthly_snapshots.insert(
            "2026-07".to_string(),
            FinanceMonthSnapshot {
                assets: vec![FinanceNamedAmountEntry {
                    id: "legacy-asset".to_string(),
                    name: "Cash".to_string(),
                    amount: 0,
                    amount_minor: None,
                    ..default_named_amount_entry()
                }],
                ..FinanceMonthSnapshot::default()
            },
        );

        let profile = FinanceProfile {
            daily_ledgers,
            monthly_snapshots,
            ..empty_profile()
        }
        .sanitized();
        let ledger = &profile.daily_ledgers["2026-07-11"];
        let snapshot = &profile.monthly_snapshots["2026-07"];
        assert_eq!(1, ledger.incomes.len());
        assert_eq!(1, snapshot.assets.len());
        assert!(ledger.has_entries());
        assert!(!ledger.has_financial_entries());
        assert!(snapshot.has_entries());
        assert!(!snapshot.has_financial_entries());
    }

    #[test]
    fn sanitizer_backfills_stable_ids_and_repairs_duplicate_explicit_ids() {
        let mut daily_ledgers = IndexMap::new();
        daily_ledgers.insert(
            "2026-07-11".to_string(),
            FinanceDayLedger {
                incomes: vec![
                    FinanceIncomeEntry {
                        id: "kept-id".to_string(),
                        name: "Salary".to_string(),
                        amount: 1_000,
                        ..default_income_entry()
                    },
                    FinanceIncomeEntry {
                        id: "kept-id".to_string(),
                        name: "Bonus".to_string(),
                        amount: 200,
                        ..default_income_entry()
                    },
                    FinanceIncomeEntry {
                        name: "Bonus".to_string(),
                        amount: 200,
                        ..default_income_entry()
                    },
                ],
                ..FinanceDayLedger::default()
            },
        );
        let profile = FinanceProfile {
            daily_ledgers,
            ..empty_profile()
        };

        let first = profile.clone().sanitized();
        let second = profile.sanitized();
        let first_ids = first.daily_ledgers["2026-07-11"]
            .incomes
            .iter()
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>();
        let second_ids = second.daily_ledgers["2026-07-11"]
            .incomes
            .iter()
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>();

        assert_eq!(first_ids, second_ids);
        assert_eq!(1, first_ids.iter().filter(|id| *id == "kept-id").count());
        assert_ne!(first_ids[0], first_ids[1]);
        assert_ne!(first_ids[1], first_ids[2]);
        assert_eq!(3, first_ids.iter().collect::<HashSet<_>>().len());
        assert_eq!(
            2,
            first_ids
                .iter()
                .filter(|id| id.starts_with("fin-legacy-"))
                .count()
        );
    }

    #[test]
    fn finance_entry_revision_fields_are_backward_compatible_and_camel_case() {
        let missing = serde_json::from_value::<FinanceIncomeEntry>(json!({
            "id": "legacy",
            "name": "Salary",
            "kind": "ACTIVE",
            "amount": 100,
            "note": ""
        }))
        .unwrap();
        let nulls = serde_json::from_value::<FinanceExpenseEntry>(json!({
            "id": "legacy-null",
            "updatedAtEpochMillis": null,
            "deletedAtEpochMillis": null,
            "name": "Food",
            "bucket": "FOOD",
            "amount": 10,
            "note": ""
        }))
        .unwrap();
        assert_eq!(0, missing.updated_at_epoch_millis);
        assert_eq!(0, missing.deleted_at_epoch_millis);
        assert_eq!(0, nulls.updated_at_epoch_millis);
        assert_eq!(0, nulls.deleted_at_epoch_millis);

        let encoded = serde_json::to_value(FinanceIncomeEntry {
            updated_at_epoch_millis: 123,
            deleted_at_epoch_millis: 456,
            ..missing
        })
        .unwrap();
        assert_eq!(123, encoded["updatedAtEpochMillis"]);
        assert_eq!(456, encoded["deletedAtEpochMillis"]);
    }

    #[test]
    fn finance_tombstones_persist_but_do_not_affect_visibility_or_totals() {
        let raw = json!({
            "dailyLedgers": {
                "2026-07-11": {
                    "incomes": [
                        {"id":"live","name":"Salary","kind":"ACTIVE","amount":100,"note":""},
                        {"id":"gone","updatedAtEpochMillis":100,"deletedAtEpochMillis":200,"name":"Old","kind":"ACTIVE","amount":900,"note":""},
                        {"id":"","deletedAtEpochMillis":300,"name":"Invalid","kind":"ACTIVE","amount":800,"note":""}
                    ],
                    "expenses": [
                        {"id":"gone-expense","deletedAtEpochMillis":200,"name":"Old food","bucket":"FOOD","amount":400,"note":""}
                    ],
                    "note":""
                },
                "2026-07-10": {
                    "incomes": [{"id":"tombstone-only","deletedAtEpochMillis":200,"name":"Old","kind":"ACTIVE","amount":500,"note":""}],
                    "expenses": [],
                    "note":""
                }
            },
            "monthlySnapshots": {
                "2026-07": {
                    "assets": [
                        {"id":"cash","name":"Cash","kind":"CASH_RESERVE","amount":2000},
                        {"id":"gone-asset","deletedAtEpochMillis":200,"name":"Old asset","kind":"OTHER_ASSET","amount":9999}
                    ],
                    "liabilities": [
                        {"id":"gone-debt","deletedAtEpochMillis":200,"name":"Old debt","kind":"LIABILITY_BALANCE","amount":1000}
                    ],
                    "note":""
                }
            }
        })
        .to_string();
        let sanitized_json = sanitize_finance_profile_json(&raw).unwrap();
        let profile = serde_json::from_str::<FinanceProfile>(&sanitized_json).unwrap();
        let ledger = &profile.daily_ledgers["2026-07-11"];
        assert_eq!(2, ledger.incomes.len());
        assert!(ledger.incomes.iter().any(|entry| entry.id == "gone"));
        assert!(profile.daily_ledgers.contains_key("2026-07-10"));
        assert_eq!(
            Some("2026-07-11".to_string()),
            profile.previous_recorded_day_key("2026-07-12")
        );

        let totals =
            aggregate_finance_ledger_values(&sanitized_json, PERIOD_MONTH, 2026, 7, 11).unwrap();
        assert_eq!(100, totals.income_total());
        assert_eq!(0, totals.expense_total());
        assert_eq!(1, totals.days_with_entries);
        let summary = profile.monthly_snapshots["2026-07"].to_summary_totals();
        assert_eq!(2_000, summary.asset_total);
        assert_eq!(0, summary.liability_total);

        let tombstone_only = FinanceProfile {
            daily_ledgers: IndexMap::from([(
                "2026-07-10".to_string(),
                profile.daily_ledgers["2026-07-10"].clone(),
            )]),
            ..empty_profile()
        };
        assert!(!tombstone_only.has_entries());
        assert!(!tombstone_only.has_detailed_finance_data());
        assert_eq!(None, tombstone_only.previous_recorded_day_key("2026-07-12"));
    }

    #[test]
    fn duplicate_explicit_ids_are_repaired_independently_of_container_order() {
        fn profile_with_order(reverse: bool) -> FinanceProfile {
            let entries = [("2026-07-10", "Alpha", 100), ("2026-07-11", "Beta", 200)];
            let ordered = if reverse {
                vec![entries[1], entries[0]]
            } else {
                entries.to_vec()
            };
            let mut daily_ledgers = IndexMap::new();
            for (day, name, amount) in ordered {
                daily_ledgers.insert(
                    day.to_string(),
                    FinanceDayLedger {
                        incomes: vec![FinanceIncomeEntry {
                            id: "duplicate".to_string(),
                            name: name.to_string(),
                            amount,
                            ..default_income_entry()
                        }],
                        ..FinanceDayLedger::default()
                    },
                );
            }
            FinanceProfile {
                daily_ledgers,
                ..empty_profile()
            }
        }
        let forward = profile_with_order(false).sanitized();
        let reverse = profile_with_order(true).sanitized();
        for day in ["2026-07-10", "2026-07-11"] {
            assert_eq!(
                forward.daily_ledgers[day].incomes[0].id,
                reverse.daily_ledgers[day].incomes[0].id
            );
        }
    }

    #[test]
    fn sanitizer_filters_invalid_ledger_keys_and_zero_value_rows() {
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
                confirmed_at_epoch_millis: 0,
            },
        );

        let sanitized = FinanceProfile {
            daily_ledgers,
            ..empty_profile()
        }
        .sanitized();

        assert_eq!(1, sanitized.daily_ledgers.len());
        assert!(!sanitized.daily_ledgers.contains_key("bad-key"));
        assert!(!sanitized.daily_ledgers.contains_key("2026-02-31"));
        let ledger = sanitized.daily_ledgers.get("2026-04-08").unwrap();
        assert_eq!(1, ledger.incomes.len());
        assert_eq!(1, ledger.expenses.len());
        assert_eq!(" Salary ", ledger.incomes[0].name);
        assert_eq!(" steady ", ledger.note);
        assert!(!ledger.incomes[0].id.is_empty());
        assert!(!ledger.expenses[0].id.is_empty());
    }

    #[test]
    fn sanitizer_filters_bad_months_and_moves_misplaced_balance_rows() {
        let mut monthly_snapshots = IndexMap::new();
        monthly_snapshots.insert(
            "2026-04".to_string(),
            FinanceMonthSnapshot {
                assets: vec![FinanceNamedAmountEntry {
                    id: String::new(),
                    updated_at_epoch_millis: 0,
                    deleted_at_epoch_millis: 0,
                    name: " Card ".to_string(),
                    kind: FinanceNamedAmountKind::LiabilityBalance,
                    amount: 800,
                    amount_minor: None,
                }],
                liabilities: vec![FinanceNamedAmountEntry {
                    id: String::new(),
                    updated_at_epoch_millis: 0,
                    deleted_at_epoch_millis: 0,
                    name: " Cash ".to_string(),
                    kind: FinanceNamedAmountKind::CashReserve,
                    amount: 1_000,
                    amount_minor: None,
                }],
                note: "\r\n month end ".to_string(),
                confirmed_at_epoch_millis: 0,
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
        assert!(!sanitized.monthly_snapshots.contains_key("2026-13"));
        let snapshot = sanitized.monthly_snapshots.get("2026-04").unwrap();
        assert_eq!(FinanceNamedAmountKind::CashReserve, snapshot.assets[0].kind);
        assert_eq!(
            FinanceNamedAmountKind::LiabilityBalance,
            snapshot.liabilities[0].kind
        );
        assert_eq!(" Cash ", snapshot.assets[0].name);
        assert_eq!(" Card ", snapshot.liabilities[0].name);
        assert_eq!("\r\n month end ", snapshot.note);
        assert!(!snapshot.assets[0].id.is_empty());
        assert!(!snapshot.liabilities[0].id.is_empty());
    }

    #[test]
    fn upsert_day_ledger_json_preserves_rows_and_removes_explicit_blank_ledger() {
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
        assert_eq!(" Salary ", ledger.incomes[0].name);
        assert_eq!(1200, ledger.incomes[0].amount);
        assert_eq!(" day note ", ledger.note);

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
    fn json_entrypoint_filters_invalid_dates_and_empty_rows_without_truncating_values() {
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

        assert_eq!(i64::MAX, profile.active_income_monthly);
        assert_eq!(-5, profile.asset_income_monthly);
        assert!(profile.daily_ledgers.contains_key("2026-04-08"));
        assert!(!profile.daily_ledgers.contains_key("2026-04-31"));
        let ledger = profile.daily_ledgers.get("2026-04-08").unwrap();
        assert!(ledger.expenses.is_empty());
        assert!(!ledger.incomes[0].id.is_empty());
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
            id: String::new(),
            updated_at_epoch_millis: 0,
            deleted_at_epoch_millis: 0,
            name: " Salary ".to_string(),
            kind: FinanceIncomeKind::Active,
            amount: 1_200,
            note: "paid".to_string(),
            amount_minor: None,
        }];
        let templates = vec![
            FinanceIncomeEntry {
                id: String::new(),
                updated_at_epoch_millis: 0,
                deleted_at_epoch_millis: 0,
                name: "salary".to_string(),
                kind: FinanceIncomeKind::Active,
                amount: 0,
                amount_minor: None,
                note: String::new(),
            },
            FinanceIncomeEntry {
                id: String::new(),
                updated_at_epoch_millis: 0,
                deleted_at_epoch_millis: 0,
                name: "Bonus".to_string(),
                kind: FinanceIncomeKind::Other,
                amount: 0,
                amount_minor: None,
                note: "annual".to_string(),
            },
            FinanceIncomeEntry {
                id: String::new(),
                updated_at_epoch_millis: 0,
                deleted_at_epoch_millis: 0,
                name: "bonus".to_string(),
                kind: FinanceIncomeKind::Other,
                amount: 0,
                amount_minor: None,
                note: "duplicate".to_string(),
            },
            FinanceIncomeEntry {
                id: String::new(),
                updated_at_epoch_millis: 0,
                deleted_at_epoch_millis: 0,
                name: "   ".to_string(),
                kind: FinanceIncomeKind::Asset,
                amount: 0,
                amount_minor: None,
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
            id: String::new(),
            updated_at_epoch_millis: 0,
            deleted_at_epoch_millis: 0,
            name: "Salary".to_string(),
            kind: FinanceIncomeKind::Active,
            amount: 1_200,
            note: String::new(),
            amount_minor: None,
        }];
        let bonus = FinanceIncomeEntry {
            id: String::new(),
            updated_at_epoch_millis: 0,
            deleted_at_epoch_millis: 0,
            name: "Bonus".to_string(),
            kind: FinanceIncomeKind::Other,
            amount: 300,
            note: "annual".to_string(),
            amount_minor: None,
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
            id: String::new(),
            updated_at_epoch_millis: 0,
            deleted_at_epoch_millis: 0,
            name: "Food".to_string(),
            bucket: FinanceExpenseBucket::Food,
            amount: 80,
            note: String::new(),
            amount_minor: None,
        }];
        let debt = FinanceExpenseEntry {
            id: String::new(),
            updated_at_epoch_millis: 0,
            deleted_at_epoch_millis: 0,
            name: "Debt".to_string(),
            bucket: FinanceExpenseBucket::Debt,
            amount: 100,
            note: "card".to_string(),
            amount_minor: None,
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
                id: String::new(),
                updated_at_epoch_millis: 0,
                deleted_at_epoch_millis: 0,
                name: "Cash".to_string(),
                kind: FinanceNamedAmountKind::CashReserve,
                amount: 1_000,
                amount_minor: None,
            },
            FinanceNamedAmountEntry {
                id: String::new(),
                updated_at_epoch_millis: 0,
                deleted_at_epoch_millis: 0,
                name: "Loan".to_string(),
                kind: FinanceNamedAmountKind::LiabilityBalance,
                amount: 500,
                amount_minor: None,
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
    fn encode_finance_backup_json_preserves_profile_and_sanitizes_metadata() {
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
        assert_eq!(i64::MAX, payload.finance_profile.active_income_monthly);
        assert_eq!(
            "  Build    assets ",
            payload.finance_profile.acquisition_focus
        );
    }

    #[test]
    fn day_ledger_and_month_snapshot_default_json_keep_lookup_rules_native() {
        let mut daily_ledgers = IndexMap::new();
        daily_ledgers.insert(
            "2026-04-08".to_string(),
            FinanceDayLedger {
                incomes: vec![FinanceIncomeEntry {
                    id: String::new(),
                    updated_at_epoch_millis: 0,
                    deleted_at_epoch_millis: 0,
                    name: "Salary".to_string(),
                    kind: FinanceIncomeKind::Active,
                    amount: 800,
                    note: String::new(),
                    amount_minor: None,
                }],
                ..FinanceDayLedger::default()
            },
        );
        let mut monthly_snapshots = IndexMap::new();
        monthly_snapshots.insert(
            "2026-04".to_string(),
            FinanceMonthSnapshot {
                assets: vec![FinanceNamedAmountEntry {
                    id: String::new(),
                    updated_at_epoch_millis: 0,
                    deleted_at_epoch_millis: 0,
                    name: "Cash".to_string(),
                    kind: FinanceNamedAmountKind::CashReserve,
                    amount: 2_000,
                    amount_minor: None,
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
    fn settings_config_json_and_replacement_preserve_values_and_bucket_order() {
        let settings_json = r#"{
            "expenseCategories": [
                {"bucket":"FOOD","label":" Food Budget ","targetShareOfIncome":2.0},
                {"bucket":"DEBT","label":"","targetShareOfIncome":0.25}
            ]
        }"#;

        let food_config = finance_settings_config_for_json(settings_json, 1).unwrap();
        let food = serde_json::from_str::<FinanceExpenseCategoryConfig>(&food_config).unwrap();
        assert_eq!(FinanceExpenseBucket::Food, food.bucket);
        assert_eq!(" Food Budget ", food.label);
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
        assert_eq!("Long Long Learning Label", learning.label);
        assert_eq!(Some(0.0), learning.target_share_of_income);

        let defaults = default_finance_expense_category_configs_json().unwrap();
        assert_eq!(
            6,
            serde_json::from_str::<Vec<FinanceExpenseCategoryConfig>>(&defaults)
                .unwrap()
                .len()
        );
    }

    #[test]
    fn risk_cockpit_marks_asset_only_profile_unknown_instead_of_safe() {
        let raw = serde_json::to_string(&FinanceProfile {
            monthly_snapshots: IndexMap::from([(
                "2026-04".to_string(),
                FinanceMonthSnapshot {
                    assets: vec![FinanceNamedAmountEntry {
                        name: "Cash".to_string(),
                        kind: FinanceNamedAmountKind::OtherAsset,
                        amount: 20_000,
                        ..default_named_amount_entry()
                    }],
                    ..FinanceMonthSnapshot::default()
                },
            )]),
            ..empty_profile()
        })
        .unwrap();

        let result = build_finance_risk_cockpit_json(&raw, PERIOD_MONTH, 2026, 4, 10).unwrap();
        let snapshot = serde_json::from_str::<FinanceRiskCockpitSnapshot>(&result).unwrap();

        assert_eq!(FinanceRiskState::Unknown, snapshot.risk_state);
        assert_eq!(REASON_CASHFLOW_BASELINE_MISSING, snapshot.reason_code);
        assert_eq!(FINANCE_RISK_POLICY_VERSION, snapshot.policy_version);
        assert_eq!(25, snapshot.data_confidence);
        assert_eq!(None, snapshot.emergency_months);
        assert!(!snapshot
            .actions
            .iter()
            .any(|action| action.kind_code == ACTION_KIND_CURRENTLY_STABLE));
    }

    #[test]
    fn risk_cockpit_full_sample_builds_budget_forecast_and_balance_sheet_metrics() {
        let mut daily_ledgers = IndexMap::new();
        for day in 1..=10 {
            daily_ledgers.insert(
                format!("2026-04-{day:02}"),
                FinanceDayLedger {
                    incomes: vec![FinanceIncomeEntry {
                        name: "Salary".to_string(),
                        amount: 1_000,
                        ..default_income_entry()
                    }],
                    expenses: vec![
                        expense("Debt", FinanceExpenseBucket::Debt, 100),
                        expense("Food", FinanceExpenseBucket::Food, 100),
                        expense("BTC", FinanceExpenseBucket::Btc, 50),
                        expense("Living", FinanceExpenseBucket::Living, 200),
                        expense("Learning", FinanceExpenseBucket::Learning, 20),
                        expense("Other", FinanceExpenseBucket::Other, 10),
                    ],
                    ..FinanceDayLedger::default()
                },
            );
        }
        let mut settings = FinanceSettings::default();
        settings
            .expense_categories
            .iter_mut()
            .find(|config| config.bucket == FinanceExpenseBucket::Other)
            .unwrap()
            .target_share_of_income = Some(0.05);
        let mut monthly_snapshots = IndexMap::new();
        monthly_snapshots.insert("2026-04".to_string(), net_worth_snapshot(12_000, 6_000));
        let raw = serde_json::to_string(&FinanceProfile {
            settings,
            daily_ledgers,
            monthly_snapshots,
            ..empty_profile()
        })
        .unwrap();

        let result = build_finance_risk_cockpit_json(&raw, PERIOD_MONTH, 2026, 4, 10).unwrap();
        let snapshot = serde_json::from_str::<FinanceRiskCockpitSnapshot>(&result).unwrap();

        assert_eq!(100, snapshot.data_confidence);
        assert_ne!(FinanceRiskState::Unknown, snapshot.risk_state);
        assert_eq!(FINANCE_RISK_POLICY_VERSION, snapshot.policy_version);
        assert!(snapshot.uses_detailed_cashflow);
        assert!(snapshot.forecast_available);
        assert!(snapshot.balance_sheet_available);
        assert!(snapshot.has_fresh_snapshot);
        assert_eq!(2_400, snapshot.protected_allocation);
        assert_eq!(2_800, snapshot.safe_to_spend);
        assert_eq!(15_600, snapshot.projected_monthly_net);
        assert_eq!(27_600, snapshot.projected_balance30_days);
        assert_eq!(58_800, snapshot.projected_balance90_days);
        assert_eq!(19_440, snapshot.stressed_balance30_days);
        assert_eq!(12_000, snapshot.cash_reserve);
        assert_eq!(72_000, snapshot.emergency_target);
        assert_eq!(60_000, snapshot.emergency_gap);
        assert_eq!(Some(1.0), snapshot.emergency_months);
        assert_eq!(6_000, snapshot.debt_balance);
        assert_eq!(3_000, snapshot.monthly_debt_payment);
        assert_eq!(Some(2.0), snapshot.debt_free_months);
        assert_eq!(6, snapshot.budget_envelopes.len());
        assert_eq!(
            BUDGET_STATUS_BELOW_FLOOR,
            snapshot.budget_envelopes[2].status_code
        );
        assert!(snapshot.actions.len() <= 3);
        let allocation_action = snapshot
            .actions
            .iter()
            .find(|action| action.kind_code == ACTION_KIND_FILL_LONG_TERM_ALLOCATION)
            .unwrap();
        assert_eq!(BUCKET_CODE_LEARNING, allocation_action.bucket_code);
        assert_eq!(1_300, allocation_action.amount);
        assert_ne!(snapshot.protected_allocation, allocation_action.amount);
    }

    #[test]
    fn risk_cockpit_sparse_detailed_data_refuses_false_precision() {
        let mut daily_ledgers = IndexMap::new();
        daily_ledgers.insert(
            "2026-04-10".to_string(),
            FinanceDayLedger {
                incomes: vec![FinanceIncomeEntry {
                    name: "Income".to_string(),
                    amount: 1_000,
                    ..default_income_entry()
                }],
                expenses: vec![expense("Food", FinanceExpenseBucket::Food, 500)],
                ..FinanceDayLedger::default()
            },
        );
        let raw = serde_json::to_string(&FinanceProfile {
            daily_ledgers,
            ..empty_profile()
        })
        .unwrap();

        let result = build_finance_risk_cockpit_json(&raw, PERIOD_MONTH, 2026, 4, 10).unwrap();
        let snapshot = serde_json::from_str::<FinanceRiskCockpitSnapshot>(&result).unwrap();

        assert!(snapshot.uses_detailed_cashflow);
        assert!(!snapshot.forecast_available);
        assert_eq!(FinanceRiskState::Unknown, snapshot.risk_state);
        assert_eq!(REASON_CASHFLOW_COVERAGE_INSUFFICIENT, snapshot.reason_code);
        assert_eq!(8, snapshot.data_confidence);
        assert_eq!(0, snapshot.projected_monthly_net);
        assert_eq!(0, snapshot.projected_balance30_days);
        assert_eq!(0, snapshot.projected_balance90_days);
        assert_eq!(0, snapshot.stressed_balance30_days);
        assert!(snapshot
            .actions
            .iter()
            .any(|action| action.kind_code == ACTION_KIND_COMPLETE_DATA));
    }

    #[test]
    fn risk_cockpit_reserves_stable_monthly_bill_until_current_month_records_it() {
        let mut daily_ledgers = IndexMap::new();
        for (day_key, amount) in [
            ("2026-01-15", 100),
            ("2026-02-14", 105),
            ("2026-03-16", 100),
        ] {
            daily_ledgers.insert(
                day_key.to_string(),
                FinanceDayLedger {
                    expenses: vec![expense("Phone plan", FinanceExpenseBucket::Living, amount)],
                    ..FinanceDayLedger::default()
                },
            );
        }
        daily_ledgers.insert(
            "2026-04-01".to_string(),
            FinanceDayLedger {
                incomes: vec![FinanceIncomeEntry {
                    name: "Income".to_string(),
                    amount: 1_000,
                    ..default_income_entry()
                }],
                ..FinanceDayLedger::default()
            },
        );
        let raw = serde_json::to_string(&FinanceProfile {
            daily_ledgers: daily_ledgers.clone(),
            ..empty_profile()
        })
        .unwrap();

        let missing = build_finance_risk_cockpit_json(&raw, PERIOD_MONTH, 2026, 4, 10).unwrap();
        let missing = serde_json::from_str::<FinanceRiskCockpitSnapshot>(&missing).unwrap();
        assert_eq!(105, missing.pending_recurring_reserve);
        assert_eq!(625, missing.safe_to_spend);

        daily_ledgers.insert(
            "2026-04-08".to_string(),
            FinanceDayLedger {
                expenses: vec![expense(" phone   PLAN ", FinanceExpenseBucket::Living, 100)],
                ..FinanceDayLedger::default()
            },
        );
        let paid_raw = serde_json::to_string(&FinanceProfile {
            daily_ledgers,
            ..empty_profile()
        })
        .unwrap();
        let paid = build_finance_risk_cockpit_json(&paid_raw, PERIOD_MONTH, 2026, 4, 10).unwrap();
        let paid = serde_json::from_str::<FinanceRiskCockpitSnapshot>(&paid).unwrap();
        assert_eq!(0, paid.pending_recurring_reserve);
        assert_eq!(630, paid.safe_to_spend);
    }

    #[test]
    fn risk_cockpit_recurring_reserve_rejects_weak_history_and_target_overlap() {
        let mut weak_history = IndexMap::new();
        for (day_key, amount) in [("2026-01-15", 100), ("2026-02-15", 100)] {
            weak_history.insert(
                day_key.to_string(),
                FinanceDayLedger {
                    expenses: vec![expense("Phone plan", FinanceExpenseBucket::Living, amount)],
                    ..FinanceDayLedger::default()
                },
            );
        }
        weak_history.insert("2026-04-01".to_string(), income_ledger(1_000));
        let two_month_raw = serde_json::to_string(&FinanceProfile {
            daily_ledgers: weak_history.clone(),
            ..empty_profile()
        })
        .unwrap();
        let two_month =
            build_finance_risk_cockpit_json(&two_month_raw, PERIOD_MONTH, 2026, 4, 10).unwrap();
        assert_eq!(
            0,
            serde_json::from_str::<FinanceRiskCockpitSnapshot>(&two_month)
                .unwrap()
                .pending_recurring_reserve
        );

        weak_history.insert(
            "2026-03-15".to_string(),
            FinanceDayLedger {
                expenses: vec![expense("Phone plan", FinanceExpenseBucket::Living, 150)],
                ..FinanceDayLedger::default()
            },
        );
        let unstable_raw = serde_json::to_string(&FinanceProfile {
            daily_ledgers: weak_history,
            ..empty_profile()
        })
        .unwrap();
        let unstable =
            build_finance_risk_cockpit_json(&unstable_raw, PERIOD_MONTH, 2026, 4, 10).unwrap();
        assert_eq!(
            0,
            serde_json::from_str::<FinanceRiskCockpitSnapshot>(&unstable)
                .unwrap()
                .pending_recurring_reserve
        );

        let mut covered_history = IndexMap::new();
        for day_key in ["2026-01-15", "2026-02-15", "2026-03-15"] {
            covered_history.insert(
                day_key.to_string(),
                FinanceDayLedger {
                    expenses: vec![expense(
                        "Reserve transfer",
                        FinanceExpenseBucket::Other,
                        100,
                    )],
                    ..FinanceDayLedger::default()
                },
            );
        }
        covered_history.insert("2026-04-01".to_string(), income_ledger(1_000));
        let mut settings = FinanceSettings::default();
        settings
            .expense_categories
            .iter_mut()
            .find(|config| config.bucket == FinanceExpenseBucket::Other)
            .unwrap()
            .target_share_of_income = Some(0.10);
        let covered_raw = serde_json::to_string(&FinanceProfile {
            settings,
            daily_ledgers: covered_history,
            ..empty_profile()
        })
        .unwrap();
        let covered =
            build_finance_risk_cockpit_json(&covered_raw, PERIOD_MONTH, 2026, 4, 10).unwrap();
        let covered = serde_json::from_str::<FinanceRiskCockpitSnapshot>(&covered).unwrap();
        assert_eq!(370, covered.protected_allocation);
        assert_eq!(0, covered.pending_recurring_reserve);
        assert_eq!(630, covered.safe_to_spend);
    }

    #[test]
    fn risk_cockpit_legacy_estimates_support_budget_but_not_forecast() {
        let raw = serde_json::to_string(&FinanceProfile {
            active_income_monthly: 10_000,
            living_expense_monthly: 4_000,
            liability_payment_monthly: 1_000,
            cash_reserve: 20_000,
            liability_balance: 30_000,
            ..empty_profile()
        })
        .unwrap();

        let result = build_finance_risk_cockpit_json(&raw, PERIOD_MONTH, 2026, 4, 10).unwrap();
        let snapshot = serde_json::from_str::<FinanceRiskCockpitSnapshot>(&result).unwrap();

        assert!(!snapshot.uses_detailed_cashflow);
        assert!(!snapshot.forecast_available);
        assert!(!snapshot.balance_sheet_available);
        assert!(!snapshot.has_fresh_snapshot);
        assert_eq!(20, snapshot.data_confidence);
        assert_eq!(2_700, snapshot.protected_allocation);
        assert_eq!(2_300, snapshot.safe_to_spend);
        assert_eq!(0, snapshot.projected_monthly_net);
        assert_eq!(30_000, snapshot.emergency_target);
        assert_eq!(0, snapshot.cash_reserve);
        assert_eq!(0, snapshot.debt_balance);
        assert_eq!(1_000, snapshot.monthly_debt_payment);
        assert_eq!(None, snapshot.emergency_months);
        assert_eq!(None, snapshot.debt_free_months);
        assert_eq!(
            BUDGET_STATUS_NO_TARGET,
            snapshot.budget_envelopes[5].status_code
        );
    }

    #[test]
    fn risk_cockpit_detects_cross_day_recurring_expense_and_median_anomaly() {
        let samples = [
            ("2026-04-01", " Stream  Plan ", 100),
            ("2026-04-02", "stream plan", 100),
            ("2026-04-03", "Groceries", 100),
            ("2026-04-04", "Trip", 600),
        ];
        let mut daily_ledgers = IndexMap::new();
        for (day_key, name, amount) in samples {
            daily_ledgers.insert(
                day_key.to_string(),
                FinanceDayLedger {
                    expenses: vec![expense(name, FinanceExpenseBucket::Living, amount)],
                    ..FinanceDayLedger::default()
                },
            );
        }
        let raw = serde_json::to_string(&FinanceProfile {
            daily_ledgers,
            ..empty_profile()
        })
        .unwrap();

        let result = build_finance_risk_cockpit_json(&raw, PERIOD_MONTH, 2026, 4, 4).unwrap();
        let snapshot = serde_json::from_str::<FinanceRiskCockpitSnapshot>(&result).unwrap();

        assert!(snapshot.forecast_available);
        assert_eq!(1, snapshot.recurring_candidates.len());
        let recurring = &snapshot.recurring_candidates[0];
        assert_eq!("Stream Plan", recurring.name);
        assert_eq!(2, recurring.occurrences);
        assert_eq!(100, recurring.average_amount);
        assert_eq!(3_000, recurring.monthly_estimate);
        assert_eq!(3_000, snapshot.recurring_monthly_estimate);
        assert_eq!("2026-04-04", snapshot.anomaly_day_key);
        assert_eq!(600, snapshot.anomaly_amount);
        assert!((snapshot.anomaly_ratio - 6.0).abs() < 0.0001);
    }

    #[test]
    fn risk_cockpit_never_uses_snapshot_older_than_three_months_as_current_balance() {
        let mut daily_ledgers = IndexMap::new();
        for day in 1..=3 {
            daily_ledgers.insert(
                format!("2026-05-{day:02}"),
                FinanceDayLedger {
                    incomes: vec![FinanceIncomeEntry {
                        name: "Income".to_string(),
                        amount: 1_000,
                        ..default_income_entry()
                    }],
                    expenses: vec![expense("Living", FinanceExpenseBucket::Living, 200)],
                    ..FinanceDayLedger::default()
                },
            );
        }
        let mut monthly_snapshots = IndexMap::new();
        monthly_snapshots.insert("2026-01".to_string(), net_worth_snapshot(9_000, 4_500));
        let raw = serde_json::to_string(&FinanceProfile {
            daily_ledgers,
            monthly_snapshots,
            ..empty_profile()
        })
        .unwrap();

        let result = build_finance_risk_cockpit_json(&raw, PERIOD_MONTH, 2026, 5, 3).unwrap();
        let snapshot = serde_json::from_str::<FinanceRiskCockpitSnapshot>(&result).unwrap();

        assert!(snapshot.balance_sheet_available);
        assert!(!snapshot.has_fresh_snapshot);
        assert_eq!(0, snapshot.cash_reserve);
        assert_eq!(0, snapshot.debt_balance);
        assert_eq!(0, snapshot.projected_balance30_days);
        assert_eq!(0, snapshot.projected_balance90_days);
        assert_eq!(0, snapshot.stressed_balance30_days);
        assert_eq!(None, snapshot.emergency_months);
        assert_eq!(None, snapshot.debt_free_months);
        assert!(snapshot.actions.iter().any(|action| {
            action.kind_code == ACTION_KIND_REFRESH_MONTH_SNAPSHOT
                && action.severity_code == ACTION_SEVERITY_URGENT
        }));
    }

    #[test]
    fn risk_cockpit_treats_recent_prior_month_snapshot_as_history_not_current_balance() {
        let mut daily_ledgers = IndexMap::new();
        for day in 1..=3 {
            daily_ledgers.insert(
                format!("2026-05-{day:02}"),
                FinanceDayLedger {
                    incomes: vec![FinanceIncomeEntry {
                        name: "Income".to_string(),
                        amount: 1_000,
                        ..default_income_entry()
                    }],
                    expenses: vec![expense("Living", FinanceExpenseBucket::Living, 200)],
                    ..FinanceDayLedger::default()
                },
            );
        }
        let mut monthly_snapshots = IndexMap::new();
        monthly_snapshots.insert("2026-04".to_string(), net_worth_snapshot(9_000, 4_500));
        let raw = serde_json::to_string(&FinanceProfile {
            daily_ledgers,
            monthly_snapshots,
            ..empty_profile()
        })
        .unwrap();

        let result = build_finance_risk_cockpit_json(&raw, PERIOD_MONTH, 2026, 5, 3).unwrap();
        let snapshot = serde_json::from_str::<FinanceRiskCockpitSnapshot>(&result).unwrap();

        assert!(snapshot.forecast_available);
        assert!(snapshot.balance_sheet_available);
        assert!(!snapshot.has_fresh_snapshot);
        assert_eq!(85, snapshot.data_confidence);
        assert_eq!(0, snapshot.cash_reserve);
        assert_eq!(0, snapshot.debt_balance);
        assert_eq!(0, snapshot.projected_balance30_days);
        assert_eq!(0, snapshot.projected_balance90_days);
        assert_eq!(0, snapshot.stressed_balance30_days);
        assert_eq!(None, snapshot.emergency_months);
        assert_eq!(None, snapshot.debt_free_months);
        assert!(snapshot.actions.iter().any(|action| {
            action.kind_code == ACTION_KIND_REFRESH_MONTH_SNAPSHOT
                && action.severity_code == ACTION_SEVERITY_URGENT
        }));
    }

    #[test]
    fn risk_cockpit_json_uses_camel_case_and_explicit_nulls() {
        let result = build_finance_risk_cockpit_json("{}", PERIOD_MONTH, 2026, 4, 10).unwrap();
        let value = serde_json::from_str::<serde_json::Value>(&result).unwrap();

        assert!(value.get("dataConfidence").is_some());
        assert_eq!("UNKNOWN", value["riskState"]);
        assert_eq!(REASON_CASHFLOW_BASELINE_MISSING, value["reasonCode"]);
        assert_eq!(FINANCE_RISK_POLICY_VERSION, value["policyVersion"]);
        assert!(value.get("forecastAvailable").is_some());
        assert!(value.get("balanceSheetAvailable").is_some());
        assert!(value.get("safeToSpend").is_some());
        assert!(value.get("pendingRecurringReserve").is_some());
        assert!(value.get("data_confidence").is_none());
        assert!(value["emergencyMonths"].is_null());
        assert_eq!(6, value["budgetEnvelopes"].as_array().unwrap().len());
        assert!(value["budgetEnvelopes"][0].get("bucketCode").is_some());
        assert!(value["actions"][0].get("severityCode").is_some());
        assert!(value["actions"][0].get("kindCode").is_some());
    }

    fn android_v2_receipt(profile: &FinanceProfile, month_key: &str) -> FinanceReviewReceipt {
        let fp = android_finance_review_fingerprints(profile, month_key).unwrap();
        let (year, month) = parse_finance_month_key(month_key).unwrap();
        let recorded_days = profile
            .aggregate_for_period(PERIOD_MONTH, year, month, days_in_month(year, month))
            .days_with_entries;
        let income_expense_fingerprint = if recorded_days == 0 {
            format!("zero-full:{}", fp.income_expense_fingerprint)
        } else {
            format!("full:{}", fp.income_expense_fingerprint)
        };
        FinanceReviewReceipt {
            month_key: month_key.to_string(),
            income_expense_fingerprint,
            cash_fingerprint: fp.cash_fingerprint,
            assets_fingerprint: fp.assets_fingerprint,
            liabilities_fingerprint: fp.liabilities_fingerprint,
            income_expense_verified: true,
            cash_verified: true,
            assets_verified: true,
            liabilities_verified: true,
            zero_cash_confirmed: false,
            zero_assets_confirmed: false,
            zero_liabilities_confirmed: false,
            confirmed_recurring_keys: Vec::new(),
        }
    }

    #[test]
    fn android_v2_asset_only_and_sparse_wage_remain_unreviewed() {
        let mut profile = empty_profile();
        profile
            .daily_ledgers
            .insert("2026-04-01".to_string(), income_ledger(3_000));
        profile
            .monthly_snapshots
            .insert("2026-04".to_string(), net_worth_snapshot(5_000, 0));
        let profile = profile.sanitized();
        let result = android_finance_risk_v2(&profile, &[], 2026, 4, 1).unwrap();
        assert_eq!(3_000, result.recorded_income);
        assert_eq!(Some(5_000), result.recorded_assets);
        assert_eq!(None, result.recorded_liabilities);
        assert_eq!(None, result.safe_to_spend);
        assert_eq!(None, result.projected_balance30_days);
        assert_eq!(FinanceRiskState::Unknown, result.risk_state);
        assert!(!result.forecast_available);
    }

    #[test]
    fn android_v2_three_reviewed_months_do_not_expand_first_day_wage() {
        let mut profile = empty_profile();
        for month in 1..=3 {
            profile
                .daily_ledgers
                .insert(format!("2026-{month:02}-01"), income_ledger(3_000));
        }
        profile
            .daily_ledgers
            .insert("2026-04-01".to_string(), income_ledger(3_000));
        profile
            .monthly_snapshots
            .insert("2026-04".to_string(), net_worth_snapshot(5_000, 0));
        let profile = profile.sanitized();
        let mut receipts = (1..=4)
            .map(|month| android_v2_receipt(&profile, &format!("2026-{month:02}")))
            .collect::<Vec<_>>();
        let incomplete = android_finance_risk_v2(&profile, &receipts, 2026, 4, 1).unwrap();
        assert!(incomplete.forecast_available);
        assert_eq!(None, incomplete.projected_balance30_days);
        assert_eq!(None, incomplete.safe_to_spend);
        receipts[3].zero_liabilities_confirmed = true;
        let result = android_finance_risk_v2(&profile, &receipts, 2026, 4, 1).unwrap();
        assert!(result.forecast_available);
        assert_eq!(Some(3_000), result.monthly_income_baseline);
        assert_eq!(Some(3_000), result.monthly_net_baseline);
        assert_eq!(Some(8_000), result.projected_balance30_days);
        assert_eq!(Some(0), result.verified_liabilities);
        assert!(result.safe_to_spend.is_some());
        assert_ne!(FinanceRiskState::Unknown, result.risk_state);
    }

    #[test]
    fn android_v2_receipt_invalidates_only_edited_lane_and_requires_explicit_zero_debt() {
        let mut profile = empty_profile();
        profile
            .monthly_snapshots
            .insert("2026-04".to_string(), net_worth_snapshot(5_000, 0));
        let profile = profile.sanitized();
        let receipt = android_v2_receipt(&profile, "2026-04");
        let before = android_finance_risk_v2(&profile, &[receipt.clone()], 2026, 4, 10).unwrap();
        assert_eq!(None, before.verified_liabilities);
        let with_zero = FinanceReviewReceipt {
            zero_liabilities_confirmed: true,
            ..receipt.clone()
        };
        assert_eq!(
            Some(0),
            android_finance_risk_v2(&profile, &[with_zero.clone()], 2026, 4, 10)
                .unwrap()
                .verified_liabilities
        );
        let mut changed = profile.clone();
        changed
            .daily_ledgers
            .insert("2026-04-09".to_string(), income_ledger(100));
        let changed = changed.sanitized();
        let result = android_finance_risk_v2(&changed, &[with_zero], 2026, 4, 10).unwrap();
        assert!(!result.income_expense_verified);
        assert_eq!(Some(5_000), result.verified_cash);
        assert_eq!(Some(0), result.verified_liabilities);
    }

    #[test]
    fn android_v2_safe_amount_is_capped_by_reviewed_cash_and_keeps_the_gap_signed() {
        let mut profile = empty_profile();
        for month in 1..=3 {
            profile.daily_ledgers.insert(
                format!("2026-{month:02}-05"),
                FinanceDayLedger {
                    incomes: income_ledger(3_000).incomes,
                    expenses: vec![expense("Food", FinanceExpenseBucket::Food, 1_000)],
                    ..FinanceDayLedger::default()
                },
            );
        }
        profile
            .monthly_snapshots
            .insert("2026-04".to_string(), net_worth_snapshot(400, 0));
        let profile = profile.sanitized();
        let mut receipts = (1..=4)
            .map(|month| android_v2_receipt(&profile, &format!("2026-{month:02}")))
            .collect::<Vec<_>>();
        receipts[3].zero_liabilities_confirmed = true;
        let result = android_finance_risk_v2(&profile, &receipts, 2026, 4, 10).unwrap();
        assert_eq!(Some(400), result.verified_cash);
        assert_eq!(Some(1_000), result.pending_essential_reserve);
        assert!(result.safe_to_spend.unwrap() <= -600);
        assert_eq!(FinanceRiskState::Tight, result.risk_state);
    }

    #[test]
    fn android_v2_unpaid_confirmed_rent_survives_unrelated_living_payment() {
        let mut profile = empty_profile();
        for month in 1..=3 {
            profile.daily_ledgers.insert(
                format!("2026-{month:02}-05"),
                FinanceDayLedger {
                    expenses: vec![expense("Rent", FinanceExpenseBucket::Living, 1_000)],
                    ..FinanceDayLedger::default()
                },
            );
        }
        profile.daily_ledgers.insert(
            "2026-04-01".to_string(),
            FinanceDayLedger {
                expenses: vec![expense("Groceries", FinanceExpenseBucket::Living, 1_000)],
                ..FinanceDayLedger::default()
            },
        );
        profile
            .monthly_snapshots
            .insert("2026-04".to_string(), net_worth_snapshot(2_000, 0));
        let profile = profile.sanitized();
        let mut receipts = (1..=4)
            .map(|month| android_v2_receipt(&profile, &format!("2026-{month:02}")))
            .collect::<Vec<_>>();
        receipts[3].confirmed_recurring_keys = vec!["3:rent".to_string()];
        receipts[3].zero_liabilities_confirmed = true;
        let result = android_finance_risk_v2(&profile, &receipts, 2026, 4, 10).unwrap();
        assert_eq!(1_000, result.recorded_outflow);
        assert_eq!(1, result.recurring_expenses.len());
        assert!(result.recurring_expenses[0].confirmed);
        assert!(!result.recurring_expenses[0].paid_this_month);
        assert_eq!(Some(1_000), result.pending_essential_reserve);
        assert!(result.safe_to_spend.unwrap() <= 1_000);
    }

    #[test]
    fn android_v2_empty_month_requires_explicit_full_zero_review_for_baseline() {
        let mut profile = empty_profile();
        for month in 1..=2 {
            profile
                .daily_ledgers
                .insert(format!("2026-{month:02}-05"), income_ledger(1_000));
        }
        let profile = profile.sanitized();
        let mut receipts = (1..=3)
            .map(|month| android_v2_receipt(&profile, &format!("2026-{month:02}")))
            .collect::<Vec<_>>();
        let zero_fp = android_finance_review_fingerprints(&profile, "2026-03")
            .unwrap()
            .income_expense_fingerprint;
        receipts[2].income_expense_fingerprint = zero_fp.clone();
        assert!(
            !android_finance_risk_v2(&profile, &receipts, 2026, 4, 10)
                .unwrap()
                .forecast_available
        );
        receipts[2].income_expense_fingerprint = format!("through:31:{zero_fp}");
        assert!(
            !android_finance_risk_v2(&profile, &receipts, 2026, 4, 10)
                .unwrap()
                .forecast_available
        );
        receipts[2].income_expense_fingerprint = format!("zero-full:{zero_fp}");
        let result = android_finance_risk_v2(&profile, &receipts, 2026, 4, 10).unwrap();
        assert!(result.forecast_available);
        assert!(result.baseline_months.contains(&"2026-03".to_string()));
    }

    #[test]
    fn android_v2_two_adjacent_payments_are_not_recurring_but_same_day_duplicate_is_flagged() {
        let mut profile = empty_profile();
        for (day, id) in [("2026-04-03", "a"), ("2026-04-04", "b")] {
            profile.daily_ledgers.insert(
                day.to_string(),
                FinanceDayLedger {
                    expenses: vec![FinanceExpenseEntry {
                        id: id.to_string(),
                        name: "Streaming".to_string(),
                        bucket: FinanceExpenseBucket::Living,
                        amount: 100,
                        ..default_expense_entry()
                    }],
                    ..FinanceDayLedger::default()
                },
            );
        }
        let profile = profile.sanitized();
        let result = android_finance_risk_v2(&profile, &[], 2026, 4, 4).unwrap();
        assert!(result.recurring_expenses.is_empty());
        assert!(result.duplicate_payments.is_empty());
        let mut duplicated = profile.clone();
        duplicated
            .daily_ledgers
            .get_mut("2026-04-04")
            .unwrap()
            .expenses
            .push(FinanceExpenseEntry {
                id: "c".to_string(),
                name: "Streaming".to_string(),
                bucket: FinanceExpenseBucket::Living,
                amount: 100,
                ..default_expense_entry()
            });
        let result = android_finance_risk_v2(&duplicated.sanitized(), &[], 2026, 4, 4).unwrap();
        assert_eq!(1, result.duplicate_payments.len());
        assert_eq!(2, result.duplicate_payments[0].entry_ids.len());
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
            legacy_amount_minor: IndexMap::new(),
            calculation_money_unit: None,
            acquisition_focus: String::new(),
            liability_focus: String::new(),
            settings: FinanceSettings::default(),
            daily_ledgers: IndexMap::new(),
            monthly_snapshots: IndexMap::new(),
        }
    }

    fn income_ledger(amount: i64) -> FinanceDayLedger {
        FinanceDayLedger {
            incomes: vec![FinanceIncomeEntry {
                name: "Income".to_string(),
                amount,
                ..default_income_entry()
            }],
            ..FinanceDayLedger::default()
        }
    }

    fn healthy_cashflow_ledger() -> FinanceDayLedger {
        FinanceDayLedger {
            incomes: vec![FinanceIncomeEntry {
                name: "Income".to_string(),
                amount: 10_000,
                ..default_income_entry()
            }],
            expenses: vec![FinanceExpenseEntry {
                name: "Living".to_string(),
                bucket: FinanceExpenseBucket::Living,
                amount: 1_000,
                ..default_expense_entry()
            }],
            ..FinanceDayLedger::default()
        }
    }

    fn allocation_ledger(
        debt: i64,
        food: i64,
        btc: i64,
        living: i64,
        learning: i64,
        other: i64,
    ) -> FinanceDayLedger {
        let expenses = [
            (FinanceExpenseBucket::Debt, debt),
            (FinanceExpenseBucket::Food, food),
            (FinanceExpenseBucket::Btc, btc),
            (FinanceExpenseBucket::Living, living),
            (FinanceExpenseBucket::Learning, learning),
            (FinanceExpenseBucket::Other, other),
        ]
        .into_iter()
        .filter(|(_, amount)| *amount > 0)
        .map(|(bucket, amount)| FinanceExpenseEntry {
            name: format!("{bucket:?}"),
            bucket,
            amount,
            ..default_expense_entry()
        })
        .collect();
        FinanceDayLedger {
            incomes: vec![FinanceIncomeEntry {
                name: "Income".to_string(),
                amount: 1_000,
                ..default_income_entry()
            }],
            expenses,
            ..FinanceDayLedger::default()
        }
    }

    fn net_worth_snapshot(asset: i64, liability: i64) -> FinanceMonthSnapshot {
        FinanceMonthSnapshot {
            assets: if asset > 0 {
                vec![FinanceNamedAmountEntry {
                    name: "Cash".to_string(),
                    kind: FinanceNamedAmountKind::CashReserve,
                    amount: asset,
                    ..default_named_amount_entry()
                }]
            } else {
                Vec::new()
            },
            liabilities: if liability > 0 {
                vec![FinanceNamedAmountEntry {
                    name: "Debt".to_string(),
                    kind: FinanceNamedAmountKind::LiabilityBalance,
                    amount: liability,
                    ..default_named_amount_entry()
                }]
            } else {
                Vec::new()
            },
            ..FinanceMonthSnapshot::default()
        }
    }

    fn default_income_entry() -> FinanceIncomeEntry {
        FinanceIncomeEntry {
            id: String::new(),
            updated_at_epoch_millis: 0,
            deleted_at_epoch_millis: 0,
            name: String::new(),
            kind: FinanceIncomeKind::Active,
            amount: 0,
            amount_minor: None,
            note: String::new(),
        }
    }

    fn expense(name: &str, bucket: FinanceExpenseBucket, amount: i64) -> FinanceExpenseEntry {
        FinanceExpenseEntry {
            name: name.to_string(),
            bucket,
            amount,
            ..default_expense_entry()
        }
    }

    fn default_expense_entry() -> FinanceExpenseEntry {
        FinanceExpenseEntry {
            id: String::new(),
            updated_at_epoch_millis: 0,
            deleted_at_epoch_millis: 0,
            name: String::new(),
            bucket: FinanceExpenseBucket::Other,
            amount: 0,
            amount_minor: None,
            note: String::new(),
        }
    }

    fn default_named_amount_entry() -> FinanceNamedAmountEntry {
        FinanceNamedAmountEntry {
            id: String::new(),
            updated_at_epoch_millis: 0,
            deleted_at_epoch_millis: 0,
            name: String::new(),
            kind: FinanceNamedAmountKind::OtherAsset,
            amount: 0,
            amount_minor: None,
        }
    }
}
