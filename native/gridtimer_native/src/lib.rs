use jni::objects::{
    JBooleanArray, JClass, JFloatArray, JIntArray, JLongArray, JObject, JObjectArray, JString,
};
use jni::sys::{
    jboolean, jbooleanArray, jdoubleArray, jfloat, jint, jintArray, jlong, jlongArray,
    jobjectArray, jstring, JNI_FALSE, JNI_TRUE,
};
use jni::JNIEnv;
use pulldown_cmark::{html, Options, Parser};
use serde::Deserialize;
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};

pub mod ai_client;
pub mod app_data;
mod finance_profile;
mod note_documents;
pub mod sync_core;
mod timer_insights;
pub mod tooling;

use note_documents::{build_note_document_text_digest, NoteBlockTextInput, NoteDocumentTextInput};

const CENTER_PREFIX: &str = "【居中】";
const CENTER_WRAP_START: &str = "[";
const CENTER_WRAP_END: &str = "]";
const HEADING_PREFIX: &str = "# ";
const LIST_PREFIX: &str = "- ";
const QUOTE_PREFIX: &str = "> ";
const TODO_PREFIX: &str = "- [ ] ";
const BOLD_MARK: &str = "**";

const MIUI_FOCUS_TIMER_PIC_KEY: &str = "miui.focus.pic_timer";
const MIUI_FOCUS_TIMER_PIC_DARK_KEY: &str = "miui.focus.pic_timer_dark";
const MIUI_FOCUS_OPEN_ACTION_KEY: &str = "miui.focus.action_open_timer";
const MIUI_FOCUS_PAUSE_ACTION_KEY: &str = "miui.focus.action_pause_timer";
const MIUI_ISLAND_HIGHLIGHT_COLOR: &str = "#C0392B";
const MICRO_BREAK_FOCUS_MIN_MILLIS: i64 = 3 * 60 * 1_000;
const MICRO_BREAK_FOCUS_MAX_MILLIS: i64 = 5 * 60 * 1_000;
const MICRO_BREAK_FOCUS_STEP_MILLIS: i64 = 15_000;
const MICRO_BREAK_FOCUS_VARIANT_COUNT: i64 =
    ((MICRO_BREAK_FOCUS_MAX_MILLIS - MICRO_BREAK_FOCUS_MIN_MILLIS) / MICRO_BREAK_FOCUS_STEP_MILLIS)
        + 1;
const MICRO_BREAK_REST_MILLIS: i64 = 15_000;
const MAX_TRACKED_DURATION_MILLIS: i64 = 10 * 365 * 24 * 60 * 60 * 1_000;
const MICRO_BREAK_PHASE_FOCUS: i32 = 0;
const MICRO_BREAK_PHASE_BREAK: i32 = 1;
const MICRO_BREAK_TRANSITION_BREAK_STARTED: i64 = 0;
const MICRO_BREAK_TRANSITION_FOCUS_RESUMED: i64 = 1;
const FINANCE_INCOME_KIND_ACTIVE: i32 = 0;
const FINANCE_INCOME_KIND_ASSET: i32 = 1;
const FINANCE_INCOME_KIND_OTHER: i32 = 2;
const FINANCE_BUCKET_DEBT: i32 = 0;
const FINANCE_BUCKET_FOOD: i32 = 1;
const FINANCE_BUCKET_BTC: i32 = 2;
const FINANCE_BUCKET_LIVING: i32 = 3;
const FINANCE_BUCKET_LEARNING: i32 = 4;
const FINANCE_BUCKET_OTHER: i32 = 5;
const FINANCE_NAMED_AMOUNT_CASH_RESERVE: i32 = 0;
const FINANCE_NAMED_AMOUNT_PRODUCTIVE_ASSET: i32 = 1;
const FINANCE_NAMED_AMOUNT_OTHER_ASSET: i32 = 2;
const FINANCE_NAMED_AMOUNT_LIABILITY_BALANCE: i32 = 3;
const FINANCE_NAMED_AMOUNT_OTHER_LIABILITY: i32 = 4;

#[derive(Clone, Copy)]
enum NoteTextAction {
    Heading,
    Center,
    BulletList,
    Bold,
    Quote,
    Todo,
}

struct NoteTextEdit {
    content: String,
    selection_start_utf16: usize,
    selection_end_utf16: usize,
}

struct XiaomiPayloadState {
    title: String,
    text: String,
    big_text: String,
    sub_text: String,
    primary_title: String,
    primary_elapsed: String,
    share_content: String,
    running_count: i32,
}

#[derive(Clone, Copy)]
struct NativePoint {
    x: f32,
    y: f32,
}

#[derive(Clone, Copy)]
struct NativeRect {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

#[derive(Clone, Copy)]
struct TimerSlotDropTargetCandidate {
    slot_id: i32,
    bounds: NativeRect,
}

#[derive(Clone, Copy)]
struct TimerSlotInsertCandidate {
    slot_index: i32,
    bounds: NativeRect,
}

struct TimerSlotInsertIndexStabilizationResult {
    insert_index: i32,
    pending_insert_index: Option<i32>,
    pending_insert_index_since_uptime_ms: i64,
}

#[derive(Clone, Copy)]
struct FinanceLedgerAggregateNative {
    active_income_total: i64,
    asset_income_total: i64,
    other_income_total: i64,
    debt_total: i64,
    food_total: i64,
    btc_total: i64,
    living_total: i64,
    learning_total: i64,
    other_expense_total: i64,
    days_with_entries: i64,
}

#[derive(Clone, Copy)]
struct NativeMicroBreakSession {
    started_at_epoch_millis: i64,
    ended_at_epoch_millis: i64,
    duration_millis: i64,
}

#[derive(Clone, Copy)]
struct NativeMicroBreakTransition {
    transition_type: i64,
    occurred_at_epoch_millis: i64,
}

struct NativeMicroBreakResolution {
    accumulated_millis: i64,
    running_since_epoch_millis: i64,
    phase: i32,
    cycle_index: i32,
    phase_progress_millis: i64,
    updated_at: i64,
    sessions: Vec<NativeMicroBreakSession>,
    transitions: Vec<NativeMicroBreakTransition>,
}

impl FinanceLedgerAggregateNative {
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

    fn add_expense(&mut self, bucket_code: i32, amount: i64) {
        if amount <= 0 {
            return;
        }
        match bucket_code {
            FINANCE_BUCKET_DEBT => self.debt_total = self.debt_total.saturating_add(amount),
            FINANCE_BUCKET_FOOD => self.food_total = self.food_total.saturating_add(amount),
            FINANCE_BUCKET_BTC => self.btc_total = self.btc_total.saturating_add(amount),
            FINANCE_BUCKET_LIVING => self.living_total = self.living_total.saturating_add(amount),
            FINANCE_BUCKET_LEARNING => {
                self.learning_total = self.learning_total.saturating_add(amount)
            }
            FINANCE_BUCKET_OTHER => {
                self.other_expense_total = self.other_expense_total.saturating_add(amount)
            }
            _ => {}
        }
    }
}

#[derive(Deserialize)]
struct NoteHtmlPayload {
    title: String,
    meta: String,
    accent_seed: String,
    markdown_enabled: bool,
    rich_text_enabled: bool,
    blocks: Vec<NoteHtmlBlock>,
}

#[derive(Deserialize)]
struct NoteHtmlBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: Option<String>,
    attachment_id: Option<String>,
    caption: Option<String>,
    contact_name: Option<String>,
    contact_organization: Option<String>,
    contact_phones: Option<Vec<NoteHtmlPhone>>,
    call_phone_number: Option<String>,
    call_contact_name: Option<String>,
    call_direction: Option<String>,
    call_occurred_at_label: Option<String>,
    call_duration_label: Option<String>,
}

#[derive(Deserialize)]
struct NoteHtmlPhone {
    label: Option<String>,
    number: Option<String>,
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeBatchMatchHistoryQuery(
    mut env: JNIEnv,
    _class: JClass,
    query: JString,
    haystacks: JObjectArray,
) -> jbooleanArray {
    let query: String = match env.get_string(&query) {
        Ok(value) => value.into(),
        Err(_) => return empty_boolean_array(&mut env),
    };
    let keywords = normalize_query(&query);
    let haystack_count = match env.get_array_length(&haystacks) {
        Ok(value) => value,
        Err(_) => return empty_boolean_array(&mut env),
    };
    let result = match env.new_boolean_array(haystack_count) {
        Ok(array) => array,
        Err(_) => return std::ptr::null_mut(),
    };

    let mut matches = Vec::<jboolean>::with_capacity(haystack_count as usize);
    if keywords.is_empty() {
        matches.resize(haystack_count as usize, JNI_TRUE);
    } else {
        for index in 0..haystack_count {
            let haystack = env
                .get_object_array_element(&haystacks, index)
                .ok()
                .and_then(|value| {
                    let value = JString::from(value);
                    env.get_string(&value).ok().map(String::from)
                })
                .unwrap_or_default();
            matches.push(boolean_to_jni(history_query_matches(&haystack, &keywords)));
        }
    }

    if env.set_boolean_array_region(&result, 0, &matches).is_err() {
        return std::ptr::null_mut();
    }
    result.into_raw()
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeComputeFinanceSnapshot(
    env: JNIEnv,
    _class: JClass,
    active_income_monthly: jlong,
    asset_income_monthly: jlong,
    living_expense_monthly: jlong,
    liability_payment_monthly: jlong,
    cash_reserve: jlong,
    productive_asset_value: jlong,
    liability_balance: jlong,
) -> jdoubleArray {
    let snapshot = compute_finance_snapshot(
        active_income_monthly,
        asset_income_monthly,
        living_expense_monthly,
        liability_payment_monthly,
        cash_reserve,
        productive_asset_value,
        liability_balance,
    );
    double_array_from_slice(env, &snapshot)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeComputeFinanceReportSnapshot(
    env: JNIEnv,
    _class: JClass,
    active_income_monthly: jlong,
    asset_income_monthly: jlong,
    living_expense_monthly: jlong,
    liability_payment_monthly: jlong,
    cash_reserve: jlong,
    productive_asset_value: jlong,
    liability_balance: jlong,
    period_code: jint,
) -> jdoubleArray {
    let snapshot = compute_finance_snapshot(
        active_income_monthly,
        asset_income_monthly,
        living_expense_monthly,
        liability_payment_monthly,
        cash_reserve,
        productive_asset_value,
        liability_balance,
    );
    let period_scale = finance_period_scale(period_code);
    let report = [
        scale_unsigned_amount(snapshot[0], period_scale),
        scale_unsigned_amount(snapshot[1], period_scale),
        scale_signed_amount(snapshot[2], period_scale),
        scale_unsigned_amount(snapshot[3], period_scale),
        snapshot[4],
        snapshot[5],
        snapshot[6],
        snapshot[7],
        scale_defensive_coverage(snapshot[8], period_scale),
        snapshot[9] * period_scale,
    ];
    double_array_from_slice(env, &report)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeEvaluateFinanceLedgerHint(
    _env: JNIEnv,
    _class: JClass,
    period_code: jint,
    income_total: jlong,
    expense_total: jlong,
    debt_total: jlong,
    food_total: jlong,
    btc_total: jlong,
    living_total: jlong,
    learning_total: jlong,
    _days_with_entries: jint,
    net_cashflow: jlong,
    opening_net_worth: jlong,
    closing_net_worth: jlong,
) -> jint {
    compute_finance_ledger_hint(
        period_code,
        income_total,
        expense_total,
        debt_total,
        food_total,
        btc_total,
        living_total,
        learning_total,
        net_cashflow,
        opening_net_worth,
        closing_net_worth,
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeEvaluateFinanceLedgerHintWithTargets(
    env: JNIEnv,
    _class: JClass,
    period_code: jint,
    income_total: jlong,
    expense_total: jlong,
    debt_total: jlong,
    food_total: jlong,
    btc_total: jlong,
    living_total: jlong,
    learning_total: jlong,
    _days_with_entries: jint,
    net_cashflow: jlong,
    opening_net_worth: jlong,
    closing_net_worth: jlong,
    target_shares: JFloatArray,
) -> jint {
    let Some(target_values) = float_array_values(&env, &target_shares) else {
        return compute_finance_ledger_hint(
            period_code,
            income_total,
            expense_total,
            debt_total,
            food_total,
            btc_total,
            living_total,
            learning_total,
            net_cashflow,
            opening_net_worth,
            closing_net_worth,
        );
    };
    compute_finance_ledger_hint_with_targets(
        period_code,
        income_total,
        expense_total,
        debt_total,
        food_total,
        btc_total,
        living_total,
        learning_total,
        net_cashflow,
        opening_net_worth,
        closing_net_worth,
        &target_values,
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeAggregateFinanceLedgers(
    env: JNIEnv,
    _class: JClass,
    period_code: jint,
    reference_year: jint,
    reference_month: jint,
    reference_day: jint,
    ledger_day_codes: JIntArray,
    ledger_note_flags: JIntArray,
    income_day_codes: JIntArray,
    income_kind_codes: JIntArray,
    income_amounts: JLongArray,
    expense_day_codes: JIntArray,
    expense_bucket_codes: JIntArray,
    expense_amounts: JLongArray,
) -> jlongArray {
    let Some(ledger_day_values) = int_array_values(&env, &ledger_day_codes) else {
        return long_array_from_slice(env, &[]);
    };
    let Some(ledger_note_values) = int_array_values(&env, &ledger_note_flags) else {
        return long_array_from_slice(env, &[]);
    };
    let Some(income_day_values) = int_array_values(&env, &income_day_codes) else {
        return long_array_from_slice(env, &[]);
    };
    let Some(income_kind_values) = int_array_values(&env, &income_kind_codes) else {
        return long_array_from_slice(env, &[]);
    };
    let Some(income_amount_values) = long_array_values(&env, &income_amounts) else {
        return long_array_from_slice(env, &[]);
    };
    let Some(expense_day_values) = int_array_values(&env, &expense_day_codes) else {
        return long_array_from_slice(env, &[]);
    };
    let Some(expense_bucket_values) = int_array_values(&env, &expense_bucket_codes) else {
        return long_array_from_slice(env, &[]);
    };
    let Some(expense_amount_values) = long_array_values(&env, &expense_amounts) else {
        return long_array_from_slice(env, &[]);
    };

    let Some(aggregate) = aggregate_finance_ledgers(
        period_code,
        reference_year,
        reference_month,
        reference_day,
        &ledger_day_values,
        &ledger_note_values,
        &income_day_values,
        &income_kind_values,
        &income_amount_values,
        &expense_day_values,
        &expense_bucket_values,
        &expense_amount_values,
    ) else {
        return long_array_from_slice(env, &[]);
    };
    long_array_from_slice(env, &finance_ledger_aggregate_values(aggregate))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeAggregateFinanceDayLedger(
    env: JNIEnv,
    _class: JClass,
    income_kind_codes: JIntArray,
    income_amounts: JLongArray,
    expense_bucket_codes: JIntArray,
    expense_amounts: JLongArray,
    has_note: jboolean,
) -> jlongArray {
    let Some(income_kind_values) = int_array_values(&env, &income_kind_codes) else {
        return long_array_from_slice(env, &[]);
    };
    let Some(income_amount_values) = long_array_values(&env, &income_amounts) else {
        return long_array_from_slice(env, &[]);
    };
    let Some(expense_bucket_values) = int_array_values(&env, &expense_bucket_codes) else {
        return long_array_from_slice(env, &[]);
    };
    let Some(expense_amount_values) = long_array_values(&env, &expense_amounts) else {
        return long_array_from_slice(env, &[]);
    };
    let Some(values) = aggregate_finance_day_ledger_values(
        &income_kind_values,
        &income_amount_values,
        &expense_bucket_values,
        &expense_amount_values,
        has_note == JNI_TRUE,
    ) else {
        return long_array_from_slice(env, &[]);
    };
    long_array_from_slice(env, &values)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeSummarizeFinanceMonthSnapshot(
    env: JNIEnv,
    _class: JClass,
    asset_kind_codes: JIntArray,
    asset_amounts: JLongArray,
    liability_amounts: JLongArray,
) -> jlongArray {
    let Some(asset_kind_values) = int_array_values(&env, &asset_kind_codes) else {
        return long_array_from_slice(env, &[]);
    };
    let Some(asset_amount_values) = long_array_values(&env, &asset_amounts) else {
        return long_array_from_slice(env, &[]);
    };
    let Some(liability_amount_values) = long_array_values(&env, &liability_amounts) else {
        return long_array_from_slice(env, &[]);
    };
    let Some(summary) = summarize_finance_month_snapshot(
        &asset_kind_values,
        &asset_amount_values,
        &liability_amount_values,
    ) else {
        return long_array_from_slice(env, &[]);
    };
    long_array_from_slice(env, &summary)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeRecentFinanceTemplateIndices(
    mut env: JNIEnv,
    _class: JClass,
    day_codes: JIntArray,
    kind_codes: JIntArray,
    names: JObjectArray,
    limit: jint,
) -> jintArray {
    let Some(day_code_values) = int_array_values(&env, &day_codes) else {
        return empty_int_array(&mut env);
    };
    let Some(kind_code_values) = int_array_values(&env, &kind_codes) else {
        return empty_int_array(&mut env);
    };
    let name_count = match env.get_array_length(&names) {
        Ok(value) => value,
        Err(_) => return empty_int_array(&mut env),
    };
    if day_code_values.len() != kind_code_values.len()
        || day_code_values.len() != name_count as usize
    {
        return empty_int_array(&mut env);
    }

    let mut name_values = Vec::<String>::with_capacity(name_count as usize);
    for index in 0..name_count {
        name_values.push(object_array_string_at(&mut env, &names, index).unwrap_or_default());
    }
    let indices = recent_finance_template_indices(
        &day_code_values,
        &kind_code_values,
        &name_values,
        limit.max(0) as usize,
    );
    int_array_from_slice(env, &indices)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFinanceMissingTemplateIndices(
    mut env: JNIEnv,
    _class: JClass,
    row_kind_codes: JIntArray,
    row_names: JObjectArray,
    template_kind_codes: JIntArray,
    template_names: JObjectArray,
) -> jintArray {
    let row_kind_codes = match int_array_values(&env, &row_kind_codes) {
        Some(values) => values,
        None => return empty_int_array(&mut env),
    };
    let row_names = match string_array_values(&mut env, &row_names) {
        Some(values) => values,
        None => return empty_int_array(&mut env),
    };
    let template_kind_codes = match int_array_values(&env, &template_kind_codes) {
        Some(values) => values,
        None => return empty_int_array(&mut env),
    };
    let template_names = match string_array_values(&mut env, &template_names) {
        Some(values) => values,
        None => return empty_int_array(&mut env),
    };
    let indices = finance_missing_template_indices(
        &row_kind_codes,
        &row_names,
        &template_kind_codes,
        &template_names,
    );
    int_array_from_slice(env, &indices)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeAppendMissingFinanceIncomeTemplatesJson(
    mut env: JNIEnv,
    _class: JClass,
    rows_json: JString,
    templates_json: JString,
) -> jstring {
    let rows_json: String = match env.get_string(&rows_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let templates_json: String = match env.get_string(&templates_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match finance_profile::append_missing_income_templates_json(&rows_json, &templates_json) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeAppendMissingFinanceExpenseTemplatesJson(
    mut env: JNIEnv,
    _class: JClass,
    rows_json: JString,
    templates_json: JString,
) -> jstring {
    let rows_json: String = match env.get_string(&rows_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let templates_json: String = match env.get_string(&templates_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match finance_profile::append_missing_expense_templates_json(&rows_json, &templates_json) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeAppendMissingFinanceNamedAmountTemplatesJson(
    mut env: JNIEnv,
    _class: JClass,
    rows_json: JString,
    templates_json: JString,
) -> jstring {
    let rows_json: String = match env.get_string(&rows_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let templates_json: String = match env.get_string(&templates_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match finance_profile::append_missing_named_amount_templates_json(&rows_json, &templates_json) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFinanceRowEditJson(
    mut env: JNIEnv,
    _class: JClass,
    row_kind_code: jint,
    operation_code: jint,
    rows_json: JString,
    index: jint,
    entry_json: JString,
) -> jstring {
    let rows_json: String = match env.get_string(&rows_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let entry_json: String = match env.get_string(&entry_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match finance_profile::finance_row_edit_json(
        row_kind_code,
        operation_code,
        &rows_json,
        index,
        &entry_json,
    ) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFilterHistoryArchived(
    mut env: JNIEnv,
    _class: JClass,
    query: JString,
    filter_category_id: JString,
    filter_slot_id: jint,
    category_ids: JObjectArray,
    slot_ids: JIntArray,
    haystacks: JObjectArray,
) -> jintArray {
    let query: String = match env.get_string(&query) {
        Ok(value) => value.into(),
        Err(_) => return empty_int_array(&mut env),
    };
    let filter_category_id: String = match env.get_string(&filter_category_id) {
        Ok(value) => value.into(),
        Err(_) => return empty_int_array(&mut env),
    };
    let item_count = match env.get_array_length(&haystacks) {
        Ok(value) => value,
        Err(_) => return empty_int_array(&mut env),
    };
    let category_count = match env.get_array_length(&category_ids) {
        Ok(value) => value,
        Err(_) => return empty_int_array(&mut env),
    };
    let slot_id_count = match env.get_array_length(&slot_ids) {
        Ok(value) => value,
        Err(_) => return empty_int_array(&mut env),
    };
    if category_count != item_count || slot_id_count != item_count {
        return empty_int_array(&mut env);
    }

    let keywords = normalize_query(&query);
    let mut slot_id_values = vec![0; slot_id_count as usize];
    if slot_id_count > 0
        && env
            .get_int_array_region(&slot_ids, 0, &mut slot_id_values)
            .is_err()
    {
        return empty_int_array(&mut env);
    }

    let has_category_filter = !filter_category_id.is_empty();
    let has_slot_filter = filter_slot_id >= 0;
    let mut matched_indices = Vec::<jint>::with_capacity(item_count as usize);

    for index in 0..item_count {
        if has_category_filter {
            let item_category_id =
                object_array_string_at(&mut env, &category_ids, index).unwrap_or_default();
            if item_category_id != filter_category_id {
                continue;
            }
        }
        if has_slot_filter && slot_id_values[index as usize] != filter_slot_id {
            continue;
        }

        if !keywords.is_empty() {
            let haystack = object_array_string_at(&mut env, &haystacks, index).unwrap_or_default();
            if !history_query_matches(&haystack, &keywords) {
                continue;
            }
        }
        matched_indices.push(index);
    }

    int_array_from_slice(env, &matched_indices)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFilterHistorySessions(
    mut env: JNIEnv,
    _class: JClass,
    query: JString,
    filter_category_id: JString,
    filter_slot_id: jint,
    category_ids: JObjectArray,
    slot_ids: JIntArray,
    ended_at_epoch_millis: JLongArray,
    duration_millis: JLongArray,
    haystacks: JObjectArray,
    day_start_epoch_millis: jlong,
    next_day_start_epoch_millis: jlong,
    week_window_start_epoch_millis: jlong,
) -> jlongArray {
    let query: String = match env.get_string(&query) {
        Ok(value) => value.into(),
        Err(_) => return long_array_from_slice(env, &[0, 0]),
    };
    let filter_category_id: String = match env.get_string(&filter_category_id) {
        Ok(value) => value.into(),
        Err(_) => return long_array_from_slice(env, &[0, 0]),
    };
    let item_count = match env.get_array_length(&haystacks) {
        Ok(value) => value,
        Err(_) => return long_array_from_slice(env, &[0, 0]),
    };
    let category_count = match env.get_array_length(&category_ids) {
        Ok(value) => value,
        Err(_) => return long_array_from_slice(env, &[0, 0]),
    };
    let slot_id_count = match env.get_array_length(&slot_ids) {
        Ok(value) => value,
        Err(_) => return long_array_from_slice(env, &[0, 0]),
    };
    let ended_at_count = match env.get_array_length(&ended_at_epoch_millis) {
        Ok(value) => value,
        Err(_) => return long_array_from_slice(env, &[0, 0]),
    };
    let duration_count = match env.get_array_length(&duration_millis) {
        Ok(value) => value,
        Err(_) => return long_array_from_slice(env, &[0, 0]),
    };
    if category_count != item_count
        || slot_id_count != item_count
        || ended_at_count != item_count
        || duration_count != item_count
    {
        return long_array_from_slice(env, &[0, 0]);
    }

    let keywords = normalize_query(&query);
    let mut slot_id_values = vec![0; slot_id_count as usize];
    if slot_id_count > 0
        && env
            .get_int_array_region(&slot_ids, 0, &mut slot_id_values)
            .is_err()
    {
        return long_array_from_slice(env, &[0, 0]);
    }
    let mut ended_at_values = vec![0; ended_at_count as usize];
    if ended_at_count > 0
        && env
            .get_long_array_region(&ended_at_epoch_millis, 0, &mut ended_at_values)
            .is_err()
    {
        return long_array_from_slice(env, &[0, 0]);
    }
    let mut duration_values = vec![0; duration_count as usize];
    if duration_count > 0
        && env
            .get_long_array_region(&duration_millis, 0, &mut duration_values)
            .is_err()
    {
        return long_array_from_slice(env, &[0, 0]);
    }

    let has_category_filter = !filter_category_id.is_empty();
    let has_slot_filter = filter_slot_id >= 0;
    let mut matched_values = Vec::<jlong>::with_capacity(item_count as usize + 2);
    let mut today_total_millis = 0_i64;
    let mut week_total_millis = 0_i64;

    matched_values.push(0);
    matched_values.push(0);

    for index in 0..item_count {
        if has_category_filter {
            let item_category_id =
                object_array_string_at(&mut env, &category_ids, index).unwrap_or_default();
            if item_category_id != filter_category_id {
                continue;
            }
        }
        if has_slot_filter && slot_id_values[index as usize] != filter_slot_id {
            continue;
        }

        if !keywords.is_empty() {
            let haystack = object_array_string_at(&mut env, &haystacks, index).unwrap_or_default();
            if !history_query_matches(&haystack, &keywords) {
                continue;
            }
        }

        let ended_at = ended_at_values[index as usize];
        let safe_duration = duration_values[index as usize].max(0);
        if ended_at >= day_start_epoch_millis && ended_at < next_day_start_epoch_millis {
            today_total_millis = today_total_millis.saturating_add(safe_duration);
        }
        if ended_at >= week_window_start_epoch_millis {
            week_total_millis = week_total_millis.saturating_add(safe_duration);
        }
        matched_values.push(index as jlong);
    }

    matched_values[0] = today_total_millis;
    matched_values[1] = week_total_millis;
    long_array_from_slice(env, &matched_values)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeTransformNoteContent(
    mut env: JNIEnv,
    _class: JClass,
    action_code: jint,
    content: JString,
    selection_start: jint,
    selection_end: jint,
) -> jobjectArray {
    let content: String = match env.get_string(&content) {
        Ok(value) => value.into(),
        Err(_) => return empty_string_array(&mut env),
    };
    let action = match NoteTextAction::from_native_code(action_code) {
        Some(action) => action,
        None => return empty_string_array(&mut env),
    };
    let edit = transform_note_content(
        action,
        &content,
        selection_start.max(0) as usize,
        selection_end.max(0) as usize,
    );
    let selection_start_text = edit.selection_start_utf16.to_string();
    let selection_end_text = edit.selection_end_utf16.to_string();
    let payload = [
        edit.content.as_str(),
        selection_start_text.as_str(),
        selection_end_text.as_str(),
    ];
    string_array_from_slice(&mut env, &payload)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeRenderNoteDocumentHtml(
    mut env: JNIEnv,
    _class: JClass,
    payload_json: JString,
) -> jstring {
    let payload_json: String = match env.get_string(&payload_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let payload = match serde_json::from_str::<NoteHtmlPayload>(&payload_json) {
        Ok(value) => value,
        Err(_) => return std::ptr::null_mut(),
    };
    new_java_string(&mut env, &render_note_document_html(payload))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeRenderRichTextBody(
    mut env: JNIEnv,
    _class: JClass,
    content: JString,
    markdown_enabled: jboolean,
) -> jstring {
    let content: String = match env.get_string(&content) {
        Ok(value) => value.into(),
        Err(_) => String::new(),
    };
    new_java_string(
        &mut env,
        &render_rich_text_body(&content, markdown_enabled == JNI_TRUE),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeSanitizeFinanceProfileJson(
    mut env: JNIEnv,
    _class: JClass,
    profile_json: JString,
) -> jstring {
    let profile_json: String = match env.get_string(&profile_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match finance_profile::sanitize_finance_profile_json(&profile_json) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeDecodeFinanceBackupProfileJson(
    mut env: JNIEnv,
    _class: JClass,
    backup_json: JString,
) -> jstring {
    let backup_json: String = match env.get_string(&backup_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match finance_profile::decode_finance_backup_profile_json(&backup_json) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeEncodeFinanceBackupJson(
    mut env: JNIEnv,
    _class: JClass,
    profile_json: JString,
    app_version_name: JString,
    exported_at_epoch_millis: jlong,
) -> jstring {
    let profile_json: String = match env.get_string(&profile_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let app_version_name: String = match env.get_string(&app_version_name) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match finance_profile::encode_finance_backup_json(
        &profile_json,
        &app_version_name,
        exported_at_epoch_millis,
    ) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeBuildFinanceTrendSnapshot(
    mut env: JNIEnv,
    _class: JClass,
    profile_json: JString,
    period_code: jint,
    reference_year: jint,
    reference_month: jint,
    reference_day: jint,
) -> jdoubleArray {
    let profile_json: String = match env.get_string(&profile_json) {
        Ok(value) => value.into(),
        Err(_) => return double_array_from_slice(env, &[]),
    };
    let Some(values) = finance_profile::build_finance_trend_values(
        &profile_json,
        period_code,
        reference_year,
        reference_month,
        reference_day,
    ) else {
        return double_array_from_slice(env, &[]);
    };
    double_array_from_slice(
        env,
        &[
            values.income_delta as f64,
            values.outflow_delta as f64,
            values.net_cashflow_delta as f64,
            values.net_worth_delta as f64,
            values.current_recorded_days as f64,
            values.previous_recorded_days as f64,
            values.most_off_target_bucket_code as f64,
            values.most_off_target_ratio_delta as f64,
        ],
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeBuildFinanceAlertPlan(
    mut env: JNIEnv,
    _class: JClass,
    profile_json: JString,
    period_code: jint,
    reference_year: jint,
    reference_month: jint,
    reference_day: jint,
) -> jintArray {
    let profile_json: String = match env.get_string(&profile_json) {
        Ok(value) => value.into(),
        Err(_) => return empty_int_array_from_env(env),
    };
    let Some(values) = finance_profile::build_finance_alert_plan_values(
        &profile_json,
        period_code,
        reference_year,
        reference_month,
        reference_day,
    ) else {
        return empty_int_array_from_env(env);
    };
    int_array_from_slice(env, &values)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeBuildFinanceAlertRenderArgs(
    mut env: JNIEnv,
    _class: JClass,
    profile_json: JString,
    period_code: jint,
    reference_year: jint,
    reference_month: jint,
    reference_day: jint,
) -> jobjectArray {
    let profile_json: String = match env.get_string(&profile_json) {
        Ok(value) => value.into(),
        Err(_) => return empty_string_array(&mut env),
    };
    let Some(values) = finance_profile::build_finance_alert_argument_values(
        &profile_json,
        period_code,
        reference_year,
        reference_month,
        reference_day,
    ) else {
        return empty_string_array(&mut env);
    };
    let refs = values.iter().map(String::as_str).collect::<Vec<_>>();
    string_array_from_slice(&mut env, &refs)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFinanceNarrativeCode(
    _env: JNIEnv,
    _class: JClass,
    has_entries: jboolean,
    passive_coverage_ratio: jfloat,
    total_outflow: jlong,
    asset_yield_ratio: jfloat,
    liability_pressure_ratio: jfloat,
    freedom_gap: jlong,
) -> jint {
    finance_narrative_code(
        has_entries == JNI_TRUE,
        passive_coverage_ratio,
        total_outflow,
        asset_yield_ratio,
        liability_pressure_ratio,
        freedom_gap,
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeBuildDetailedFinanceSnapshot(
    mut env: JNIEnv,
    _class: JClass,
    profile_json: JString,
    reference_year: jint,
    reference_month: jint,
) -> jdoubleArray {
    let profile_json: String = match env.get_string(&profile_json) {
        Ok(value) => value.into(),
        Err(_) => return double_array_from_slice(env, &[]),
    };
    let Some(values) = finance_profile::build_detailed_finance_snapshot_values(
        &profile_json,
        reference_year,
        reference_month,
    ) else {
        return double_array_from_slice(env, &[]);
    };
    double_array_from_slice(env, &finance_snapshot_values(values, false))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeBuildDetailedFinanceReportSnapshot(
    mut env: JNIEnv,
    _class: JClass,
    profile_json: JString,
    period_code: jint,
    reference_year: jint,
    reference_month: jint,
    reference_day: jint,
) -> jdoubleArray {
    let profile_json: String = match env.get_string(&profile_json) {
        Ok(value) => value.into(),
        Err(_) => return double_array_from_slice(env, &[]),
    };
    let Some(values) = finance_profile::build_detailed_finance_report_values(
        &profile_json,
        period_code,
        reference_year,
        reference_month,
        reference_day,
    ) else {
        return double_array_from_slice(env, &[]);
    };
    double_array_from_slice(env, &finance_snapshot_values(values, true))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeYearNetWorthSummary(
    mut env: JNIEnv,
    _class: JClass,
    profile_json: JString,
    year: jint,
) -> jlongArray {
    let profile_json: String = match env.get_string(&profile_json) {
        Ok(value) => value.into(),
        Err(_) => return empty_long_array_from_env(env),
    };
    let Some(values) = finance_profile::year_net_worth_summary_values(&profile_json, year) else {
        return empty_long_array_from_env(env);
    };
    long_array_from_slice(env, &values)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeLatestSnapshotMonthKeyUpTo(
    mut env: JNIEnv,
    _class: JClass,
    profile_json: JString,
    reference_month_key: JString,
) -> jstring {
    let profile_json: String = match env.get_string(&profile_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let reference_month_key: String = match env.get_string(&reference_month_key) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let Some(value) =
        finance_profile::latest_snapshot_month_key_up_to(&profile_json, &reference_month_key)
    else {
        return std::ptr::null_mut();
    };
    new_java_string(&mut env, &value)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativePreviousRecordedDayKey(
    mut env: JNIEnv,
    _class: JClass,
    profile_json: JString,
    before_day_key: JString,
) -> jstring {
    let profile_json: String = match env.get_string(&profile_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let before_day_key: String = match env.get_string(&before_day_key) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let Some(value) = finance_profile::previous_recorded_day_key(&profile_json, &before_day_key)
    else {
        return std::ptr::null_mut();
    };
    new_java_string(&mut env, &value)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativePreviousRecordedMonthKey(
    mut env: JNIEnv,
    _class: JClass,
    profile_json: JString,
    before_month_key: JString,
) -> jstring {
    let profile_json: String = match env.get_string(&profile_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let before_month_key: String = match env.get_string(&before_month_key) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let Some(value) =
        finance_profile::previous_recorded_month_key(&profile_json, &before_month_key)
    else {
        return std::ptr::null_mut();
    };
    new_java_string(&mut env, &value)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFinanceProfileSummaryFlags(
    mut env: JNIEnv,
    _class: JClass,
    profile_json: JString,
) -> jintArray {
    let profile_json: String = match env.get_string(&profile_json) {
        Ok(value) => value.into(),
        Err(_) => return empty_int_array(&mut env),
    };
    let Some(values) = finance_profile::profile_summary_flags(&profile_json) else {
        return empty_int_array(&mut env);
    };
    int_array_from_slice(env, &values)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeAggregateFinanceProfileLedgers(
    mut env: JNIEnv,
    _class: JClass,
    profile_json: JString,
    period_code: jint,
    reference_year: jint,
    reference_month: jint,
    reference_day: jint,
) -> jlongArray {
    let profile_json: String = match env.get_string(&profile_json) {
        Ok(value) => value.into(),
        Err(_) => return empty_long_array(&mut env),
    };
    let Some(values) = finance_profile::aggregate_finance_ledger_values(
        &profile_json,
        period_code,
        reference_year,
        reference_month,
        reference_day,
    ) else {
        return empty_long_array(&mut env);
    };
    let array = [
        values.active_income_total,
        values.asset_income_total,
        values.other_income_total,
        values.debt_total,
        values.food_total,
        values.btc_total,
        values.living_total,
        values.learning_total,
        values.other_expense_total,
        values.days_with_entries,
    ];
    long_array_from_slice(env, &array)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeUpsertFinanceDayLedgerJson(
    mut env: JNIEnv,
    _class: JClass,
    profile_json: JString,
    day_key: JString,
    ledger_json: JString,
) -> jstring {
    let profile_json: String = match env.get_string(&profile_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let day_key: String = match env.get_string(&day_key) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let ledger_json: String = match env.get_string(&ledger_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match finance_profile::upsert_finance_day_ledger_json(&profile_json, &day_key, &ledger_json) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeUpsertFinanceMonthSnapshotJson(
    mut env: JNIEnv,
    _class: JClass,
    profile_json: JString,
    month_key: JString,
    snapshot_json: JString,
) -> jstring {
    let profile_json: String = match env.get_string(&profile_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let month_key: String = match env.get_string(&month_key) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let snapshot_json: String = match env.get_string(&snapshot_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match finance_profile::upsert_finance_month_snapshot_json(
        &profile_json,
        &month_key,
        &snapshot_json,
    ) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFinanceDayLedgerOrDefaultJson(
    mut env: JNIEnv,
    _class: JClass,
    profile_json: JString,
    day_key: JString,
) -> jstring {
    let profile_json: String = match env.get_string(&profile_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let day_key: String = match env.get_string(&day_key) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match finance_profile::finance_day_ledger_or_default_json(&profile_json, &day_key) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFinanceMonthSnapshotOrDefaultJson(
    mut env: JNIEnv,
    _class: JClass,
    profile_json: JString,
    month_key: JString,
) -> jstring {
    let profile_json: String = match env.get_string(&profile_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let month_key: String = match env.get_string(&month_key) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match finance_profile::finance_month_snapshot_or_default_json(&profile_json, &month_key) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeSanitizeFinanceSettingsJson(
    mut env: JNIEnv,
    _class: JClass,
    settings_json: JString,
) -> jstring {
    let settings_json: String = match env.get_string(&settings_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match finance_profile::sanitize_finance_settings_json(&settings_json) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFinanceSettingsConfigForJson(
    mut env: JNIEnv,
    _class: JClass,
    settings_json: JString,
    bucket_code: jint,
) -> jstring {
    let settings_json: String = match env.get_string(&settings_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match finance_profile::finance_settings_config_for_json(&settings_json, bucket_code) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeReplaceFinanceExpenseCategoryConfigJson(
    mut env: JNIEnv,
    _class: JClass,
    settings_json: JString,
    bucket_code: jint,
    config_json: JString,
) -> jstring {
    let settings_json: String = match env.get_string(&settings_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let config_json: String = match env.get_string(&config_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match finance_profile::replace_finance_expense_category_config_json(
        &settings_json,
        bucket_code,
        &config_json,
    ) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeSanitizeFinanceExpenseCategoryConfigJson(
    mut env: JNIEnv,
    _class: JClass,
    config_json: JString,
    fallback_bucket_code: jint,
) -> jstring {
    let config_json: String = match env.get_string(&config_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match finance_profile::sanitize_finance_expense_category_config_json(
        &config_json,
        fallback_bucket_code,
    ) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeDefaultFinanceExpenseCategoryConfigsJson(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    match finance_profile::default_finance_expense_category_configs_json() {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeDefaultFinanceExpenseCategoryConfigJson(
    mut env: JNIEnv,
    _class: JClass,
    bucket_code: jint,
) -> jstring {
    match finance_profile::default_finance_expense_category_config_json(bucket_code) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeSanitizeAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::sanitize_app_data_json(&app_data_json, now) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeRegisterSyncAccount(
    mut env: JNIEnv,
    _class: JClass,
    server_url: JString,
    email: JString,
    password: JString,
    device_name: JString,
) -> jstring {
    let server_url = string_from_jstring(&mut env, &server_url);
    let email = string_from_jstring(&mut env, &email);
    let password = string_from_jstring(&mut env, &password);
    let device_name = string_from_jstring(&mut env, &device_name);
    let result = sync_core::register_account_json(&server_url, &email, &password, &device_name);
    new_java_string(&mut env, &result)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeLoginSyncAccount(
    mut env: JNIEnv,
    _class: JClass,
    server_url: JString,
    email: JString,
    password: JString,
    device_name: JString,
) -> jstring {
    let server_url = string_from_jstring(&mut env, &server_url);
    let email = string_from_jstring(&mut env, &email);
    let password = string_from_jstring(&mut env, &password);
    let device_name = string_from_jstring(&mut env, &device_name);
    let result = sync_core::login_account_json(&server_url, &email, &password, &device_name);
    new_java_string(&mut env, &result)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeSyncAppData(
    mut env: JNIEnv,
    _class: JClass,
    server_url: JString,
    token: JString,
    app_data_json: JString,
    client_updated_at_epoch_millis: jlong,
    device_name: JString,
) -> jstring {
    let server_url = string_from_jstring(&mut env, &server_url);
    let token = string_from_jstring(&mut env, &token);
    let app_data_json = string_from_jstring(&mut env, &app_data_json);
    let device_name = string_from_jstring(&mut env, &device_name);
    let result = sync_core::sync_app_data_json(
        &server_url,
        &token,
        &app_data_json,
        client_updated_at_epoch_millis,
        &device_name,
    );
    new_java_string(&mut env, &result)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeUploadLocalAppData(
    mut env: JNIEnv,
    _class: JClass,
    server_url: JString,
    token: JString,
    app_data_json: JString,
    client_updated_at_epoch_millis: jlong,
    device_name: JString,
) -> jstring {
    let server_url = string_from_jstring(&mut env, &server_url);
    let token = string_from_jstring(&mut env, &token);
    let app_data_json = string_from_jstring(&mut env, &app_data_json);
    let device_name = string_from_jstring(&mut env, &device_name);
    let result = sync_core::upload_local_app_data_json(
        &server_url,
        &token,
        &app_data_json,
        client_updated_at_epoch_millis,
        &device_name,
    );
    new_java_string(&mut env, &result)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeCompleteNoteWithAi(
    mut env: JNIEnv,
    _class: JClass,
    api_key: JString,
    base_url: JString,
    model: JString,
    title: JString,
    content: JString,
    user_instruction: JString,
) -> jobjectArray {
    let api_key = string_from_jstring(&mut env, &api_key);
    let base_url = string_from_jstring(&mut env, &base_url);
    let model = string_from_jstring(&mut env, &model);
    let title = string_from_jstring(&mut env, &title);
    let content = string_from_jstring(&mut env, &content);
    let user_instruction = string_from_jstring(&mut env, &user_instruction);
    let result = ai_client::complete_note(
        &api_key,
        &base_url,
        &model,
        &title,
        &content,
        &user_instruction,
    );
    let ok = if result.ok { "true" } else { "false" };
    let values = [ok, result.message.as_str(), result.content.as_str()];
    string_array_from_slice(&mut env, &values)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeRankKnowledgeSources(
    mut env: JNIEnv,
    _class: JClass,
    query: JString,
    titles: JObjectArray,
    bodies: JObjectArray,
    folders: JObjectArray,
) -> jintArray {
    let query = string_from_jstring(&mut env, &query);
    let Some(titles) = string_array_values(&mut env, &titles) else {
        return empty_int_array(&mut env);
    };
    let Some(bodies) = string_array_values(&mut env, &bodies) else {
        return empty_int_array(&mut env);
    };
    let Some(folders) = string_array_values(&mut env, &folders) else {
        return empty_int_array(&mut env);
    };
    let indices = rank_knowledge_source_indices(&query, &titles, &bodies, &folders);
    int_array_from_slice(env, &indices)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeCompleteKnowledgeWithAi(
    mut env: JNIEnv,
    _class: JClass,
    api_key: JString,
    base_url: JString,
    model: JString,
    question: JString,
    source_titles: JObjectArray,
    source_folders: JObjectArray,
    source_excerpts: JObjectArray,
) -> jobjectArray {
    let api_key = string_from_jstring(&mut env, &api_key);
    let base_url = string_from_jstring(&mut env, &base_url);
    let model = string_from_jstring(&mut env, &model);
    let question = string_from_jstring(&mut env, &question);
    let source_titles = string_array_values(&mut env, &source_titles).unwrap_or_default();
    let source_folders = string_array_values(&mut env, &source_folders).unwrap_or_default();
    let source_excerpts = string_array_values(&mut env, &source_excerpts).unwrap_or_default();
    let result = ai_client::complete_knowledge_query(
        &api_key,
        &base_url,
        &model,
        &question,
        &source_titles,
        &source_folders,
        &source_excerpts,
    );
    let ok = if result.ok { "true" } else { "false" };
    let values = [ok, result.message.as_str(), result.content.as_str()];
    string_array_from_slice(&mut env, &values)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeDefaultFinanceExpenseEntriesJson(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    match finance_profile::default_finance_expense_entries_json() {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeDefaultFinanceIncomeEntriesJson(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    match finance_profile::default_finance_income_entries_json() {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeDefaultFinanceAssetEntriesJson(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    match finance_profile::default_finance_asset_entries_json() {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeDefaultFinanceLiabilityEntriesJson(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    match finance_profile::default_finance_liability_entries_json() {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeSelectPersistedAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    primary_json: JString,
    backup_json: JString,
    now: jlong,
) -> jstring {
    let primary_json: String = env
        .get_string(&primary_json)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    let backup_json: String = env
        .get_string(&backup_json)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    match timer_insights::select_persisted_app_data_json(&primary_json, &backup_json, now) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeDeleteSessionAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    session_id: JString,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let session_id: String = match env.get_string(&session_id) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::delete_session_app_data_json(&app_data_json, &session_id, now) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeDeleteArchivedTaskAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    archived_task_id: JString,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let archived_task_id: String = match env.get_string(&archived_task_id) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::delete_archived_task_app_data_json(&app_data_json, &archived_task_id, now) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeUpsertNoteAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    note_json: JString,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let note_json: String = match env.get_string(&note_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::upsert_note_app_data_json(&app_data_json, &note_json, now) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeHistoryDeletionSummary(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    session_id: JString,
    archived_task_id: JString,
    now: jlong,
) -> jlongArray {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return empty_long_array(&mut env),
    };
    let session_id: String = match env.get_string(&session_id) {
        Ok(value) => value.into(),
        Err(_) => return empty_long_array(&mut env),
    };
    let archived_task_id: String = match env.get_string(&archived_task_id) {
        Ok(value) => value.into(),
        Err(_) => return empty_long_array(&mut env),
    };
    let Some(values) = app_data::history_deletion_summary_values(
        &app_data_json,
        &session_id,
        &archived_task_id,
        now,
    ) else {
        return empty_long_array(&mut env);
    };
    long_array_from_slice(env, &values)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeUpdateSlotTitleAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    slot_id: jint,
    title: JString,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let title: String = match env.get_string(&title) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::update_slot_title_app_data_json(&app_data_json, slot_id, &title, now) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeUpdateSlotNoteAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    slot_id: jint,
    note: JString,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let note: String = match env.get_string(&note) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::update_slot_note_app_data_json(&app_data_json, slot_id, &note, now) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeSetSlotCategoryAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    slot_id: jint,
    category_id: JString,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let category_id: String = match env.get_string(&category_id) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::set_slot_category_app_data_json(&app_data_json, slot_id, &category_id, now) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeSetSlotOrderAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    slot_order: JIntArray,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let Some(slot_order_values) = int_array_values(&env, &slot_order) else {
        return std::ptr::null_mut();
    };
    match app_data::set_slot_order_app_data_json(&app_data_json, &slot_order_values, now) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeAddCategoryAndAssignAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    slot_id: jint,
    category_id: JString,
    name: JString,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let category_id: String = match env.get_string(&category_id) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let name: String = match env.get_string(&name) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::add_category_and_assign_app_data_json(
        &app_data_json,
        slot_id,
        &category_id,
        &name,
        now,
    ) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeStartSlotAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    slot_id: jint,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::start_slot_app_data_json(&app_data_json, slot_id, now) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativePauseSlotsAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    slot_ids: JIntArray,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let Some(slot_id_values) = int_array_values(&env, &slot_ids) else {
        return std::ptr::null_mut();
    };
    match app_data::pause_slots_app_data_json(&app_data_json, &slot_id_values, now) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeResetSlotAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    slot_id: jint,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::reset_slot_app_data_json(&app_data_json, slot_id, now) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeCreateNoteFolderAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    folder_id: JString,
    name: JString,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let folder_id: String = match env.get_string(&folder_id) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let name: String = match env.get_string(&name) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::create_note_folder_app_data_json(&app_data_json, &folder_id, &name, now) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeRenameNoteFolderAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    folder_id: JString,
    name: JString,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let folder_id: String = match env.get_string(&folder_id) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let name: String = match env.get_string(&name) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::rename_note_folder_app_data_json(&app_data_json, &folder_id, &name, now) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeDeleteNoteFolderAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    folder_id: JString,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let folder_id: String = match env.get_string(&folder_id) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::delete_note_folder_app_data_json(&app_data_json, &folder_id, now) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeSetSelectedNoteFolderAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    folder_id: JString,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let folder_id: String = match env.get_string(&folder_id) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::set_selected_note_folder_app_data_json(&app_data_json, &folder_id, now) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeSetNoteSortModeAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    sort_mode_code: jint,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::set_note_sort_mode_app_data_json(&app_data_json, sort_mode_code, now) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeMoveNoteToFolderAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    note_id: JString,
    folder_id: JString,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let note_id: String = match env.get_string(&note_id) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let folder_id: String = match env.get_string(&folder_id) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::move_note_to_folder_app_data_json(&app_data_json, &note_id, &folder_id, now) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeRestoreNoteAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    note_id: JString,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let note_id: String = match env.get_string(&note_id) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::restore_note_app_data_json(&app_data_json, &note_id, now) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeDeleteNoteAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    note_id: JString,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let note_id: String = match env.get_string(&note_id) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::delete_note_app_data_json(&app_data_json, &note_id, now) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeSetNotePinnedAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    note_id: JString,
    pinned: jboolean,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let note_id: String = match env.get_string(&note_id) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::set_note_pinned_app_data_json(&app_data_json, &note_id, pinned == JNI_TRUE, now)
    {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeCaptureNoteRevisionAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    note_id: JString,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let note_id: String = match env.get_string(&note_id) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::capture_note_revision_app_data_json(&app_data_json, &note_id, now) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeRestoreNoteRevisionAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    note_id: JString,
    revision_id: JString,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let note_id: String = match env.get_string(&note_id) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let revision_id: String = match env.get_string(&revision_id) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::restore_note_revision_app_data_json(&app_data_json, &note_id, &revision_id, now)
    {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeDeleteNotePermanentlyAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    note_id: JString,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let note_id: String = match env.get_string(&note_id) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::delete_note_permanently_app_data_json(&app_data_json, &note_id, now) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeEmptyNoteTrashAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::empty_note_trash_app_data_json(&app_data_json, now) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeSetThemeModeAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    theme_mode_code: jint,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::set_theme_mode_app_data_json(&app_data_json, theme_mode_code, now) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeNoteFolderCountPairs(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
) -> jobjectArray {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return empty_string_array(&mut env),
    };
    let Some(pairs) = app_data::note_folder_count_pairs(&app_data_json) else {
        return empty_string_array(&mut env);
    };
    let payload = pairs
        .into_iter()
        .flat_map(|(folder_id, count)| [folder_id, count.to_string()])
        .collect::<Vec<_>>();
    let refs = payload.iter().map(String::as_str).collect::<Vec<_>>();
    string_array_from_slice(&mut env, &refs)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeNoteVisibilityIndices(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    deleted: jboolean,
) -> jintArray {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return empty_int_array(&mut env),
    };
    let Some(indices) = app_data::note_visibility_indices(&app_data_json, deleted == JNI_TRUE)
    else {
        return empty_int_array(&mut env);
    };
    int_array_from_slice(env, &indices)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeIsNoteBlankDraftJson(
    mut env: JNIEnv,
    _class: JClass,
    note_json: JString,
    now: jlong,
) -> jboolean {
    let note_json: String = match env.get_string(&note_json) {
        Ok(value) => value.into(),
        Err(_) => return JNI_FALSE,
    };
    boolean_to_jni(app_data::is_note_blank_draft_json(&note_json, now).unwrap_or(false))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeUpdateFinanceProfileAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    profile_json: JString,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let profile_json: String = match env.get_string(&profile_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::update_finance_profile_app_data_json(&app_data_json, &profile_json, now) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeArchiveSlotAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    slot_id: jint,
    archived_task_id: JString,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let archived_task_id: String = match env.get_string(&archived_task_id) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::archive_slot_app_data_json(&app_data_json, slot_id, &archived_task_id, now) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeRestoreArchivedTaskAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    archived_task_id: JString,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let archived_task_id: String = match env.get_string(&archived_task_id) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::restore_archived_task_app_data_json(&app_data_json, &archived_task_id, now) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeImportNoteImageAttachmentAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    note_id: JString,
    attachment_json: JString,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let note_id: String = match env.get_string(&note_id) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let attachment_json: String = match env.get_string(&attachment_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::import_note_image_attachment_app_data_json(
        &app_data_json,
        &note_id,
        &attachment_json,
        now,
    ) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeDeleteNoteAttachmentAppDataJson(
    mut env: JNIEnv,
    _class: JClass,
    app_data_json: JString,
    note_id: JString,
    attachment_id: JString,
    now: jlong,
) -> jstring {
    let app_data_json: String = match env.get_string(&app_data_json) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let note_id: String = match env.get_string(&note_id) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let attachment_id: String = match env.get_string(&attachment_id) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match app_data::delete_note_attachment_app_data_json(
        &app_data_json,
        &note_id,
        &attachment_id,
        now,
    ) {
        Some(value) => new_java_string(&mut env, &value),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeNoteImageImportPolicy(
    _env: JNIEnv,
    _class: JClass,
    note_exists: jboolean,
    note_deleted: jboolean,
    attachment_count: jint,
    max_attachment_count: jint,
) -> jint {
    app_data::note_image_import_policy(
        note_exists == JNI_TRUE,
        note_deleted == JNI_TRUE,
        attachment_count,
        max_attachment_count,
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeShouldDeleteUnattachedImportedNoteImage(
    _env: JNIEnv,
    _class: JClass,
    imported: jboolean,
    attached: jboolean,
) -> jboolean {
    boolean_to_jni(app_data::should_delete_unattached_imported_note_image(
        imported == JNI_TRUE,
        attached == JNI_TRUE,
    ))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeRepositoryPersistPlanFlags(
    _env: JNIEnv,
    _class: JClass,
    state_file_exists: jboolean,
    backup_file_exists: jboolean,
) -> jint {
    app_data::repository_persist_plan_flags(
        state_file_exists == JNI_TRUE,
        backup_file_exists == JNI_TRUE,
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeDeriveSmartNoteDigest(
    mut env: JNIEnv,
    _class: JClass,
    title: JString,
    content: JString,
    attachment_count: jint,
) -> jobjectArray {
    let title: String = match env.get_string(&title) {
        Ok(value) => value.into(),
        Err(_) => return empty_string_array(&mut env),
    };
    let content: String = match env.get_string(&content) {
        Ok(value) => value.into(),
        Err(_) => return empty_string_array(&mut env),
    };
    let digest = derive_smart_note_digest(&title, &content, attachment_count.max(0) as usize);
    let payload = [digest.0.as_str(), digest.1.as_str()];
    string_array_from_slice(&mut env, &payload)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeBuildNoteDocumentTextDigest(
    mut env: JNIEnv,
    _class: JClass,
    rich_text_enabled: jboolean,
    rich_text_plain_text: JString,
    block_type_codes: JIntArray,
    block_texts: JObjectArray,
    block_captions: JObjectArray,
    block_contact_names: JObjectArray,
    block_contact_organizations: JObjectArray,
    block_first_contact_phone_numbers: JObjectArray,
    block_contact_phone_search_texts: JObjectArray,
    block_call_contact_names: JObjectArray,
    block_call_phone_numbers: JObjectArray,
    block_call_direction_names: JObjectArray,
    preserve_structure: jboolean,
    skip_leading_title_line: jboolean,
    excluded_text: JString,
) -> jobjectArray {
    let rich_text_plain_text: String = match env.get_string(&rich_text_plain_text) {
        Ok(value) => value.into(),
        Err(_) => return empty_string_array(&mut env),
    };
    let excluded_text: String = match env.get_string(&excluded_text) {
        Ok(value) => value.into(),
        Err(_) => return empty_string_array(&mut env),
    };
    let block_type_codes = match int_array_values(&env, &block_type_codes) {
        Some(values) => values,
        None => return empty_string_array(&mut env),
    };
    let block_texts = match string_array_values(&mut env, &block_texts) {
        Some(values) => values,
        None => return empty_string_array(&mut env),
    };
    let block_captions = match string_array_values(&mut env, &block_captions) {
        Some(values) => values,
        None => return empty_string_array(&mut env),
    };
    let block_contact_names = match string_array_values(&mut env, &block_contact_names) {
        Some(values) => values,
        None => return empty_string_array(&mut env),
    };
    let block_contact_organizations =
        match string_array_values(&mut env, &block_contact_organizations) {
            Some(values) => values,
            None => return empty_string_array(&mut env),
        };
    let block_first_contact_phone_numbers =
        match string_array_values(&mut env, &block_first_contact_phone_numbers) {
            Some(values) => values,
            None => return empty_string_array(&mut env),
        };
    let block_contact_phone_search_texts =
        match string_array_values(&mut env, &block_contact_phone_search_texts) {
            Some(values) => values,
            None => return empty_string_array(&mut env),
        };
    let block_call_contact_names = match string_array_values(&mut env, &block_call_contact_names) {
        Some(values) => values,
        None => return empty_string_array(&mut env),
    };
    let block_call_phone_numbers = match string_array_values(&mut env, &block_call_phone_numbers) {
        Some(values) => values,
        None => return empty_string_array(&mut env),
    };
    let block_call_direction_names =
        match string_array_values(&mut env, &block_call_direction_names) {
            Some(values) => values,
            None => return empty_string_array(&mut env),
        };

    let block_count = block_type_codes.len();
    if [
        block_texts.len(),
        block_captions.len(),
        block_contact_names.len(),
        block_contact_organizations.len(),
        block_first_contact_phone_numbers.len(),
        block_contact_phone_search_texts.len(),
        block_call_contact_names.len(),
        block_call_phone_numbers.len(),
        block_call_direction_names.len(),
    ]
    .iter()
    .any(|size| *size != block_count)
    {
        return empty_string_array(&mut env);
    }

    let blocks = (0..block_count)
        .map(|index| NoteBlockTextInput {
            type_code: block_type_codes[index],
            text: block_texts[index].clone(),
            caption: block_captions[index].clone(),
            contact_name: block_contact_names[index].clone(),
            contact_organization: block_contact_organizations[index].clone(),
            first_contact_phone_number: block_first_contact_phone_numbers[index].clone(),
            contact_phone_search_text: block_contact_phone_search_texts[index].clone(),
            call_contact_name: block_call_contact_names[index].clone(),
            call_phone_number: block_call_phone_numbers[index].clone(),
            call_direction_name: block_call_direction_names[index].clone(),
        })
        .collect::<Vec<_>>();
    let input = NoteDocumentTextInput {
        rich_text_enabled: rich_text_enabled == JNI_TRUE,
        rich_text_plain_text,
        blocks,
    };
    let digest = build_note_document_text_digest(
        &input,
        preserve_structure == JNI_TRUE,
        skip_leading_title_line == JNI_TRUE,
        &excluded_text,
    );
    let title = digest.suggested_title.unwrap_or_default();
    let preview = digest.suggested_preview.unwrap_or_default();
    let payload = [
        title.as_str(),
        preview.as_str(),
        digest.plain_text.as_str(),
        digest.searchable_text.as_str(),
    ];
    string_array_from_slice(&mut env, &payload)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeRichTextAttachmentIds(
    mut env: JNIEnv,
    _class: JClass,
    html: JString,
) -> jobjectArray {
    let html: String = match env.get_string(&html) {
        Ok(value) => value.into(),
        Err(_) => return empty_string_array(&mut env),
    };
    let ids = rich_text_attachment_ids(&html);
    let values = ids.iter().map(String::as_str).collect::<Vec<_>>();
    string_array_from_slice(&mut env, &values)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeRemoveRichTextAttachmentReference(
    mut env: JNIEnv,
    _class: JClass,
    html: JString,
    attachment_id: JString,
) -> jstring {
    let html: String = match env.get_string(&html) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let attachment_id: String = match env.get_string(&attachment_id) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    new_java_string(
        &mut env,
        &remove_rich_text_attachment_reference(&html, &attachment_id),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeBuildNoteCollectionSections(
    env: JNIEnv,
    _class: JClass,
    updated_epoch_days: JLongArray,
    pinned_flags: JBooleanArray,
) -> jlongArray {
    let updated_epoch_days = match long_array_values(&env, &updated_epoch_days) {
        Some(values) => values,
        None => return empty_long_array_from_env(env),
    };
    let pinned_flags = match boolean_array_values(&env, &pinned_flags) {
        Some(values) => values,
        None => return empty_long_array_from_env(env),
    };
    if updated_epoch_days.len() != pinned_flags.len() {
        return empty_long_array_from_env(env);
    }
    let values = build_note_collection_sections(&updated_epoch_days, &pinned_flags);
    long_array_from_slice(env, &values)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeNormalizeSlotOrder(
    env: JNIEnv,
    _class: JClass,
    slot_ids: JIntArray,
    min_slot_id: jint,
    max_slot_id: jint,
) -> jintArray {
    let slot_ids = match int_array_values(&env, &slot_ids) {
        Some(values) => values,
        None => return empty_int_array_from_env(env),
    };
    let values = normalize_slot_order(&slot_ids, min_slot_id, max_slot_id);
    int_array_from_slice(env, &values)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeIsValidFinanceDateKey(
    mut env: JNIEnv,
    _class: JClass,
    value: JString,
) -> jboolean {
    let value: String = match env.get_string(&value) {
        Ok(value) => value.into(),
        Err(_) => return JNI_FALSE,
    };
    boolean_to_jni(is_valid_finance_date_key(&value))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeIsValidFinanceMonthKey(
    mut env: JNIEnv,
    _class: JClass,
    value: JString,
) -> jboolean {
    let value: String = match env.get_string(&value) {
        Ok(value) => value.into(),
        Err(_) => return JNI_FALSE,
    };
    boolean_to_jni(is_valid_finance_month_key(&value))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeCountNoteChecklistStats(
    mut env: JNIEnv,
    _class: JClass,
    content: JString,
) -> jintArray {
    let content: String = match env.get_string(&content) {
        Ok(value) => value.into(),
        Err(_) => return empty_int_array(&mut env),
    };
    let (total, completed) = count_note_checklist_stats(&content);
    int_array_from_slice(env, &[total, completed])
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeCountNoteAttachments(
    env: JNIEnv,
    _class: JClass,
    attachment_kind_codes: JIntArray,
    target_kind_code: jint,
) -> jint {
    let Some(kind_codes) = int_array_values(&env, &attachment_kind_codes) else {
        return -1;
    };
    count_note_attachments(&kind_codes, target_kind_code)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeDisplayNoteLine(
    mut env: JNIEnv,
    _class: JClass,
    line: JString,
    preserve_structure: jboolean,
) -> jstring {
    let line: String = match env.get_string(&line) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    new_java_string(
        &mut env,
        &display_note_line(&line, preserve_structure == JNI_TRUE),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeHomeFocusSuggestion(
    env: JNIEnv,
    _class: JClass,
    slot_ids: JIntArray,
    blank_flags: JIntArray,
    running_since_epoch_millis: JLongArray,
    updated_at_epoch_millis: JLongArray,
) -> jlongArray {
    let Some(slot_id_values) = int_array_values(&env, &slot_ids) else {
        return empty_long_array_from_env(env);
    };
    let Some(blank_flag_values) = int_array_values(&env, &blank_flags) else {
        return empty_long_array_from_env(env);
    };
    let Some(running_since_values) = long_array_values(&env, &running_since_epoch_millis) else {
        return empty_long_array_from_env(env);
    };
    let Some(updated_at_values) = long_array_values(&env, &updated_at_epoch_millis) else {
        return empty_long_array_from_env(env);
    };
    let Some(values) = home_focus_suggestion_values(
        &slot_id_values,
        &blank_flag_values,
        &running_since_values,
        &updated_at_values,
    ) else {
        return empty_long_array_from_env(env);
    };
    long_array_from_slice(env, &values)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeSafeElapsedMillis(
    _env: JNIEnv,
    _class: JClass,
    accumulated_millis: jlong,
    running_since_epoch_millis: jlong,
    phase_code: jint,
    now: jlong,
) -> jlong {
    safe_elapsed_millis(
        accumulated_millis,
        running_since_epoch_millis,
        phase_code,
        now,
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeSumTimerSessionDurationsInWindow(
    env: JNIEnv,
    _class: JClass,
    ended_at_epoch_millis: JLongArray,
    duration_millis: JLongArray,
    window_start_epoch_millis: jlong,
    window_end_epoch_millis: jlong,
) -> jlong {
    let Some(ended_at_values) = long_array_values(&env, &ended_at_epoch_millis) else {
        return -1;
    };
    let Some(duration_values) = long_array_values(&env, &duration_millis) else {
        return -1;
    };
    timer_insights::sum_timer_session_durations_in_window(
        &ended_at_values,
        &duration_values,
        window_start_epoch_millis,
        window_end_epoch_millis,
    )
    .unwrap_or(-1)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeSummarizeTimerSessionsInWindow(
    env: JNIEnv,
    _class: JClass,
    slot_ids: JIntArray,
    ended_at_epoch_millis: JLongArray,
    duration_millis: JLongArray,
    window_start_epoch_millis: jlong,
    window_end_epoch_millis: jlong,
) -> jlongArray {
    let Some(slot_id_values) = int_array_values(&env, &slot_ids) else {
        return empty_long_array_from_env(env);
    };
    let Some(ended_at_values) = long_array_values(&env, &ended_at_epoch_millis) else {
        return empty_long_array_from_env(env);
    };
    let Some(duration_values) = long_array_values(&env, &duration_millis) else {
        return empty_long_array_from_env(env);
    };
    let Some(summary) = timer_insights::summarize_timer_sessions_in_window(
        &slot_id_values,
        &ended_at_values,
        &duration_values,
        window_start_epoch_millis,
        window_end_epoch_millis,
    ) else {
        return empty_long_array_from_env(env);
    };
    long_array_from_slice(env, &summary.as_i64_array())
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeNextMicroBreakDelayMillis(
    env: JNIEnv,
    _class: JClass,
    slot_ids: JIntArray,
    running_since_epoch_millis: JLongArray,
    phase_codes: JIntArray,
    cycle_indices: JIntArray,
    phase_progress_millis: JLongArray,
    now: jlong,
) -> jlong {
    let Some(slot_id_values) = int_array_values(&env, &slot_ids) else {
        return -1;
    };
    let Some(running_since_values) = long_array_values(&env, &running_since_epoch_millis) else {
        return -1;
    };
    let Some(phase_values) = int_array_values(&env, &phase_codes) else {
        return -1;
    };
    let Some(cycle_values) = int_array_values(&env, &cycle_indices) else {
        return -1;
    };
    let Some(progress_values) = long_array_values(&env, &phase_progress_millis) else {
        return -1;
    };
    timer_insights::next_micro_break_delay_millis(
        &slot_id_values,
        &running_since_values,
        &phase_values,
        &cycle_values,
        &progress_values,
        now,
    )
    .unwrap_or(-1)
}

fn empty_boolean_array(env: &mut JNIEnv) -> jbooleanArray {
    env.new_boolean_array(0)
        .map(|value| value.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

fn empty_int_array(env: &mut JNIEnv) -> jintArray {
    env.new_int_array(0)
        .map(|value| value.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

fn empty_long_array(env: &mut JNIEnv) -> jlongArray {
    env.new_long_array(0)
        .map(|value| value.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

fn empty_int_array_from_env(env: JNIEnv) -> jintArray {
    int_array_from_slice(env, &[])
}

fn empty_long_array_from_env(env: JNIEnv) -> jlongArray {
    long_array_from_slice(env, &[])
}

fn empty_string_array(env: &mut JNIEnv) -> jobjectArray {
    env.new_object_array(0, "java/lang/String", JObject::null())
        .map(|value| value.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

fn object_array_string_at(env: &mut JNIEnv, values: &JObjectArray, index: i32) -> Option<String> {
    env.get_object_array_element(values, index)
        .ok()
        .and_then(|value| {
            let value = JString::from(value);
            env.get_string(&value).ok().map(String::from)
        })
}

fn string_array_from_slice(env: &mut JNIEnv, values: &[&str]) -> jobjectArray {
    let result =
        match env.new_object_array(values.len() as i32, "java/lang/String", JObject::null()) {
            Ok(array) => array,
            Err(_) => return std::ptr::null_mut(),
        };
    for (index, value) in values.iter().enumerate() {
        let java_string = match env.new_string(*value) {
            Ok(value) => value,
            Err(_) => return std::ptr::null_mut(),
        };
        if env
            .set_object_array_element(&result, index as i32, java_string)
            .is_err()
        {
            return std::ptr::null_mut();
        }
    }
    result.into_raw()
}

fn string_array_values(env: &mut JNIEnv, values: &JObjectArray) -> Option<Vec<String>> {
    let value_count = env.get_array_length(values).ok()? as usize;
    let mut result = Vec::with_capacity(value_count);
    for index in 0..value_count {
        let raw_value = env.get_object_array_element(values, index as i32).ok()?;
        let java_string = JString::from(raw_value);
        let value: String = env.get_string(&java_string).ok()?.into();
        result.push(value);
    }
    Some(result)
}

fn new_java_string(env: &mut JNIEnv, value: &str) -> jstring {
    env.new_string(value)
        .map(|java_string| java_string.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

fn string_from_jstring(env: &mut JNIEnv, value: &JString) -> String {
    env.get_string(value)
        .ok()
        .map(String::from)
        .unwrap_or_default()
}

fn double_array_from_slice(env: JNIEnv, values: &[f64]) -> jdoubleArray {
    let result = match env.new_double_array(values.len() as i32) {
        Ok(array) => array,
        Err(_) => return std::ptr::null_mut(),
    };
    if env.set_double_array_region(&result, 0, values).is_err() {
        return std::ptr::null_mut();
    }
    result.into_raw()
}

fn int_array_from_slice(env: JNIEnv, values: &[jint]) -> jintArray {
    let result = match env.new_int_array(values.len() as i32) {
        Ok(array) => array,
        Err(_) => return std::ptr::null_mut(),
    };
    if env.set_int_array_region(&result, 0, values).is_err() {
        return std::ptr::null_mut();
    }
    result.into_raw()
}

fn long_array_from_slice(env: JNIEnv, values: &[jlong]) -> jlongArray {
    let result = match env.new_long_array(values.len() as i32) {
        Ok(array) => array,
        Err(_) => return std::ptr::null_mut(),
    };
    if env.set_long_array_region(&result, 0, values).is_err() {
        return std::ptr::null_mut();
    }
    result.into_raw()
}

fn compute_finance_snapshot(
    active_income_monthly: jlong,
    asset_income_monthly: jlong,
    living_expense_monthly: jlong,
    liability_payment_monthly: jlong,
    cash_reserve: jlong,
    productive_asset_value: jlong,
    liability_balance: jlong,
) -> [f64; 10] {
    let safe_active_income_monthly = active_income_monthly.max(0);
    let safe_asset_income_monthly = asset_income_monthly.max(0);
    let safe_living_expense_monthly = living_expense_monthly.max(0);
    let safe_liability_payment_monthly = liability_payment_monthly.max(0);
    let safe_cash_reserve = cash_reserve.max(0);
    let safe_productive_asset_value = productive_asset_value.max(0);
    let safe_liability_balance = liability_balance.max(0);
    let total_income_monthly = safe_active_income_monthly.saturating_add(safe_asset_income_monthly);
    let total_outflow_monthly =
        safe_living_expense_monthly.saturating_add(safe_liability_payment_monthly);
    let net_cashflow_monthly = total_income_monthly - total_outflow_monthly;
    let freedom_gap_monthly = (total_outflow_monthly - safe_asset_income_monthly).max(0);
    let passive_coverage_ratio = safe_ratio(safe_asset_income_monthly, total_outflow_monthly);
    let wage_dependence_ratio = safe_ratio(safe_active_income_monthly, total_income_monthly);
    let liability_pressure_ratio = safe_ratio(safe_liability_payment_monthly, total_income_monthly);
    let net_worth =
        safe_productive_asset_value.saturating_add(safe_cash_reserve) - safe_liability_balance;
    let defensive_base = total_outflow_monthly - safe_asset_income_monthly;
    let defensive_months = if safe_cash_reserve <= 0 {
        0.0
    } else if defensive_base <= 0 {
        f64::NAN
    } else {
        safe_cash_reserve as f64 / defensive_base as f64
    };
    let asset_yield_ratio = safe_ratio(safe_asset_income_monthly, safe_productive_asset_value);

    [
        total_income_monthly as f64,
        total_outflow_monthly as f64,
        net_cashflow_monthly as f64,
        freedom_gap_monthly as f64,
        passive_coverage_ratio,
        wage_dependence_ratio,
        liability_pressure_ratio,
        net_worth as f64,
        defensive_months,
        asset_yield_ratio,
    ]
}

fn compute_finance_ledger_hint(
    period_code: jint,
    income_total: jlong,
    expense_total: jlong,
    debt_total: jlong,
    food_total: jlong,
    btc_total: jlong,
    living_total: jlong,
    learning_total: jlong,
    net_cashflow: jlong,
    opening_net_worth: jlong,
    closing_net_worth: jlong,
) -> jint {
    const HINT_NEED_DATA: jint = 0;
    const HINT_STRUCTURE_GOOD: jint = 1;
    const HINT_DEBT_HIGH: jint = 2;
    const HINT_BTC_LOW: jint = 3;
    const HINT_LEARNING_LOW: jint = 4;
    const HINT_FOOD_HIGH: jint = 5;
    const HINT_TRIM_EXPENSE: jint = 6;
    const HINT_QUARTER_GOOD: jint = 10;
    const HINT_QUARTER_DEBT_HIGH: jint = 11;
    const HINT_QUARTER_MIXED: jint = 12;
    const HINT_YEAR_UP: jint = 20;
    const HINT_YEAR_CASHFLOW_WEAK: jint = 21;
    const HINT_YEAR_DOWN: jint = 22;
    const HINT_YEAR_TARGET: jint = 23;

    match period_code {
        0 | 1 => {
            if income_total <= 0 || expense_total <= 0 {
                return HINT_NEED_DATA;
            }

            let debt_ratio = safe_ratio(debt_total, income_total);
            let food_ratio = safe_ratio(food_total, income_total);
            let btc_ratio = safe_ratio(btc_total, income_total);
            let living_ratio = safe_ratio(living_total, income_total);
            let learning_ratio = safe_ratio(learning_total, income_total);

            let mut near_count = 0;
            if within_target_ratio(debt_ratio, 0.30) {
                near_count += 1;
            }
            if within_target_ratio(food_ratio, 0.20) {
                near_count += 1;
            }
            if within_target_ratio(btc_ratio, 0.10) {
                near_count += 1;
            }
            if within_target_ratio(living_ratio, 0.30) {
                near_count += 1;
            }
            if within_target_ratio(learning_ratio, 0.10) {
                near_count += 1;
            }

            if debt_ratio > 0.40 {
                HINT_DEBT_HIGH
            } else if food_ratio > 0.30 {
                HINT_FOOD_HIGH
            } else if btc_ratio < 0.05 {
                HINT_BTC_LOW
            } else if learning_ratio < 0.05 {
                HINT_LEARNING_LOW
            } else if near_count >= 3 {
                HINT_STRUCTURE_GOOD
            } else {
                HINT_TRIM_EXPENSE
            }
        }
        2 => {
            if income_total <= 0 || expense_total <= 0 {
                HINT_NEED_DATA
            } else if net_cashflow > 0
                && safe_ratio(debt_total, income_total) <= 0.30
                && safe_ratio(learning_total, income_total) >= 0.08
            {
                HINT_QUARTER_GOOD
            } else if safe_ratio(debt_total, income_total) > 0.40 {
                HINT_QUARTER_DEBT_HIGH
            } else {
                HINT_QUARTER_MIXED
            }
        }
        3 => {
            if income_total <= 0 && expense_total <= 0 {
                HINT_NEED_DATA
            } else {
                let delta = closing_net_worth - opening_net_worth;
                if delta > 0 && net_cashflow > 0 {
                    HINT_YEAR_UP
                } else if delta >= 0 && net_cashflow < 0 {
                    HINT_YEAR_CASHFLOW_WEAK
                } else if delta < 0 {
                    HINT_YEAR_DOWN
                } else {
                    HINT_YEAR_TARGET
                }
            }
        }
        _ => HINT_NEED_DATA,
    }
}

fn compute_finance_ledger_hint_with_targets(
    period_code: jint,
    income_total: jlong,
    expense_total: jlong,
    debt_total: jlong,
    food_total: jlong,
    btc_total: jlong,
    living_total: jlong,
    learning_total: jlong,
    net_cashflow: jlong,
    opening_net_worth: jlong,
    closing_net_worth: jlong,
    target_shares: &[f32],
) -> jint {
    const HINT_NEED_DATA: jint = 0;
    const HINT_STRUCTURE_GOOD: jint = 1;
    const HINT_DEBT_HIGH: jint = 2;
    const HINT_BTC_LOW: jint = 3;
    const HINT_LEARNING_LOW: jint = 4;
    const HINT_FOOD_HIGH: jint = 5;
    const HINT_TRIM_EXPENSE: jint = 6;
    const HINT_QUARTER_GOOD: jint = 10;
    const HINT_QUARTER_DEBT_HIGH: jint = 11;
    const HINT_QUARTER_MIXED: jint = 12;
    const HINT_YEAR_UP: jint = 20;
    const HINT_YEAR_CASHFLOW_WEAK: jint = 21;
    const HINT_YEAR_DOWN: jint = 22;
    const HINT_YEAR_TARGET: jint = 23;

    let target = |index: usize, fallback: f64| -> f64 {
        target_shares
            .get(index)
            .copied()
            .filter(|value| value.is_finite())
            .map(|value| value.clamp(0.0, 1.0) as f64)
            .unwrap_or(fallback)
    };

    match period_code {
        0 | 1 => {
            if income_total <= 0 || expense_total <= 0 {
                return HINT_NEED_DATA;
            }

            let ratios = [
                safe_ratio(debt_total, income_total),
                safe_ratio(food_total, income_total),
                safe_ratio(btc_total, income_total),
                safe_ratio(living_total, income_total),
                safe_ratio(learning_total, income_total),
            ];
            let targets = [
                target(0, 0.30),
                target(1, 0.20),
                target(2, 0.10),
                target(3, 0.30),
                target(4, 0.10),
            ];
            let near_count = ratios
                .iter()
                .zip(targets.iter())
                .filter(|(actual, target)| within_target_ratio(**actual, **target))
                .count();

            if ratios[0] > (targets[0] + 0.10).min(0.60) {
                HINT_DEBT_HIGH
            } else if ratios[1] > (targets[1] + 0.10).min(0.60) {
                HINT_FOOD_HIGH
            } else if ratios[2] < targets[2] * 0.5 {
                HINT_BTC_LOW
            } else if ratios[4] < targets[4] * 0.5 {
                HINT_LEARNING_LOW
            } else if near_count >= 3 {
                HINT_STRUCTURE_GOOD
            } else {
                HINT_TRIM_EXPENSE
            }
        }
        2 => {
            if income_total <= 0 || expense_total <= 0 {
                HINT_NEED_DATA
            } else if net_cashflow > 0
                && safe_ratio(debt_total, income_total) <= target(0, 0.30)
                && safe_ratio(learning_total, income_total) >= target(4, 0.08) * 0.8
            {
                HINT_QUARTER_GOOD
            } else if safe_ratio(debt_total, income_total) > target(0, 0.30) + 0.10 {
                HINT_QUARTER_DEBT_HIGH
            } else {
                HINT_QUARTER_MIXED
            }
        }
        3 => {
            if income_total <= 0 && expense_total <= 0 {
                HINT_NEED_DATA
            } else {
                let delta = closing_net_worth - opening_net_worth;
                if delta > 0 && net_cashflow > 0 {
                    HINT_YEAR_UP
                } else if delta >= 0 && net_cashflow < 0 {
                    HINT_YEAR_CASHFLOW_WEAK
                } else if delta < 0 {
                    HINT_YEAR_DOWN
                } else {
                    HINT_YEAR_TARGET
                }
            }
        }
        _ => HINT_NEED_DATA,
    }
}

fn transform_note_content(
    action: NoteTextAction,
    content: &str,
    selection_start_utf16: usize,
    selection_end_utf16: usize,
) -> NoteTextEdit {
    let utf16_length = content.encode_utf16().count();
    let safe_start_utf16 = selection_start_utf16.min(utf16_length);
    let safe_end_utf16 = selection_end_utf16.min(utf16_length);
    let normalized_start_utf16 = safe_start_utf16.min(safe_end_utf16);
    let normalized_end_utf16 = safe_start_utf16.max(safe_end_utf16);

    match action {
        NoteTextAction::Heading => transform_prefixed_lines(
            content,
            normalized_start_utf16,
            normalized_end_utf16,
            HEADING_PREFIX,
        ),
        NoteTextAction::Center => {
            transform_centered_lines(content, normalized_start_utf16, normalized_end_utf16)
        }
        NoteTextAction::BulletList => transform_prefixed_lines(
            content,
            normalized_start_utf16,
            normalized_end_utf16,
            LIST_PREFIX,
        ),
        NoteTextAction::Quote => transform_prefixed_lines(
            content,
            normalized_start_utf16,
            normalized_end_utf16,
            QUOTE_PREFIX,
        ),
        NoteTextAction::Todo => transform_prefixed_lines(
            content,
            normalized_start_utf16,
            normalized_end_utf16,
            TODO_PREFIX,
        ),
        NoteTextAction::Bold => {
            transform_bold(content, normalized_start_utf16, normalized_end_utf16)
        }
    }
}

fn derive_smart_note_digest(
    title: &str,
    content: &str,
    attachment_count: usize,
) -> (String, String) {
    let explicit_title = title.trim();
    let title_lines = meaningful_note_lines(content, false);
    let preview_lines = meaningful_note_lines(content, true);
    let resolved_title = if !explicit_title.is_empty() {
        explicit_title.to_string()
    } else if let Some(first) = title_lines.first() {
        first.clone()
    } else if attachment_count > 0 {
        "图片便签".to_string()
    } else {
        "空白便签".to_string()
    };
    let resolved_preview = if !explicit_title.is_empty() {
        preview_lines.first().cloned()
    } else if preview_lines.len() > 1 {
        preview_lines.get(1).cloned()
    } else if preview_lines.first().map(String::as_str) != Some(resolved_title.as_str()) {
        preview_lines.first().cloned()
    } else {
        None
    }
    .unwrap_or_else(|| attachment_preview_text(attachment_count));

    (resolved_title, resolved_preview)
}

fn rich_text_attachment_ids(html: &str) -> Vec<String> {
    if html.trim().is_empty() {
        return Vec::new();
    }

    let mut ordered_ids = Vec::<String>::new();
    for id in quoted_attribute_values(html, "data-note-image") {
        push_unique_trimmed(&mut ordered_ids, id);
    }
    for id in note_image_src_values(html) {
        push_unique_trimmed(&mut ordered_ids, id);
    }
    ordered_ids
}

fn remove_rich_text_attachment_reference(html: &str, attachment_id: &str) -> String {
    if html.trim().is_empty() || attachment_id.trim().is_empty() {
        return html.to_string();
    }

    let without_figures = remove_matching_figure_blocks(html, attachment_id);
    let without_images = remove_matching_image_tags(&without_figures, attachment_id);
    collapse_repeated_blank_paragraphs(&without_images)
        .trim()
        .to_string()
}

fn build_note_collection_sections(
    updated_epoch_days: &[i64],
    pinned_flags: &[jboolean],
) -> Vec<i64> {
    if updated_epoch_days.len() != pinned_flags.len() || updated_epoch_days.is_empty() {
        return vec![0];
    }

    let mut encoded_sections = Vec::<Vec<i64>>::new();
    let pinned_indices = pinned_flags
        .iter()
        .enumerate()
        .filter_map(|(index, pinned)| {
            if *pinned == JNI_TRUE {
                Some(index as i64)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    if !pinned_indices.is_empty() {
        let mut section = vec![0, i64::MIN, pinned_indices.len() as i64];
        section.extend(pinned_indices);
        encoded_sections.push(section);
    }

    let mut by_day = BTreeMap::<i64, Vec<i64>>::new();
    for (index, epoch_day) in updated_epoch_days.iter().enumerate() {
        if pinned_flags[index] != JNI_TRUE {
            by_day.entry(*epoch_day).or_default().push(index as i64);
        }
    }
    for (epoch_day, indices) in by_day.iter().rev() {
        let mut section = vec![1, *epoch_day, indices.len() as i64];
        section.extend(indices.iter().copied());
        encoded_sections.push(section);
    }

    let encoded_len = 1 + encoded_sections.iter().map(Vec::len).sum::<usize>();
    let mut result = Vec::with_capacity(encoded_len);
    result.push(encoded_sections.len() as i64);
    for section in encoded_sections {
        result.extend(section);
    }
    result
}

fn normalize_slot_order(slot_ids: &[i32], min_slot_id: i32, max_slot_id: i32) -> Vec<i32> {
    if min_slot_id > max_slot_id {
        return Vec::new();
    }
    let slot_count = (max_slot_id - min_slot_id + 1) as usize;
    let mut seen = vec![false; slot_count];
    let mut normalized = Vec::with_capacity(slot_count);

    for slot_id in slot_ids.iter().copied() {
        if slot_id < min_slot_id || slot_id > max_slot_id {
            continue;
        }
        let index = (slot_id - min_slot_id) as usize;
        if !seen[index] {
            seen[index] = true;
            normalized.push(slot_id);
        }
    }

    for slot_id in min_slot_id..=max_slot_id {
        let index = (slot_id - min_slot_id) as usize;
        if !seen[index] {
            seen[index] = true;
            normalized.push(slot_id);
        }
    }

    normalized
}

fn is_valid_finance_date_key(value: &str) -> bool {
    if value.len() != 10 {
        return false;
    }
    let bytes = value.as_bytes();
    if bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
        return false;
    }
    let Some(year) = parse_fixed_digits(value, 0, 4) else {
        return false;
    };
    let Some(month) = parse_fixed_digits(value, 5, 2) else {
        return false;
    };
    let Some(day) = parse_fixed_digits(value, 8, 2) else {
        return false;
    };
    if !(1..=12).contains(&month) {
        return false;
    }
    let max_day = days_in_month(year, month);
    day >= 1 && day <= max_day
}

fn is_valid_finance_month_key(value: &str) -> bool {
    if value.len() != 7 {
        return false;
    }
    let bytes = value.as_bytes();
    if bytes.get(4) != Some(&b'-') {
        return false;
    }
    let Some(_year) = parse_fixed_digits(value, 0, 4) else {
        return false;
    };
    let Some(month) = parse_fixed_digits(value, 5, 2) else {
        return false;
    };
    (1..=12).contains(&month)
}

fn shift_finance_day_key(value: &str, offset: i64) -> Option<String> {
    let (year, month, day) = parse_finance_date_components(value)?;
    let shifted_days =
        days_from_civil(year as i64, month as i64, day as i64).checked_add(offset)?;
    let (shifted_year, shifted_month, shifted_day) = civil_from_days(shifted_days);
    if !(0..=9_999).contains(&shifted_year) {
        return None;
    }
    Some(format!(
        "{:04}-{:02}-{:02}",
        shifted_year, shifted_month, shifted_day
    ))
}

fn shift_finance_month_key(value: &str, offset: i64) -> Option<String> {
    if !is_valid_finance_month_key(value) {
        return None;
    }
    let year = parse_fixed_digits(value, 0, 4)? as i64;
    let month = parse_fixed_digits(value, 5, 2)? as i64;
    let shifted = year
        .checked_mul(12)?
        .checked_add(month - 1)?
        .checked_add(offset)?;
    if shifted < 0 {
        return None;
    }
    let shifted_year = shifted / 12;
    if shifted_year > 9_999 {
        return None;
    }
    let shifted_month = shifted % 12 + 1;
    Some(format!("{shifted_year:04}-{shifted_month:02}"))
}

fn parse_finance_date_components(value: &str) -> Option<(i32, i32, i32)> {
    if !is_valid_finance_date_key(value) {
        return None;
    }
    Some((
        parse_fixed_digits(value, 0, 4)?,
        parse_fixed_digits(value, 5, 2)?,
        parse_fixed_digits(value, 8, 2)?,
    ))
}

fn days_from_civil(mut year: i64, month: i64, day: i64) -> i64 {
    year -= if month <= 2 { 1 } else { 0 };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let adjusted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted_days = days + 719_468;
    let era = if shifted_days >= 0 {
        shifted_days
    } else {
        shifted_days - 146_096
    } / 146_097;
    let day_of_era = shifted_days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

fn parse_fixed_digits(value: &str, start: usize, len: usize) -> Option<i32> {
    let end = start.checked_add(len)?;
    let slice = value.as_bytes().get(start..end)?;
    if !slice.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let mut number = 0_i32;
    for digit in slice {
        number = number * 10 + (digit - b'0') as i32;
    }
    Some(number)
}

fn days_in_month(year: i32, month: i32) -> i32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn count_note_checklist_stats(content: &str) -> (i32, i32) {
    let mut total = 0_i32;
    let mut completed = 0_i32;

    for raw_line in content.replace("\r\n", "\n").lines() {
        let trimmed = raw_line.trim();
        if trimmed.starts_with(TODO_PREFIX) {
            total += 1;
        } else if trimmed
            .get(.."- [x] ".len())
            .map(|prefix| prefix.eq_ignore_ascii_case("- [x] "))
            .unwrap_or(false)
        {
            total += 1;
            completed += 1;
        }
    }

    (total, completed)
}

fn count_note_attachments(attachment_kind_codes: &[i32], target_kind_code: i32) -> i32 {
    attachment_kind_codes
        .iter()
        .filter(|kind_code| target_kind_code < 0 || **kind_code == target_kind_code)
        .count()
        .min(i32::MAX as usize) as i32
}

fn display_note_line(line: &str, preserve_structure: bool) -> String {
    const LEGACY_NOTE_CENTER_PREFIX: &str = "\u{3010}\u{5c45}\u{4e2d}\u{3011}";
    const DONE_PREFIX: &str = "- [x] ";

    let trimmed = line.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let plain_bold = trimmed.replace("**", "").replace("__", "");
    if let Some(centered) = plain_bold.strip_prefix(LEGACY_NOTE_CENTER_PREFIX) {
        let centered = centered.trim_start();
        if !centered.trim().is_empty() {
            return centered.to_string();
        }
    }
    if plain_bold.starts_with('[') && plain_bold.ends_with(']') && plain_bold.chars().count() > 2 {
        let centered = plain_bold
            .strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
            .map(str::trim)
            .unwrap_or("");
        if !centered.is_empty() {
            return centered.to_string();
        }
    }

    if let Some(payload) = plain_bold.strip_prefix(TODO_PREFIX) {
        let payload = payload.trim_start();
        return if preserve_structure {
            format!("\u{2610} {payload}")
        } else {
            payload.to_string()
        };
    }
    if plain_bold
        .get(..DONE_PREFIX.len())
        .map(|prefix| prefix.eq_ignore_ascii_case(DONE_PREFIX))
        .unwrap_or(false)
    {
        let payload = plain_bold[DONE_PREFIX.len()..].trim_start();
        return if preserve_structure {
            format!("\u{2611} {payload}")
        } else {
            payload.to_string()
        };
    }
    if let Some(payload) = plain_bold.strip_prefix(HEADING_PREFIX) {
        return payload.trim_start().to_string();
    }
    if let Some(payload) = plain_bold.strip_prefix(QUOTE_PREFIX) {
        let payload = payload.trim_start();
        return if preserve_structure {
            format!("\u{201c}{payload}\u{201d}")
        } else {
            payload.to_string()
        };
    }
    if let Some(payload) = plain_bold.strip_prefix(LIST_PREFIX) {
        let payload = payload.trim_start();
        return if preserve_structure {
            format!("\u{2022} {payload}")
        } else {
            payload.to_string()
        };
    }
    plain_bold
}

fn home_focus_suggestion_values(
    slot_ids: &[i32],
    blank_flags: &[i32],
    running_since_epoch_millis: &[i64],
    updated_at_epoch_millis: &[i64],
) -> Option<[i64; 2]> {
    if slot_ids.len() != blank_flags.len()
        || slot_ids.len() != running_since_epoch_millis.len()
        || slot_ids.len() != updated_at_epoch_millis.len()
        || slot_ids.is_empty()
    {
        return None;
    }

    let mut running_slot: Option<(i32, i64)> = None;
    for (index, slot_id) in slot_ids.iter().copied().enumerate() {
        let running_since = running_since_epoch_millis[index];
        if running_since < 0 {
            continue;
        }
        if running_slot
            .map(|(_, best_running_since)| running_since > best_running_since)
            .unwrap_or(true)
        {
            running_slot = Some((slot_id, running_since));
        }
    }
    if let Some((slot_id, _)) = running_slot {
        return Some([slot_id as i64, 0]);
    }

    let mut resumable_slot: Option<(i32, i64)> = None;
    for (index, slot_id) in slot_ids.iter().copied().enumerate() {
        if blank_flags[index] != 0 {
            continue;
        }
        let updated_at = updated_at_epoch_millis[index];
        if resumable_slot
            .map(|(_, best_updated_at)| updated_at > best_updated_at)
            .unwrap_or(true)
        {
            resumable_slot = Some((slot_id, updated_at));
        }
    }
    if let Some((slot_id, _)) = resumable_slot {
        return Some([slot_id as i64, 1]);
    }

    let empty_slot_id = slot_ids.iter().copied().min().unwrap_or(1);
    Some([empty_slot_id as i64, 2])
}

fn safe_elapsed_millis(
    accumulated_millis: i64,
    running_since_epoch_millis: i64,
    phase_code: i32,
    now: i64,
) -> i64 {
    let base_duration = sanitize_tracked_duration(accumulated_millis);
    let running_duration = if phase_code == MICRO_BREAK_PHASE_FOCUS {
        safe_elapsed_since(running_since_epoch_millis, now)
    } else {
        0
    };
    base_duration
        .saturating_add(running_duration)
        .min(MAX_TRACKED_DURATION_MILLIS)
}

fn push_unique_trimmed(values: &mut Vec<String>, value: String) {
    let trimmed = value.trim();
    if trimmed.is_empty() || values.iter().any(|existing| existing == trimmed) {
        return;
    }
    values.push(trimmed.to_string());
}

fn quoted_attribute_values(html: &str, attr_name: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let attr = attr_name.to_ascii_lowercase();
    let mut values = Vec::new();
    let mut search_start = 0usize;

    while let Some(relative_index) = lower[search_start..].find(&attr) {
        let attr_start = search_start + relative_index;
        if !is_attribute_name_boundary(html, attr_start, attr.len()) {
            search_start = attr_start + attr.len();
            continue;
        }

        let mut cursor = attr_start + attr.len();
        cursor = skip_ascii_whitespace(html, cursor);
        if html.as_bytes().get(cursor) != Some(&b'=') {
            search_start = cursor;
            continue;
        }
        cursor += 1;
        cursor = skip_ascii_whitespace(html, cursor);
        let quote = match html.as_bytes().get(cursor) {
            Some(b'\'') => b'\'',
            Some(b'"') => b'"',
            _ => {
                search_start = cursor;
                continue;
            }
        };
        cursor += 1;
        let value_start = cursor;
        while cursor < html.len() && html.as_bytes()[cursor] != quote {
            cursor += 1;
        }
        if cursor <= html.len() {
            values.push(html[value_start..cursor].to_string());
        }
        search_start = cursor.saturating_add(1);
    }

    values
}

fn note_image_src_values(html: &str) -> Vec<String> {
    const NOTE_IMAGE_PREFIX: &str = "note-image://";
    let lower = html.to_ascii_lowercase();
    let mut values = Vec::new();
    let mut search_start = 0usize;

    while let Some(relative_index) = lower[search_start..].find(NOTE_IMAGE_PREFIX) {
        let value_start = search_start + relative_index + NOTE_IMAGE_PREFIX.len();
        let mut cursor = value_start;
        while cursor < html.len() && !is_note_image_src_delimiter(html.as_bytes()[cursor]) {
            cursor += 1;
        }
        values.push(html[value_start..cursor].to_string());
        search_start = cursor;
    }

    values
}

fn remove_matching_figure_blocks(html: &str, attachment_id: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut output = String::with_capacity(html.len());
    let mut cursor = 0usize;

    while let Some(relative_start) = lower[cursor..].find("<figure") {
        let figure_start = cursor + relative_start;
        let Some(open_end) = html[figure_start..]
            .find('>')
            .map(|offset| figure_start + offset + 1)
        else {
            break;
        };
        let opening_tag = &html[figure_start..open_end];
        let matches_attachment = first_quoted_attribute_value(opening_tag, "data-note-image")
            .map(|value| value.trim() == attachment_id)
            .unwrap_or(false);

        if matches_attachment {
            let close_search_start = open_end;
            if let Some(relative_close) = lower[close_search_start..].find("</figure>") {
                output.push_str(&html[cursor..figure_start]);
                let close_end = close_search_start + relative_close + "</figure>".len();
                cursor = skip_ascii_whitespace(html, close_end);
                continue;
            }
        }

        output.push_str(&html[cursor..open_end]);
        cursor = open_end;
    }

    output.push_str(&html[cursor..]);
    output
}

fn remove_matching_image_tags(html: &str, attachment_id: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut output = String::with_capacity(html.len());
    let mut cursor = 0usize;

    while let Some(relative_start) = lower[cursor..].find("<img") {
        let image_start = cursor + relative_start;
        let Some(tag_end) = html[image_start..]
            .find('>')
            .map(|offset| image_start + offset + 1)
        else {
            break;
        };
        let image_tag = &html[image_start..tag_end];
        let matches_attachment = first_quoted_attribute_value(image_tag, "src")
            .and_then(|value| value.strip_prefix("note-image://").map(str::to_string))
            .map(|id| id.trim() == attachment_id)
            .unwrap_or(false);

        if matches_attachment {
            output.push_str(&html[cursor..image_start]);
            cursor = skip_ascii_whitespace(html, tag_end);
        } else {
            output.push_str(&html[cursor..tag_end]);
            cursor = tag_end;
        }
    }

    output.push_str(&html[cursor..]);
    output
}

fn first_quoted_attribute_value(tag: &str, attr_name: &str) -> Option<String> {
    quoted_attribute_values(tag, attr_name).into_iter().next()
}

fn collapse_repeated_blank_paragraphs(html: &str) -> String {
    let mut output = String::with_capacity(html.len());
    let mut cursor = 0usize;

    while cursor < html.len() {
        if let Some(first_end) = parse_blank_paragraph(html, cursor) {
            let mut end = first_end;
            let mut count = 1usize;
            while let Some(next_end) = parse_blank_paragraph(html, end) {
                end = next_end;
                count += 1;
            }
            if count > 1 {
                output.push_str("<p><br></p>");
            } else {
                output.push_str(&html[cursor..first_end]);
            }
            cursor = end;
        } else {
            let ch = html[cursor..].chars().next().unwrap_or_default();
            output.push(ch);
            cursor += ch.len_utf8();
        }
    }

    output
}

fn parse_blank_paragraph(html: &str, start: usize) -> Option<usize> {
    let mut cursor = skip_ascii_whitespace(html, start);
    cursor = consume_ascii_case_insensitive(html, cursor, "<p>")?;
    cursor = skip_ascii_whitespace(html, cursor);
    if let Some(after_break) = consume_blank_break(html, cursor) {
        cursor = skip_ascii_whitespace(html, after_break);
    }
    cursor = consume_ascii_case_insensitive(html, cursor, "</p>")?;
    Some(skip_ascii_whitespace(html, cursor))
}

fn consume_blank_break(html: &str, start: usize) -> Option<usize> {
    let mut cursor = consume_ascii_case_insensitive(html, start, "<br")?;
    cursor = skip_ascii_whitespace(html, cursor);
    if html.as_bytes().get(cursor) == Some(&b'/') {
        cursor += 1;
        cursor = skip_ascii_whitespace(html, cursor);
    }
    if html.as_bytes().get(cursor) == Some(&b'>') {
        Some(cursor + 1)
    } else {
        None
    }
}

fn consume_ascii_case_insensitive(html: &str, start: usize, token: &str) -> Option<usize> {
    let end = start.checked_add(token.len())?;
    if end <= html.len() && html[start..end].eq_ignore_ascii_case(token) {
        Some(end)
    } else {
        None
    }
}

fn skip_ascii_whitespace(value: &str, mut index: usize) -> usize {
    while index < value.len() && value.as_bytes()[index].is_ascii_whitespace() {
        index += 1;
    }
    index
}

fn is_attribute_name_boundary(value: &str, start: usize, len: usize) -> bool {
    let before = if start == 0 {
        true
    } else {
        !is_attribute_name_char(value.as_bytes()[start - 1])
    };
    let after_index = start + len;
    let after = if after_index >= value.len() {
        true
    } else {
        !is_attribute_name_char(value.as_bytes()[after_index])
    };
    before && after
}

fn is_attribute_name_char(value: u8) -> bool {
    value.is_ascii_alphanumeric() || value == b'-' || value == b'_' || value == b':'
}

fn is_note_image_src_delimiter(value: u8) -> bool {
    value == b'"' || value == b'\'' || value == b'>' || value.is_ascii_whitespace()
}

fn attachment_preview_text(attachment_count: usize) -> String {
    match attachment_count {
        0 => "写下今天最想记住的事，回来时就能直接接上。".to_string(),
        1 => "包含 1 张图片".to_string(),
        count => format!("包含 {count} 张图片"),
    }
}

fn meaningful_note_lines(content: &str, preserve_structure: bool) -> Vec<String> {
    content
        .replace("\r\n", "\n")
        .lines()
        .map(|line| smart_note_line(line, preserve_structure))
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

fn smart_note_line(line: &str, preserve_structure: bool) -> String {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let plain_bold = trimmed.replace(BOLD_MARK, "").replace("__", "");
    if let Some(centered) = centered_payload(&plain_bold) {
        return centered;
    }

    if let Some(payload) = plain_bold.strip_prefix(TODO_PREFIX) {
        let payload = payload.trim_start();
        return if preserve_structure {
            format!("☐ {payload}")
        } else {
            payload.to_string()
        };
    }
    const DONE_PREFIX: &str = "- [x] ";
    if plain_bold
        .get(..DONE_PREFIX.len())
        .map(|prefix| prefix.eq_ignore_ascii_case(DONE_PREFIX))
        .unwrap_or(false)
    {
        let payload = plain_bold[DONE_PREFIX.len()..].trim_start();
        return if preserve_structure {
            format!("☑ {payload}")
        } else {
            payload.to_string()
        };
    }
    if let Some(payload) = plain_bold.strip_prefix(HEADING_PREFIX) {
        return payload.trim_start().to_string();
    }
    if let Some(payload) = plain_bold.strip_prefix(QUOTE_PREFIX) {
        let payload = payload.trim_start();
        return if preserve_structure {
            format!("“{payload}”")
        } else {
            payload.to_string()
        };
    }
    if let Some(payload) = plain_bold.strip_prefix(LIST_PREFIX) {
        let payload = payload.trim_start();
        return if preserve_structure {
            format!("• {payload}")
        } else {
            payload.to_string()
        };
    }
    plain_bold
}

fn centered_payload(line: &str) -> Option<String> {
    if let Some(payload) = line.strip_prefix(CENTER_PREFIX) {
        let payload = payload.trim_start();
        if !payload.is_empty() {
            return Some(payload.to_string());
        }
    }
    if line.starts_with(CENTER_WRAP_START)
        && line.ends_with(CENTER_WRAP_END)
        && line.len() > CENTER_WRAP_START.len() + CENTER_WRAP_END.len()
    {
        let end_index = line.len() - CENTER_WRAP_END.len();
        let payload = line[CENTER_WRAP_START.len()..end_index].trim();
        if !payload.is_empty() {
            return Some(payload.to_string());
        }
    }
    None
}

fn transform_prefixed_lines(
    content: &str,
    selection_start_utf16: usize,
    selection_end_utf16: usize,
    prefix: &str,
) -> NoteTextEdit {
    let (block_start, block_end) =
        line_block_range(content, selection_start_utf16, selection_end_utf16);
    let original_block = &content[block_start..block_end];
    let lines: Vec<&str> = original_block.split('\n').collect();
    let removing = !lines.is_empty() && lines.iter().all(|line| line.starts_with(prefix));
    let transformed_block = lines
        .iter()
        .map(|line| {
            if removing {
                line.strip_prefix(prefix).unwrap_or(line).to_string()
            } else {
                format!("{prefix}{line}")
            }
        })
        .collect::<Vec<String>>()
        .join("\n");

    let mut updated_content = String::with_capacity(content.len() + transformed_block.len());
    updated_content.push_str(&content[..block_start]);
    updated_content.push_str(&transformed_block);
    updated_content.push_str(&content[block_end..]);

    if selection_start_utf16 != selection_end_utf16 {
        let updated_selection_start = byte_to_utf16_index(&updated_content, block_start);
        let updated_selection_end =
            byte_to_utf16_index(&updated_content, block_start + transformed_block.len());
        return NoteTextEdit {
            content: updated_content,
            selection_start_utf16: updated_selection_start,
            selection_end_utf16: updated_selection_end,
        };
    }

    let start_byte = utf16_to_byte_index(content, selection_start_utf16);
    let line_offset = start_byte.saturating_sub(block_start);
    let current_line_start = original_block[..line_offset.min(original_block.len())]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let cursor_byte = if removing {
        block_start + current_line_start
    } else {
        block_start + current_line_start + prefix.len()
    };
    let cursor_utf16 =
        byte_to_utf16_index(&updated_content, cursor_byte.min(updated_content.len()));
    NoteTextEdit {
        content: updated_content,
        selection_start_utf16: cursor_utf16,
        selection_end_utf16: cursor_utf16,
    }
}

fn transform_centered_lines(
    content: &str,
    selection_start_utf16: usize,
    selection_end_utf16: usize,
) -> NoteTextEdit {
    let (block_start, block_end) =
        line_block_range(content, selection_start_utf16, selection_end_utf16);
    let original_block = &content[block_start..block_end];
    let lines: Vec<&str> = original_block.split('\n').collect();
    let removing = !lines.is_empty() && lines.iter().all(|line| is_centered_line(line));
    let transformed_lines: Vec<String> = lines
        .iter()
        .map(|line| {
            if removing {
                unwrap_centered_line(line)
            } else {
                wrap_centered_line(line)
            }
        })
        .collect();
    let transformed_block = transformed_lines.join("\n");

    let mut updated_content = String::with_capacity(content.len() + transformed_block.len());
    updated_content.push_str(&content[..block_start]);
    updated_content.push_str(&transformed_block);
    updated_content.push_str(&content[block_end..]);

    if selection_start_utf16 != selection_end_utf16 {
        let updated_selection_start = byte_to_utf16_index(&updated_content, block_start);
        let updated_selection_end =
            byte_to_utf16_index(&updated_content, block_start + transformed_block.len());
        return NoteTextEdit {
            content: updated_content,
            selection_start_utf16: updated_selection_start,
            selection_end_utf16: updated_selection_end,
        };
    }

    let start_byte = utf16_to_byte_index(content, selection_start_utf16);
    let line_offset = start_byte.saturating_sub(block_start);
    let current_line_index = original_block[..line_offset.min(original_block.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count();
    let current_line_start = original_block[..line_offset.min(original_block.len())]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let cursor_offset_in_line = line_offset.saturating_sub(current_line_start);
    let original_line = lines.get(current_line_index).copied().unwrap_or_default();
    let transformed_line = transformed_lines
        .get(current_line_index)
        .map(String::as_str)
        .unwrap_or_default();
    let cursor_offset = if removing {
        if cursor_offset_in_line <= CENTER_WRAP_START.len() {
            0
        } else if cursor_offset_in_line >= original_line.len() {
            transformed_line.len()
        } else {
            cursor_offset_in_line
                .saturating_sub(CENTER_WRAP_START.len())
                .min(transformed_line.len())
        }
    } else {
        (cursor_offset_in_line + CENTER_WRAP_START.len()).min(transformed_line.len())
    };
    let cursor_byte = block_start + current_line_start + cursor_offset;
    let cursor_utf16 =
        byte_to_utf16_index(&updated_content, cursor_byte.min(updated_content.len()));
    NoteTextEdit {
        content: updated_content,
        selection_start_utf16: cursor_utf16,
        selection_end_utf16: cursor_utf16,
    }
}

fn is_centered_line(line: &str) -> bool {
    line.starts_with(CENTER_WRAP_START)
        && line.ends_with(CENTER_WRAP_END)
        && line.len() >= CENTER_WRAP_START.len() + CENTER_WRAP_END.len()
}

fn wrap_centered_line(line: &str) -> String {
    format!("{CENTER_WRAP_START}{line}{CENTER_WRAP_END}")
}

fn unwrap_centered_line(line: &str) -> String {
    if is_centered_line(line) {
        line[CENTER_WRAP_START.len()..line.len() - CENTER_WRAP_END.len()].to_string()
    } else {
        line.to_string()
    }
}

fn transform_bold(
    content: &str,
    selection_start_utf16: usize,
    selection_end_utf16: usize,
) -> NoteTextEdit {
    let selection_start = utf16_to_byte_index(content, selection_start_utf16);
    let selection_end = utf16_to_byte_index(content, selection_end_utf16);

    if selection_start_utf16 == selection_end_utf16 {
        let mut updated_content = String::with_capacity(content.len() + BOLD_MARK.len() * 2);
        updated_content.push_str(&content[..selection_start]);
        updated_content.push_str(BOLD_MARK);
        updated_content.push_str(BOLD_MARK);
        updated_content.push_str(&content[selection_end..]);
        let cursor_byte = selection_start + BOLD_MARK.len();
        let cursor_utf16 = byte_to_utf16_index(&updated_content, cursor_byte);
        return NoteTextEdit {
            content: updated_content,
            selection_start_utf16: cursor_utf16,
            selection_end_utf16: cursor_utf16,
        };
    }

    let has_leading_mark = selection_start
        .checked_sub(BOLD_MARK.len())
        .and_then(|start| content.get(start..selection_start))
        .map(|prefix| prefix == BOLD_MARK)
        .unwrap_or(false);
    let has_trailing_mark = selection_end
        .checked_add(BOLD_MARK.len())
        .and_then(|end| content.get(selection_end..end))
        .map(|suffix| suffix == BOLD_MARK)
        .unwrap_or(false);

    if has_leading_mark && has_trailing_mark {
        let mut updated_content =
            String::with_capacity(content.len().saturating_sub(BOLD_MARK.len() * 2));
        updated_content.push_str(&content[..selection_start - BOLD_MARK.len()]);
        updated_content.push_str(&content[selection_start..selection_end]);
        updated_content.push_str(&content[selection_end + BOLD_MARK.len()..]);
        let updated_start_byte = selection_start - BOLD_MARK.len();
        let updated_end_byte = selection_end - BOLD_MARK.len();
        let updated_selection_start = byte_to_utf16_index(&updated_content, updated_start_byte);
        let updated_selection_end = byte_to_utf16_index(&updated_content, updated_end_byte);
        return NoteTextEdit {
            content: updated_content,
            selection_start_utf16: updated_selection_start,
            selection_end_utf16: updated_selection_end,
        };
    }

    let mut updated_content = String::with_capacity(content.len() + BOLD_MARK.len() * 2);
    updated_content.push_str(&content[..selection_start]);
    updated_content.push_str(BOLD_MARK);
    updated_content.push_str(&content[selection_start..selection_end]);
    updated_content.push_str(BOLD_MARK);
    updated_content.push_str(&content[selection_end..]);

    let updated_start_byte = selection_start + BOLD_MARK.len();
    let updated_end_byte = selection_end + BOLD_MARK.len();
    let updated_selection_start = byte_to_utf16_index(&updated_content, updated_start_byte);
    let updated_selection_end = byte_to_utf16_index(&updated_content, updated_end_byte);
    NoteTextEdit {
        content: updated_content,
        selection_start_utf16: updated_selection_start,
        selection_end_utf16: updated_selection_end,
    }
}

fn line_block_range(
    content: &str,
    selection_start_utf16: usize,
    selection_end_utf16: usize,
) -> (usize, usize) {
    if content.is_empty() {
        return (0, 0);
    }
    let start_byte = utf16_to_byte_index(content, selection_start_utf16);
    let end_byte = utf16_to_byte_index(content, selection_end_utf16);
    let block_start = line_start(content, start_byte);
    let inclusive_end_byte = if selection_start_utf16 == selection_end_utf16 {
        start_byte.min(content.len().saturating_sub(1))
    } else if end_byte > start_byte && content.as_bytes()[end_byte - 1] == b'\n' {
        end_byte - 1
    } else {
        previous_char_start(content, end_byte).max(start_byte)
    };
    let block_end = line_end(content, inclusive_end_byte);
    (block_start, block_end)
}

fn line_start(content: &str, byte_index: usize) -> usize {
    if content.is_empty() || byte_index == 0 {
        return 0;
    }
    let search_index = byte_index
        .saturating_sub(1)
        .min(content.len().saturating_sub(1));
    content[..=search_index]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0)
}

fn line_end(content: &str, byte_index: usize) -> usize {
    if content.is_empty() {
        return 0;
    }
    let search_index = byte_index.min(content.len());
    content[search_index..]
        .find('\n')
        .map(|index| search_index + index)
        .unwrap_or(content.len())
}

fn previous_char_start(content: &str, byte_index: usize) -> usize {
    let clamped = byte_index.min(content.len());
    if clamped == 0 {
        return 0;
    }
    content[..clamped]
        .char_indices()
        .last()
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn utf16_to_byte_index(content: &str, utf16_index: usize) -> usize {
    let mut consumed_utf16 = 0;
    for (byte_index, ch) in content.char_indices() {
        if consumed_utf16 >= utf16_index {
            return byte_index;
        }
        consumed_utf16 += ch.len_utf16();
        if consumed_utf16 >= utf16_index {
            return byte_index + ch.len_utf8();
        }
    }
    content.len()
}

fn byte_to_utf16_index(content: &str, byte_index: usize) -> usize {
    let clamped = byte_index.min(content.len());
    let mut utf16_index = 0;
    for (index, ch) in content.char_indices() {
        if index >= clamped {
            break;
        }
        utf16_index += ch.len_utf16();
    }
    utf16_index
}

fn normalize_query(query: &str) -> Vec<String> {
    query
        .trim()
        .to_lowercase()
        .split_whitespace()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

fn rank_knowledge_source_indices(
    query: &str,
    titles: &[String],
    bodies: &[String],
    folders: &[String],
) -> Vec<jint> {
    let source_count = titles.len().min(bodies.len());
    if source_count == 0 {
        return Vec::new();
    }
    let keywords = normalize_knowledge_keywords(query);
    let mut scored = Vec::<(i64, usize)>::new();
    for index in 0..source_count {
        let title = titles[index].to_lowercase();
        let body = bodies[index].to_lowercase();
        let folder = folders
            .get(index)
            .map(|value| value.to_lowercase())
            .unwrap_or_default();
        let mut score = if keywords.is_empty() { 1 } else { 0 };
        for keyword in &keywords {
            let weight = if keyword.chars().count() > 1 { 4 } else { 1 };
            if title.contains(keyword) {
                score += 24 * weight;
            }
            if folder.contains(keyword) {
                score += 10 * weight;
            }
            let body_hits = body.matches(keyword).take(6).count() as i64;
            score += body_hits * 4 * weight;
        }
        if score > 0 {
            scored.push((score, index));
        }
    }
    scored.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    scored
        .into_iter()
        .map(|(_, index)| index as jint)
        .collect()
}

fn normalize_knowledge_keywords(query: &str) -> Vec<String> {
    let lowered = query.trim().to_lowercase();
    let mut keywords = normalize_query(&lowered);
    let cjk_chars = lowered
        .chars()
        .filter(|ch| is_cjk(*ch) && !is_common_cjk_stopword(*ch))
        .collect::<Vec<_>>();
    for window in cjk_chars.windows(2) {
        keywords.push(window.iter().collect());
    }
    for ch in cjk_chars {
        keywords.push(ch.to_string());
    }
    keywords.sort();
    keywords.dedup();
    keywords
        .into_iter()
        .filter(|keyword| !keyword.trim().is_empty())
        .collect()
}

fn is_cjk(ch: char) -> bool {
    ('\u{4E00}'..='\u{9FFF}').contains(&ch)
        || ('\u{3400}'..='\u{4DBF}').contains(&ch)
        || ('\u{F900}'..='\u{FAFF}').contains(&ch)
}

fn is_common_cjk_stopword(ch: char) -> bool {
    matches!(
        ch,
        '的' | '了' | '是' | '在' | '和' | '或' | '与' | '吗' | '呢' | '啊' | '这' | '那'
            | '我' | '你' | '他' | '她' | '它' | '们' | '一' | '个' | '怎' | '么'
    )
}

fn history_query_matches(haystack: &str, keywords: &[String]) -> bool {
    if keywords.is_empty() {
        return true;
    }
    let normalized_haystack = haystack.to_lowercase();
    keywords
        .iter()
        .all(|keyword| normalized_haystack.contains(keyword))
}

fn render_note_document_html(payload: NoteHtmlPayload) -> String {
    let accent = note_accent_color(&payload.accent_seed);
    let mut body_sections = String::new();

    for block in payload.blocks {
        match block.block_type.as_str() {
            "TEXT" => {
                let text = block.text.unwrap_or_default();
                if text.trim().is_empty() {
                    continue;
                }
                let html = if payload.rich_text_enabled {
                    text
                } else if payload.markdown_enabled {
                    markdown_to_html(&text)
                } else {
                    plain_text_to_html(&text)
                };
                body_sections.push_str("<section class=\"note-block note-block-text\">");
                body_sections.push_str(&html);
                body_sections.push_str("</section>");
            }
            "IMAGE" => {
                if let Some(attachment_id) = block.attachment_id {
                    body_sections.push_str("<section class=\"note-block note-block-image\">");
                    body_sections.push_str(&format!(
                        "<img class=\"note-image\" src=\"note-image://{}\" alt=\"{}\" />",
                        escape_html(&attachment_id),
                        escape_html(block.caption.as_deref().unwrap_or("image"))
                    ));
                    if let Some(caption) = block.caption {
                        if !caption.trim().is_empty() {
                            body_sections.push_str(&format!(
                                "<div class=\"note-caption\">{}</div>",
                                escape_html(caption.trim())
                            ));
                        }
                    }
                    body_sections.push_str("</section>");
                }
            }
            "CONTACT" => {
                body_sections.push_str("<section class=\"note-block note-block-chip\">");
                body_sections.push_str("<div class=\"chip-title\">联系人</div>");
                let contact_name = block.contact_name.unwrap_or_default();
                if !contact_name.trim().is_empty() {
                    body_sections.push_str(&format!(
                        "<div class=\"chip-main\">{}</div>",
                        escape_html(contact_name.trim())
                    ));
                }
                let org = block.contact_organization.unwrap_or_default();
                if !org.trim().is_empty() {
                    body_sections.push_str(&format!(
                        "<div class=\"chip-sub\">{}</div>",
                        escape_html(org.trim())
                    ));
                }
                if let Some(phones) = block.contact_phones {
                    for phone in phones {
                        let number = phone.number.unwrap_or_default();
                        if number.trim().is_empty() {
                            continue;
                        }
                        let label = phone.label.unwrap_or_default();
                        let phone_text = if label.trim().is_empty() {
                            number.trim().to_owned()
                        } else {
                            format!("{} {}", label.trim(), number.trim())
                        };
                        body_sections.push_str(&format!(
                            "<div class=\"chip-row\">{}</div>",
                            escape_html(&phone_text)
                        ));
                    }
                }
                if let Some(caption) = block.caption {
                    if !caption.trim().is_empty() {
                        body_sections.push_str(&format!(
                            "<div class=\"chip-sub\">{}</div>",
                            escape_html(caption.trim())
                        ));
                    }
                }
                body_sections.push_str("</section>");
            }
            "CALL" => {
                body_sections.push_str("<section class=\"note-block note-block-chip\">");
                body_sections.push_str("<div class=\"chip-title\">通话速记</div>");
                let call_name = block.call_contact_name.unwrap_or_default();
                let call_number = block.call_phone_number.unwrap_or_default();
                let headline = if !call_name.trim().is_empty() {
                    call_name.trim().to_owned()
                } else if !call_number.trim().is_empty() {
                    call_number.trim().to_owned()
                } else {
                    "未记录号码".to_owned()
                };
                body_sections.push_str(&format!(
                    "<div class=\"chip-main\">{}</div>",
                    escape_html(&headline)
                ));
                let mut meta_parts = Vec::<String>::new();
                if !call_number.trim().is_empty() && call_name.trim().is_empty() {
                    meta_parts.push(call_number.trim().to_owned());
                }
                if let Some(direction) = block.call_direction {
                    let direction_label = match direction.as_str() {
                        "ONGOING" => "通话中",
                        "INCOMING" => "呼入",
                        "OUTGOING" => "呼出",
                        "MISSED" => "未接",
                        _ => "",
                    };
                    if !direction_label.is_empty() {
                        meta_parts.push(direction_label.to_owned());
                    }
                }
                if let Some(label) = block.call_occurred_at_label {
                    if !label.trim().is_empty() {
                        meta_parts.push(label.trim().to_owned());
                    }
                }
                if let Some(label) = block.call_duration_label {
                    if !label.trim().is_empty() {
                        meta_parts.push(label.trim().to_owned());
                    }
                }
                if !meta_parts.is_empty() {
                    body_sections.push_str(&format!(
                        "<div class=\"chip-sub\">{}</div>",
                        escape_html(&meta_parts.join(" · "))
                    ));
                }
                if let Some(text) = block.text {
                    if !text.trim().is_empty() {
                        body_sections.push_str(&format!(
                            "<div class=\"chip-row\">{}</div>",
                            escape_html(text.trim())
                        ));
                    }
                }
                body_sections.push_str("</section>");
            }
            _ => {}
        }
    }

    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"/><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"/><style>{}</style></head><body><main class=\"note-shell\"><header class=\"note-header\"><div class=\"note-kicker\"></div><h1>{}</h1><p class=\"note-meta\">{}</p></header><div class=\"note-body\">{}</div></main></body></html>",
        note_html_css(accent),
        escape_html(payload.title.trim()),
        escape_html(payload.meta.trim()),
        body_sections
    )
}

fn note_html_css(accent: &str) -> String {
    format!(
        ":root{{--bg:#f7f6f3;--card:#ffffff;--text:#2c2c2c;--sub:#6b6b6b;--line:rgba(44,44,44,.08);--accent:{accent};font-family:'SF Pro Text','PingFang SC','Noto Sans SC','Microsoft YaHei',sans-serif;}}*{{box-sizing:border-box;}}body{{margin:0;background:radial-gradient(circle at top left,rgba(255,255,255,.92),transparent 48%),linear-gradient(180deg,#faf8f3 0%,var(--bg) 100%);color:var(--text);padding:24px;}}.note-shell{{max-width:900px;margin:0 auto;background:rgba(255,255,255,.76);backdrop-filter:blur(18px) saturate(160%);border-radius:28px;box-shadow:0 18px 48px rgba(0,0,0,.12);border:1px solid rgba(255,255,255,.7);padding:32px;}}.note-kicker{{width:52px;height:6px;border-radius:999px;background:var(--accent);box-shadow:0 2px 8px rgba(0,0,0,.12);margin-bottom:18px;}}h1{{margin:0;font-size:32px;line-height:1.2;letter-spacing:.04em;}}.note-meta{{margin:12px 0 0;color:var(--sub);font-size:14px;line-height:1.7;}}.note-body{{margin-top:28px;display:grid;gap:18px;}}.note-block{{background:var(--card);border:1px solid var(--line);border-radius:22px;box-shadow:0 8px 22px rgba(0,0,0,.06);padding:20px 22px;overflow:hidden;}}.note-block-text h1,.note-block-text h2,.note-block-text h3,.note-block-text h4{{margin:0 0 12px;line-height:1.3;}}.note-block-text p,.note-block-text li,.note-block-text blockquote,.note-block-text div{{margin:0 0 10px;line-height:1.85;font-size:16px;}}.note-block-text ul,.note-block-text ol{{margin:0;padding-left:20px;}}.note-block-text blockquote{{padding:10px 14px;border-left:4px solid var(--accent);background:rgba(0,0,0,.02);border-radius:0 16px 16px 0;color:var(--sub);}}.note-block-text [data-align='center']{{text-align:center;}}.note-block-text a{{color:var(--accent);text-decoration:none;}}.note-block-text strong,.note-block-text b{{font-weight:700;}}.note-block-text em,.note-block-text i{{font-style:italic;}}.note-block-text u{{text-decoration:underline;}}.note-block-text s,.note-block-text strike,.note-block-text del{{text-decoration:line-through;}}.note-block-text ul[data-note-todo='true']{{list-style:none;padding-left:0;display:grid;gap:10px;}}.note-block-text ul[data-note-todo='true'] li{{position:relative;padding-left:28px;margin:0;}}.note-block-text ul[data-note-todo='true'] li::before{{content:'☐';position:absolute;left:0;top:0;color:var(--accent);font-weight:700;}}.note-block-text ul[data-note-todo='true'] li[data-done='true']{{color:var(--sub);text-decoration:line-through;}}.note-block-text ul[data-note-todo='true'] li[data-done='true']::before{{content:'☑';}}.note-center,.note-center>*{{text-align:center;}}.note-center>*:last-child{{margin-bottom:0;}}.note-block-image{{padding:14px;}}.note-image{{display:block;width:100%;border-radius:18px;object-fit:cover;}}.note-caption{{margin-top:12px;font-size:14px;line-height:1.7;color:var(--sub);}}.note-block-chip{{display:grid;gap:8px;}}.chip-title{{font-size:12px;font-weight:700;letter-spacing:.08em;color:var(--accent);text-transform:uppercase;}}.chip-main{{font-size:22px;font-weight:700;line-height:1.35;}}.chip-sub,.chip-row{{font-size:15px;line-height:1.8;color:var(--sub);}}@media (max-width:720px){{body{{padding:14px;}}.note-shell{{padding:20px 18px;border-radius:24px;}}h1{{font-size:26px;}}.note-block{{padding:18px;}}}}"
    )
}

fn render_rich_text_body(content: &str, markdown_enabled: bool) -> String {
    if content.trim().is_empty() {
        return String::new();
    }
    if markdown_enabled {
        markdown_to_html(content)
    } else {
        plain_text_to_rich_html(content)
    }
}

fn markdown_to_html(markdown: &str) -> String {
    let normalized = markdown.replace("\r\n", "\n");
    let mut sections = Vec::<String>::new();
    let mut markdown_buffer = Vec::<String>::new();

    for line in normalized.lines() {
        if let Some(centered_markdown) = extract_centered_markdown(line) {
            flush_markdown_buffer(&mut markdown_buffer, &mut sections);
            let centered_html = render_standard_markdown(centered_markdown);
            sections.push(format!("<div class=\"note-center\">{centered_html}</div>"));
        } else {
            markdown_buffer.push(line.to_string());
        }
    }

    flush_markdown_buffer(&mut markdown_buffer, &mut sections);
    sections.join("")
}

fn extract_centered_markdown(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if is_centered_line(trimmed) {
        Some(trimmed[CENTER_WRAP_START.len()..trimmed.len() - CENTER_WRAP_END.len()].trim())
    } else {
        None
    }
}

fn flush_markdown_buffer(markdown_buffer: &mut Vec<String>, sections: &mut Vec<String>) {
    if markdown_buffer.is_empty() {
        return;
    }
    let joined = markdown_buffer.join("\n");
    let rendered = render_standard_markdown(&joined);
    if !rendered.trim().is_empty() {
        sections.push(rendered);
    }
    markdown_buffer.clear();
}

fn render_standard_markdown(markdown: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(markdown, options);
    let mut rendered = String::new();
    html::push_html(&mut rendered, parser);
    rendered
}

fn plain_text_to_html(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| format!("<p>{}</p>", escape_html(line.trim())))
        .collect::<Vec<String>>()
        .join("")
}

fn plain_text_to_rich_html(text: &str) -> String {
    text.lines()
        .map(|line| {
            if line.trim().is_empty() {
                "<p><br/></p>".to_owned()
            } else {
                format!("<p>{}</p>", escape_html(line.trim()))
            }
        })
        .collect::<Vec<String>>()
        .join("")
}

fn note_accent_color(seed: &str) -> &'static str {
    match seed {
        "red" => "#c0392b",
        "blue" => "#2980b9",
        "green" => "#27ae60",
        "teal" => "#1abc9c",
        _ => "#e67e22",
    }
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\"', "&quot;")
        .replace('\'', "&#39;")
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_notifications_NativeNotificationBridge_nativeFormatNotificationDuration(
    mut env: JNIEnv,
    _class: JClass,
    duration_millis: jlong,
) -> jstring {
    new_java_string(&mut env, &format_notification_duration(duration_millis))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeComputeMicroBreakTargetMillis(
    _env: JNIEnv,
    _class: JClass,
    slot_id: jint,
    cycle_index: jint,
) -> jlong {
    compute_micro_break_target_millis(slot_id, cycle_index)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeResolveMicroBreak(
    env: JNIEnv,
    _class: JClass,
    now: jlong,
    slot_id: jint,
    accumulated_millis: jlong,
    running_since_epoch_millis: jlong,
    phase_code: jint,
    cycle_index: jint,
    phase_progress_millis: jlong,
    updated_at: jlong,
) -> jlongArray {
    let Some(resolution) = resolve_micro_break(
        now,
        slot_id,
        accumulated_millis,
        running_since_epoch_millis,
        phase_code,
        cycle_index,
        phase_progress_millis,
        updated_at,
    ) else {
        return long_array_from_slice(env, &[]);
    };
    long_array_from_slice(env, &micro_break_resolution_values(&resolution))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativePauseMicroBreak(
    env: JNIEnv,
    _class: JClass,
    now: jlong,
    slot_id: jint,
    accumulated_millis: jlong,
    running_since_epoch_millis: jlong,
    phase_code: jint,
    cycle_index: jint,
    phase_progress_millis: jlong,
    updated_at: jlong,
) -> jlongArray {
    let Some(values) = pause_micro_break(
        now,
        slot_id,
        accumulated_millis,
        running_since_epoch_millis,
        phase_code,
        cycle_index,
        phase_progress_millis,
        updated_at,
    ) else {
        return long_array_from_slice(env, &[]);
    };
    long_array_from_slice(env, &values)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFinanceStructureHeadline(
    mut env: JNIEnv,
    _class: JClass,
    income_total: jlong,
    expense_total: jlong,
    net_cashflow: jlong,
) -> jstring {
    new_java_string(
        &mut env,
        finance_structure_headline(income_total, expense_total, net_cashflow),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFinanceIncomeKindLabel(
    mut env: JNIEnv,
    _class: JClass,
    kind_code: jint,
) -> jstring {
    new_java_string(&mut env, finance_income_kind_label(kind_code))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFinanceNamedAmountKindLabel(
    mut env: JNIEnv,
    _class: JClass,
    kind_code: jint,
) -> jstring {
    new_java_string(&mut env, finance_named_amount_kind_label(kind_code))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFinanceDefaultTemplateKindCodes(
    env: JNIEnv,
    _class: JClass,
    template_kind_code: jint,
) -> jintArray {
    int_array_from_slice(env, finance_default_template_kind_codes(template_kind_code))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFinanceAssetKindCodes(
    env: JNIEnv,
    _class: JClass,
) -> jintArray {
    int_array_from_slice(env, finance_asset_kind_codes())
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFinanceLiabilityKindCodes(
    env: JNIEnv,
    _class: JClass,
) -> jintArray {
    int_array_from_slice(env, finance_liability_kind_codes())
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeSanitizeFinanceDraft(
    mut env: JNIEnv,
    _class: JClass,
    value: JString,
    max_length: jint,
) -> jstring {
    let value: String = match env.get_string(&value) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    new_java_string(
        &mut env,
        &sanitize_finance_draft(&value, max_length.max(0) as usize),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeParseFinanceAmountDraft(
    mut env: JNIEnv,
    _class: JClass,
    value: JString,
    max_digits: jint,
    max_amount: jlong,
) -> jlong {
    let value: String = match env.get_string(&value) {
        Ok(value) => value.into(),
        Err(_) => return 0,
    };
    parse_finance_amount_draft(&value, max_digits.max(0) as usize, max_amount.max(0))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeParseFinanceTargetShare(
    mut env: JNIEnv,
    _class: JClass,
    value: JString,
) -> jfloat {
    let value: String = match env.get_string(&value) {
        Ok(value) => value.into(),
        Err(_) => return f32::NAN,
    };
    parse_finance_target_share(&value).unwrap_or(f32::NAN)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFinanceTargetPercentText(
    mut env: JNIEnv,
    _class: JClass,
    value: jfloat,
) -> jstring {
    new_java_string(&mut env, &finance_target_percent_text(value))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFinanceScaleLong(
    _env: JNIEnv,
    _class: JClass,
    period_code: jint,
    amount: jlong,
    mode_code: jint,
) -> jlong {
    finance_scale_long(period_code, amount, mode_code)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFinanceScaleFloat(
    _env: JNIEnv,
    _class: JClass,
    period_code: jint,
    value: jfloat,
    mode_code: jint,
) -> jfloat {
    finance_scale_float(period_code, value, mode_code)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFinanceBucketShare(
    _env: JNIEnv,
    _class: JClass,
    bucket_code: jint,
    share_kind_code: jint,
    income_total: jlong,
    expense_total: jlong,
    debt_total: jlong,
    food_total: jlong,
    btc_total: jlong,
    living_total: jlong,
    learning_total: jlong,
    other_total: jlong,
) -> jfloat {
    finance_bucket_share(
        bucket_code,
        share_kind_code,
        income_total,
        expense_total,
        debt_total,
        food_total,
        btc_total,
        living_total,
        learning_total,
        other_total,
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFinanceSettingsTargetSummary(
    mut env: JNIEnv,
    _class: JClass,
    bucket_codes: JIntArray,
    labels: JObjectArray,
    target_shares: JFloatArray,
) -> jstring {
    let Some(bucket_values) = int_array_values(&env, &bucket_codes) else {
        return std::ptr::null_mut();
    };
    let Some(label_values) = string_array_values(&mut env, &labels) else {
        return std::ptr::null_mut();
    };
    let Some(target_values) = float_array_values(&env, &target_shares) else {
        return std::ptr::null_mut();
    };
    if bucket_values.len() != label_values.len() || bucket_values.len() != target_values.len() {
        return std::ptr::null_mut();
    }
    new_java_string(
        &mut env,
        &finance_settings_target_summary(&bucket_values, &label_values, &target_values),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFormatFinanceCurrency(
    mut env: JNIEnv,
    _class: JClass,
    amount: jlong,
) -> jstring {
    new_java_string(&mut env, &format_finance_currency(amount))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFormatFinanceSignedCurrency(
    mut env: JNIEnv,
    _class: JClass,
    amount: jlong,
) -> jstring {
    new_java_string(&mut env, &format_finance_signed_currency(amount))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFormatFinancePercent(
    mut env: JNIEnv,
    _class: JClass,
    value: jfloat,
) -> jstring {
    new_java_string(&mut env, &format_finance_percent(value))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFinanceBucketRatioLine(
    mut env: JNIEnv,
    _class: JClass,
    income_share: jfloat,
    expense_share: jfloat,
    target_share: jfloat,
) -> jstring {
    new_java_string(
        &mut env,
        &finance_bucket_ratio_line(income_share, expense_share, target_share),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFinanceBucketDriftLine(
    mut env: JNIEnv,
    _class: JClass,
    income_share: jfloat,
    target_share: jfloat,
) -> jstring {
    new_java_string(
        &mut env,
        &finance_bucket_drift_line(income_share, target_share),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeIsFinanceBucketDriftHealthy(
    _env: JNIEnv,
    _class: JClass,
    bucket_code: jint,
    income_share: jfloat,
    target_share: jfloat,
) -> jboolean {
    boolean_to_jni(is_finance_bucket_drift_healthy(
        bucket_code,
        income_share,
        target_share,
    ))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeMatchesNoteQuickFilter(
    _env: JNIEnv,
    _class: JClass,
    filter_code: jint,
    pinned: jboolean,
    todo_remaining: jint,
    image_attachment_count: jint,
    has_folder: jboolean,
) -> jboolean {
    boolean_to_jni(matches_note_quick_filter(
        filter_code,
        pinned == JNI_TRUE,
        todo_remaining,
        image_attachment_count,
        has_folder == JNI_TRUE,
    ))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFormatCurrencyAmount(
    mut env: JNIEnv,
    _class: JClass,
    amount: jlong,
    prefix: JString,
) -> jstring {
    let prefix: String = match env.get_string(&prefix) {
        Ok(value) => value.into(),
        Err(_) => String::new(),
    };
    new_java_string(&mut env, &format_currency_amount(amount, &prefix))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFormatSignedCurrencyAmount(
    mut env: JNIEnv,
    _class: JClass,
    amount: jlong,
    prefix: JString,
) -> jstring {
    let prefix: String = match env.get_string(&prefix) {
        Ok(value) => value.into(),
        Err(_) => String::new(),
    };
    new_java_string(&mut env, &format_signed_currency_amount(amount, &prefix))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFormatRoundedPercent(
    mut env: JNIEnv,
    _class: JClass,
    value: jfloat,
) -> jstring {
    new_java_string(&mut env, &format_rounded_percent(value))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFormatSignedPercent(
    mut env: JNIEnv,
    _class: JClass,
    value: jfloat,
) -> jstring {
    new_java_string(&mut env, &format_signed_percent(value))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFormatDefensiveCoverage(
    mut env: JNIEnv,
    _class: JClass,
    coverage: jfloat,
    unit_label: JString,
) -> jstring {
    let unit_label: String = match env.get_string(&unit_label) {
        Ok(value) => value.into(),
        Err(_) => String::new(),
    };
    new_java_string(&mut env, &format_defensive_coverage(coverage, &unit_label))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFormatAssetLiabilityRatio(
    mut env: JNIEnv,
    _class: JClass,
    asset_total: jlong,
    liability_total: jlong,
) -> jstring {
    new_java_string(
        &mut env,
        &format_asset_liability_ratio(asset_total, liability_total),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFinanceCoverageCaption(
    mut env: JNIEnv,
    _class: JClass,
    period_code: jint,
) -> jstring {
    new_java_string(&mut env, finance_coverage_caption(period_code))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFormatLongDuration(
    mut env: JNIEnv,
    _class: JClass,
    duration_millis: jlong,
) -> jstring {
    new_java_string(&mut env, &format_long_duration(duration_millis))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFormatCompactTimerDuration(
    mut env: JNIEnv,
    _class: JClass,
    duration_millis: jlong,
) -> jstring {
    new_java_string(&mut env, &format_compact_timer_duration(duration_millis))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFormatCompactDurationLabel(
    mut env: JNIEnv,
    _class: JClass,
    duration_millis: jlong,
) -> jstring {
    new_java_string(&mut env, &format_compact_duration_label(duration_millis))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeTimerTileTodayStatsLabel(
    mut env: JNIEnv,
    _class: JClass,
    today_duration: jlong,
    today_total_duration: jlong,
) -> jstring {
    new_java_string(
        &mut env,
        &timer_tile_today_stats_label(today_duration, today_total_duration),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFormatCountdownDuration(
    mut env: JNIEnv,
    _class: JClass,
    duration_millis: jlong,
) -> jstring {
    new_java_string(&mut env, &format_countdown_duration(duration_millis))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeSlotLabel(
    mut env: JNIEnv,
    _class: JClass,
    slot_id: jint,
) -> jstring {
    new_java_string(&mut env, &slot_label(slot_id))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeTimerSlotDisplayTitle(
    mut env: JNIEnv,
    _class: JClass,
    title: JString,
    slot_id: jint,
) -> jstring {
    let title: String = match env.get_string(&title) {
        Ok(value) => value.into(),
        Err(_) => String::new(),
    };
    new_java_string(&mut env, &timer_slot_display_title(&title, slot_id))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeArchivedTaskDisplayTitle(
    mut env: JNIEnv,
    _class: JClass,
    title: JString,
    original_slot_id: jint,
) -> jstring {
    let title: String = match env.get_string(&title) {
        Ok(value) => value.into(),
        Err(_) => String::new(),
    };
    new_java_string(
        &mut env,
        &archived_task_display_title(&title, original_slot_id),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeLatestFinishBadgeLabel(
    mut env: JNIEnv,
    _class: JClass,
    moment_label: JString,
) -> jstring {
    let moment_label: String = match env.get_string(&moment_label) {
        Ok(value) => value.into(),
        Err(_) => String::new(),
    };
    new_java_string(&mut env, &latest_finish_badge_label(&moment_label))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeLongestSessionBadgeLabel(
    mut env: JNIEnv,
    _class: JClass,
    duration_millis: jlong,
) -> jstring {
    new_java_string(&mut env, &longest_session_badge_label(duration_millis))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeBusiestSlotBadgeLabel(
    mut env: JNIEnv,
    _class: JClass,
    slot_id: jint,
    duration_millis: jlong,
) -> jstring {
    new_java_string(
        &mut env,
        &busiest_slot_badge_label(slot_id, duration_millis),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeArchivedTaskCountBadgeLabel(
    mut env: JNIEnv,
    _class: JClass,
    count: jint,
) -> jstring {
    new_java_string(&mut env, &archived_task_count_badge_label(count))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeHistoryWeekSummaryNote(
    mut env: JNIEnv,
    _class: JClass,
    archived_task_count: jint,
) -> jstring {
    new_java_string(&mut env, &history_week_summary_note(archived_task_count))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeBuildHistorySearchHaystack(
    mut env: JNIEnv,
    _class: JClass,
    fields: JObjectArray,
) -> jstring {
    let fields = match string_array_values(&mut env, &fields) {
        Some(values) => values,
        None => return std::ptr::null_mut(),
    };
    new_java_string(&mut env, &build_history_search_haystack(&fields))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeElapsedBadgeLabel(
    mut env: JNIEnv,
    _class: JClass,
    duration_millis: jlong,
) -> jstring {
    new_java_string(&mut env, &elapsed_badge_label(duration_millis))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeTodayLogSummaryBadgeLabel(
    mut env: JNIEnv,
    _class: JClass,
    duration_millis: jlong,
    log_count: jint,
) -> jstring {
    new_java_string(
        &mut env,
        &today_log_summary_badge_label(duration_millis, log_count),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeHomeRhythmNarrative(
    mut env: JNIEnv,
    _class: JClass,
    active_count: jint,
    today_session_count: jint,
    today_total: jlong,
    latest_moment_label: JString,
    latest_slot_title: JString,
    latest_duration: jlong,
    peak_duration: jlong,
    focus_kind_code: jint,
    focus_slot_title: JString,
    busiest_slot_title: JString,
    busiest_duration: jlong,
) -> jstring {
    let latest_moment_label: String = env
        .get_string(&latest_moment_label)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    let latest_slot_title: String = env
        .get_string(&latest_slot_title)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    let focus_slot_title: String = env
        .get_string(&focus_slot_title)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    let busiest_slot_title: String = env
        .get_string(&busiest_slot_title)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    new_java_string(
        &mut env,
        &timer_insights::home_rhythm_narrative(
            active_count,
            today_session_count,
            today_total,
            &latest_moment_label,
            &latest_slot_title,
            latest_duration,
            peak_duration,
            focus_kind_code,
            &focus_slot_title,
            &busiest_slot_title,
            busiest_duration,
        ),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeHomeFocusGuidance(
    mut env: JNIEnv,
    _class: JClass,
    focus_kind_code: jint,
    slot_id: jint,
    elapsed_millis: jlong,
    has_title: jboolean,
    has_note: jboolean,
) -> jstring {
    new_java_string(
        &mut env,
        &timer_insights::home_focus_guidance(
            focus_kind_code,
            slot_id,
            elapsed_millis,
            has_title == JNI_TRUE,
            has_note == JNI_TRUE,
        ),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeHeaderMetricValue(
    mut env: JNIEnv,
    _class: JClass,
    metric_code: jint,
    active_count: jint,
    today_total: jlong,
    archived_count: jint,
) -> jstring {
    new_java_string(
        &mut env,
        &timer_insights::header_metric_value(
            metric_code,
            active_count,
            today_total,
            archived_count,
        ),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeHeaderMetricNote(
    mut env: JNIEnv,
    _class: JClass,
    metric_code: jint,
    active_count: jint,
    today_session_count: jint,
    archived_count: jint,
) -> jstring {
    new_java_string(
        &mut env,
        &timer_insights::header_metric_note(
            metric_code,
            active_count,
            today_session_count,
            archived_count,
        ),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeTimerSlotStatusLabel(
    mut env: JNIEnv,
    _class: JClass,
    is_running: jboolean,
    is_on_break: jboolean,
    phase_remaining_millis: jlong,
    running_since_label: JString,
    is_blank_slate: jboolean,
    today_duration: jlong,
    latest_finish_label: JString,
) -> jstring {
    let running_since_label: String = env
        .get_string(&running_since_label)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    let latest_finish_label: String = env
        .get_string(&latest_finish_label)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    new_java_string(
        &mut env,
        &timer_insights::timer_slot_status_label(
            is_running == JNI_TRUE,
            is_on_break == JNI_TRUE,
            phase_remaining_millis,
            &running_since_label,
            is_blank_slate == JNI_TRUE,
            today_duration,
            &latest_finish_label,
        ),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeTimerSlotFooterLabel(
    mut env: JNIEnv,
    _class: JClass,
    note: JString,
    is_running: jboolean,
    is_on_break: jboolean,
    can_archive: jboolean,
    session_count: jint,
    today_duration: jlong,
    is_blank_slate: jboolean,
) -> jstring {
    let note: String = env
        .get_string(&note)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    new_java_string(
        &mut env,
        &timer_insights::timer_slot_footer_label(
            &note,
            is_running == JNI_TRUE,
            is_on_break == JNI_TRUE,
            can_archive == JNI_TRUE,
            session_count,
            today_duration,
            is_blank_slate == JNI_TRUE,
        ),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeTimerDetailStatusLine(
    mut env: JNIEnv,
    _class: JClass,
    is_running: jboolean,
    is_on_break: jboolean,
    phase_remaining_millis: jlong,
    can_archive: jboolean,
) -> jstring {
    new_java_string(
        &mut env,
        &timer_insights::timer_detail_status_line(
            is_running == JNI_TRUE,
            is_on_break == JNI_TRUE,
            phase_remaining_millis,
            can_archive == JNI_TRUE,
        ),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeTimerDetailSummaryNote(
    mut env: JNIEnv,
    _class: JClass,
    summary_kind_code: jint,
    is_running: jboolean,
    is_on_break: jboolean,
    can_archive: jboolean,
    today_session_count: jint,
    moment_label: JString,
) -> jstring {
    let moment_label: String = env
        .get_string(&moment_label)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    new_java_string(
        &mut env,
        &timer_insights::timer_detail_summary_note(
            summary_kind_code,
            is_running == JNI_TRUE,
            is_on_break == JNI_TRUE,
            can_archive == JNI_TRUE,
            today_session_count,
            &moment_label,
        ),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeHistoryEmptyMessage(
    mut env: JNIEnv,
    _class: JClass,
    has_search_query: jboolean,
    has_archived_tasks: jboolean,
) -> jstring {
    new_java_string(
        &mut env,
        &timer_insights::history_empty_message(
            has_search_query == JNI_TRUE,
            has_archived_tasks == JNI_TRUE,
        ),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeArchivedTaskDetailText(
    mut env: JNIEnv,
    _class: JClass,
    archived_at_label: JString,
    original_slot_id: jint,
) -> jstring {
    let archived_at_label: String = env
        .get_string(&archived_at_label)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    new_java_string(
        &mut env,
        &timer_insights::archived_task_detail_text(&archived_at_label, original_slot_id),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeArchivedTaskRestoreTargetText(
    mut env: JNIEnv,
    _class: JClass,
    restore_target_slot_id: jint,
) -> jstring {
    new_java_string(
        &mut env,
        &timer_insights::archived_task_restore_target_text(restore_target_slot_id),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeArchivedTaskRestoreActionLabel(
    mut env: JNIEnv,
    _class: JClass,
    restore_target_slot_id: jint,
    original_slot_id: jint,
) -> jstring {
    new_java_string(
        &mut env,
        &timer_insights::archived_task_restore_action_label(
            restore_target_slot_id,
            original_slot_id,
        ),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeDiagnosticLogButtonLabel(
    mut env: JNIEnv,
    _class: JClass,
    is_preparing_log: jboolean,
    recording_active: jboolean,
) -> jstring {
    new_java_string(
        &mut env,
        &timer_insights::diagnostic_log_button_label(
            is_preparing_log == JNI_TRUE,
            recording_active == JNI_TRUE,
        ),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeDiagnosticDataExportButtonLabel(
    mut env: JNIEnv,
    _class: JClass,
    is_exporting_data: jboolean,
) -> jstring {
    new_java_string(
        &mut env,
        &timer_insights::diagnostic_data_export_button_label(is_exporting_data == JNI_TRUE),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeDiagnosticDataExportStatusMessage(
    mut env: JNIEnv,
    _class: JClass,
    status_kind_code: jint,
) -> jstring {
    new_java_string(
        &mut env,
        &timer_insights::diagnostic_data_export_status_message(status_kind_code),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFinanceDataStatusMessage(
    mut env: JNIEnv,
    _class: JClass,
    status_kind_code: jint,
    file_name: JString,
) -> jstring {
    let file_name: String = env
        .get_string(&file_name)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    new_java_string(
        &mut env,
        &timer_insights::finance_data_status_message(status_kind_code, &file_name),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFinanceDataActionLabel(
    mut env: JNIEnv,
    _class: JClass,
    is_exporting: jboolean,
) -> jstring {
    new_java_string(
        &mut env,
        &timer_insights::finance_data_action_label(is_exporting == JNI_TRUE),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFinanceBackupFileName(
    mut env: JNIEnv,
    _class: JClass,
    version_name: JString,
    timestamp_label: JString,
) -> jstring {
    let version_name: String = match env.get_string(&version_name) {
        Ok(value) => value.into(),
        Err(_) => String::new(),
    };
    let timestamp_label: String = match env.get_string(&timestamp_label) {
        Ok(value) => value.into(),
        Err(_) => String::new(),
    };
    new_java_string(
        &mut env,
        &timer_insights::finance_backup_file_name(&version_name, &timestamp_label),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFinanceDataIdleStatusMessage(
    mut env: JNIEnv,
    _class: JClass,
    version_name: JString,
) -> jstring {
    let version_name: String = match env.get_string(&version_name) {
        Ok(value) => value.into(),
        Err(_) => String::new(),
    };
    new_java_string(
        &mut env,
        &timer_insights::finance_data_idle_status_message(&version_name),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeFinanceMonthlyLedgerStateValues(
    env: JNIEnv,
    _class: JClass,
    asset_count: jint,
    liability_count: jint,
    asset_draft_count: jint,
    liability_draft_count: jint,
    days_with_entries: jlong,
    net_cashflow: jlong,
    previous_snapshot_exists: jboolean,
    can_fill_asset_templates: jboolean,
    can_fill_liability_templates: jboolean,
) -> jlongArray {
    let values = timer_insights::finance_monthly_ledger_state_values(
        asset_count,
        liability_count,
        asset_draft_count,
        liability_draft_count,
        days_with_entries,
        net_cashflow,
        previous_snapshot_exists == JNI_TRUE,
        can_fill_asset_templates == JNI_TRUE,
        can_fill_liability_templates == JNI_TRUE,
    );
    long_array_from_slice(env, &values)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeDiagnosticStatusMessage(
    mut env: JNIEnv,
    _class: JClass,
    explicit_status: JString,
    recording_active: jboolean,
    has_crash_report: jboolean,
    recording_started_at_label: JString,
) -> jstring {
    let explicit_status: String = env
        .get_string(&explicit_status)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    let recording_started_at_label: String = env
        .get_string(&recording_started_at_label)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    new_java_string(
        &mut env,
        &timer_insights::diagnostic_status_message(
            &explicit_status,
            recording_active == JNI_TRUE,
            has_crash_report == JNI_TRUE,
            &recording_started_at_label,
        ),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeDiagnosticShareStatusMessage(
    mut env: JNIEnv,
    _class: JClass,
    file_name: JString,
    share_opened: jboolean,
) -> jstring {
    let file_name: String = env
        .get_string(&file_name)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    new_java_string(
        &mut env,
        &timer_insights::diagnostic_share_status_message(&file_name, share_opened == JNI_TRUE),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeDiagnosticRecordingStartedStatusMessage(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    new_java_string(
        &mut env,
        &timer_insights::diagnostic_recording_started_status_message(),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeDiagnosticRecordingExportedStatusMessage(
    mut env: JNIEnv,
    _class: JClass,
    file_name: JString,
) -> jstring {
    let file_name: String = env
        .get_string(&file_name)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    new_java_string(
        &mut env,
        &timer_insights::diagnostic_recording_exported_status_message(&file_name),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeHomeDockBoardSummaryLabel(
    mut env: JNIEnv,
    _class: JClass,
    active_count: jint,
    active_label: JString,
    today_label: JString,
) -> jstring {
    let active_label: String = env
        .get_string(&active_label)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    let today_label: String = env
        .get_string(&today_label)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    new_java_string(
        &mut env,
        &timer_insights::home_dock_board_summary_label(active_count, &active_label, &today_label),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeHomeDockBoardSummaryValue(
    mut env: JNIEnv,
    _class: JClass,
    active_count: jint,
    today_total: jlong,
    today_total_label: JString,
) -> jstring {
    let today_total_label: String = env
        .get_string(&today_total_label)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    new_java_string(
        &mut env,
        &timer_insights::home_dock_board_summary_value(
            active_count,
            today_total,
            &today_total_label,
        ),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeHomeDockHistorySummaryLabel(
    mut env: JNIEnv,
    _class: JClass,
    finance_has_entries: jboolean,
    latest_session_exists: jboolean,
    coverage_label: JString,
    recent_label: JString,
    archive_label: JString,
) -> jstring {
    let coverage_label: String = env
        .get_string(&coverage_label)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    let recent_label: String = env
        .get_string(&recent_label)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    let archive_label: String = env
        .get_string(&archive_label)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    new_java_string(
        &mut env,
        &timer_insights::home_dock_history_summary_label(
            finance_has_entries == JNI_TRUE,
            latest_session_exists == JNI_TRUE,
            &coverage_label,
            &recent_label,
            &archive_label,
        ),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeHomeDockHistorySummaryValue(
    mut env: JNIEnv,
    _class: JClass,
    finance_has_entries: jboolean,
    latest_session_exists: jboolean,
    coverage_value: JString,
    latest_session_time_label: JString,
    archived_task_count: jint,
) -> jstring {
    let coverage_value: String = env
        .get_string(&coverage_value)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    let latest_session_time_label: String = env
        .get_string(&latest_session_time_label)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    new_java_string(
        &mut env,
        &timer_insights::home_dock_history_summary_value(
            finance_has_entries == JNI_TRUE,
            latest_session_exists == JNI_TRUE,
            &coverage_value,
            &latest_session_time_label,
            archived_task_count,
        ),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeNoteUpdatedLabel(
    mut env: JNIEnv,
    _class: JClass,
    is_today: jboolean,
    time_label: JString,
    date_label: JString,
) -> jstring {
    let time_label: String = env
        .get_string(&time_label)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    let date_label: String = env
        .get_string(&date_label)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    new_java_string(
        &mut env,
        &timer_insights::note_updated_label(is_today == JNI_TRUE, &time_label, &date_label),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeShareableNoteText(
    mut env: JNIEnv,
    _class: JClass,
    title: JString,
    content: JString,
) -> jstring {
    let title: String = env
        .get_string(&title)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    let content: String = env
        .get_string(&content)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    new_java_string(
        &mut env,
        &timer_insights::shareable_note_text(&title, &content),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeRestoreIntoSlotLabel(
    mut env: JNIEnv,
    _class: JClass,
    slot_id: jint,
) -> jstring {
    new_java_string(&mut env, &restore_into_slot_label(slot_id))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeSlotReferenceText(
    mut env: JNIEnv,
    _class: JClass,
    slot_id: jint,
) -> jstring {
    new_java_string(&mut env, &slot_reference_text(slot_id))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeIsTimerSlotBlankSlate(
    mut env: JNIEnv,
    _class: JClass,
    title: JString,
    has_category: jboolean,
    note: JString,
    accumulated_millis: jlong,
    running_since_epoch_millis: jlong,
    phase_code: jint,
    cycle_index: jint,
    phase_progress_millis: jlong,
) -> jboolean {
    let title: String = env
        .get_string(&title)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    let note: String = env
        .get_string(&note)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    boolean_to_jni(is_timer_slot_blank_slate(
        &title,
        has_category == JNI_TRUE,
        &note,
        accumulated_millis,
        running_since_epoch_millis,
        phase_code,
        cycle_index,
        phase_progress_millis,
    ))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeRestoreTargetSlotId(
    env: JNIEnv,
    _class: JClass,
    original_slot_id: jint,
    slot_ids: JIntArray,
    blank_flags: JBooleanArray,
) -> jint {
    let Some(slot_id_values) = int_array_values(&env, &slot_ids) else {
        return -1;
    };
    let Some(blank_values) = boolean_array_values(&env, &blank_flags) else {
        return -1;
    };
    restore_target_slot_id(original_slot_id, &slot_id_values, &blank_values).unwrap_or(-1)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeShiftFinanceDayKey(
    mut env: JNIEnv,
    _class: JClass,
    value: JString,
    offset: jlong,
) -> jstring {
    let value: String = match env.get_string(&value) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match shift_finance_day_key(&value, offset) {
        Some(shifted) => new_java_string(&mut env, &shifted),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeShiftFinanceMonthKey(
    mut env: JNIEnv,
    _class: JClass,
    value: JString,
    offset: jlong,
) -> jstring {
    let value: String = match env.get_string(&value) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match shift_finance_month_key(&value, offset) {
        Some(shifted) => new_java_string(&mut env, &shifted),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeHasFinanceIncomeDraftContent(
    mut env: JNIEnv,
    _class: JClass,
    name: JString,
    amount: jlong,
    note: JString,
) -> jboolean {
    let name: String = env
        .get_string(&name)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    let note: String = env
        .get_string(&note)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    boolean_to_jni(has_finance_row_draft_content(&name, amount, Some(&note)))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeHasFinanceExpenseDraftContent(
    mut env: JNIEnv,
    _class: JClass,
    name: JString,
    amount: jlong,
    note: JString,
) -> jboolean {
    let name: String = env
        .get_string(&name)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    let note: String = env
        .get_string(&note)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    boolean_to_jni(has_finance_row_draft_content(&name, amount, Some(&note)))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeHasFinanceNamedAmountDraftContent(
    mut env: JNIEnv,
    _class: JClass,
    name: JString,
    amount: jlong,
) -> jboolean {
    let name: String = env
        .get_string(&name)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    boolean_to_jni(has_finance_row_draft_content(&name, amount, None))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeNormalizeRepositoryText(
    mut env: JNIEnv,
    _class: JClass,
    value: JString,
    max_length: jint,
    trim_start_only: jboolean,
    compact_whitespace: jboolean,
) -> jstring {
    let value: String = env
        .get_string(&value)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    new_java_string(
        &mut env,
        &normalize_repository_text(
            &value,
            max_length.max(0) as usize,
            trim_start_only == JNI_TRUE,
            compact_whitespace == JNI_TRUE,
        ),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeMissingExistingAttachmentIndices(
    mut env: JNIEnv,
    _class: JClass,
    existing_ids: JObjectArray,
    incoming_ids: JObjectArray,
) -> jintArray {
    let Some(existing_values) = string_array_values(&mut env, &existing_ids) else {
        return int_array_from_slice(env, &[]);
    };
    let Some(incoming_values) = string_array_values(&mut env, &incoming_ids) else {
        return int_array_from_slice(env, &[]);
    };
    int_array_from_slice(
        env,
        &missing_existing_attachment_indices(&existing_values, &incoming_values),
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeShouldCaptureNoteRevision(
    _env: JNIEnv,
    _class: JClass,
    structure_changed: jboolean,
    metadata_changed: jboolean,
    last_captured_at_epoch_millis: jlong,
    timestamp: jlong,
) -> jboolean {
    boolean_to_jni(should_capture_note_revision(
        structure_changed == JNI_TRUE,
        metadata_changed == JNI_TRUE,
        if last_captured_at_epoch_millis >= 0 {
            Some(last_captured_at_epoch_millis)
        } else {
            None
        },
        timestamp,
    ))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeBuildNoteRevisionPlan(
    mut env: JNIEnv,
    _class: JClass,
    revision_ids: JObjectArray,
    captured_at_epoch_millis: JLongArray,
    matches_previous_flags: JBooleanArray,
    previous_exists: jboolean,
    should_capture: jboolean,
    snapshot_id: JString,
    max_count: jint,
) -> jintArray {
    let Some(ids) = string_array_values(&mut env, &revision_ids) else {
        return int_array_from_slice(env, &[]);
    };
    let Some(captured_at_values) = long_array_values(&env, &captured_at_epoch_millis) else {
        return int_array_from_slice(env, &[]);
    };
    let Some(matches_previous_values) = boolean_array_values(&env, &matches_previous_flags) else {
        return int_array_from_slice(env, &[]);
    };
    let snapshot_id = env
        .get_string(&snapshot_id)
        .ok()
        .map(String::from)
        .unwrap_or_default();
    let plan = build_note_revision_plan(
        &ids,
        &captured_at_values,
        &matches_previous_values,
        previous_exists == JNI_TRUE,
        should_capture == JNI_TRUE,
        &snapshot_id,
        max_count.max(0) as usize,
    );
    int_array_from_slice(env, &plan)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeSortNoteIndices(
    mut env: JNIEnv,
    _class: JClass,
    sort_mode_code: jint,
    pinned_flags: JBooleanArray,
    created_at_epoch_millis: JLongArray,
    updated_at_epoch_millis: JLongArray,
    titles: JObjectArray,
) -> jintArray {
    let Some(pinned_values) = boolean_array_values(&env, &pinned_flags) else {
        return int_array_from_slice(env, &[]);
    };
    let Some(created_values) = long_array_values(&env, &created_at_epoch_millis) else {
        return int_array_from_slice(env, &[]);
    };
    let Some(updated_values) = long_array_values(&env, &updated_at_epoch_millis) else {
        return int_array_from_slice(env, &[]);
    };
    let Some(title_values) = string_array_values(&mut env, &titles) else {
        return int_array_from_slice(env, &[]);
    };
    let Some(indices) = sort_note_indices(
        sort_mode_code,
        &pinned_values,
        &created_values,
        &updated_values,
        &title_values,
    ) else {
        return int_array_from_slice(env, &[]);
    };
    int_array_from_slice(env, &indices)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeSortTrashedNoteIndices(
    env: JNIEnv,
    _class: JClass,
    deleted_at_epoch_millis: JLongArray,
    updated_at_epoch_millis: JLongArray,
) -> jintArray {
    let Some(deleted_values) = long_array_values(&env, &deleted_at_epoch_millis) else {
        return int_array_from_slice(env, &[]);
    };
    let Some(updated_values) = long_array_values(&env, &updated_at_epoch_millis) else {
        return int_array_from_slice(env, &[]);
    };
    let Some(indices) = sort_trashed_note_indices(&deleted_values, &updated_values) else {
        return int_array_from_slice(env, &[]);
    };
    int_array_from_slice(env, &indices)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeBuildTimerSessionUiIndexStats(
    env: JNIEnv,
    _class: JClass,
    session_slot_ids: JIntArray,
    session_ended_at_epoch_millis: JLongArray,
    session_duration_millis: JLongArray,
    archived_original_slot_ids: JIntArray,
    slot_ids: JIntArray,
) -> jlongArray {
    let Some(session_slot_id_values) = int_array_values(&env, &session_slot_ids) else {
        return long_array_from_slice(env, &[]);
    };
    let Some(session_ended_at_values) = long_array_values(&env, &session_ended_at_epoch_millis)
    else {
        return long_array_from_slice(env, &[]);
    };
    let Some(session_duration_values) = long_array_values(&env, &session_duration_millis) else {
        return long_array_from_slice(env, &[]);
    };
    let Some(archived_slot_id_values) = int_array_values(&env, &archived_original_slot_ids) else {
        return long_array_from_slice(env, &[]);
    };
    let Some(slot_id_values) = int_array_values(&env, &slot_ids) else {
        return long_array_from_slice(env, &[]);
    };

    let Some(stats) = build_timer_session_ui_index_stats(
        &session_slot_id_values,
        &session_ended_at_values,
        &session_duration_values,
        &archived_slot_id_values,
        &slot_id_values,
    ) else {
        return long_array_from_slice(env, &[]);
    };
    long_array_from_slice(env, &stats)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeResolveTimerSlotDropTargetId(
    env: JNIEnv,
    _class: JClass,
    slot_ids: JIntArray,
    candidate_bounds: JFloatArray,
    candidate_translations: JFloatArray,
    dragged_bounds: JFloatArray,
    pointer_position: JFloatArray,
    dragged_center: JFloatArray,
    excluded_slot_id: jint,
) -> jint {
    let slot_id_values = match int_array_values(&env, &slot_ids) {
        Some(values) if !values.is_empty() => values,
        _ => return -1,
    };
    let candidate_bounds = match float_array_values(&env, &candidate_bounds) {
        Some(values) => values,
        None => return -1,
    };
    let candidate_translations = match float_array_values(&env, &candidate_translations) {
        Some(values) => values,
        None => return -1,
    };
    let dragged_bounds = match float_array_values(&env, &dragged_bounds)
        .and_then(|values| NativeRect::from_slice(&values))
    {
        Some(values) => values,
        None => return -1,
    };
    let pointer_position = match float_array_values(&env, &pointer_position)
        .and_then(|values| NativePoint::from_slice(&values))
    {
        Some(values) => values,
        None => return -1,
    };
    let dragged_center = match float_array_values(&env, &dragged_center)
        .and_then(|values| NativePoint::from_slice(&values))
    {
        Some(values) => values,
        None => return -1,
    };

    if candidate_bounds.len() != slot_id_values.len() * 4
        || candidate_translations.len() != slot_id_values.len() * 2
    {
        return -1;
    }

    let candidates = slot_id_values
        .iter()
        .enumerate()
        .filter_map(|(index, slot_id)| {
            let bounds_offset = index * 4;
            let translation_offset = index * 2;
            let bounds =
                NativeRect::from_slice(&candidate_bounds[bounds_offset..bounds_offset + 4])?;
            let translation = NativePoint::from_slice(
                &candidate_translations[translation_offset..translation_offset + 2],
            )?;
            Some(TimerSlotDropTargetCandidate {
                slot_id: *slot_id,
                bounds: bounds.translated_by(translation),
            })
        })
        .collect::<Vec<_>>();

    if candidates.len() != slot_id_values.len() {
        return -1;
    }

    resolve_timer_slot_drop_target_id(
        &candidates,
        dragged_bounds,
        pointer_position,
        dragged_center,
        excluded_slot_id,
    )
    .unwrap_or(-1)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeResolveTimerSlotInsertIndex(
    env: JNIEnv,
    _class: JClass,
    slot_indices: JIntArray,
    candidate_bounds: JFloatArray,
    current_insert_index: jint,
    dragged_center_x: jfloat,
    dragged_center_y: jfloat,
    auto_scrolling: jboolean,
    now_uptime_ms: jlong,
    last_switch_uptime_ms: jlong,
    minimum_hysteresis_px: jfloat,
) -> jint {
    let slot_indices = match int_array_values(&env, &slot_indices) {
        Some(values) if !values.is_empty() => values,
        _ => return current_insert_index,
    };
    let candidate_bounds = match float_array_values(&env, &candidate_bounds) {
        Some(values) if values.len() == slot_indices.len() * 4 => values,
        _ => return current_insert_index,
    };
    let candidates = slot_indices
        .iter()
        .enumerate()
        .filter_map(|(index, slot_index)| {
            let bounds_offset = index * 4;
            let bounds =
                NativeRect::from_slice(&candidate_bounds[bounds_offset..bounds_offset + 4])?;
            Some(TimerSlotInsertCandidate {
                slot_index: *slot_index,
                bounds,
            })
        })
        .collect::<Vec<_>>();

    if candidates.len() != slot_indices.len() {
        return current_insert_index;
    }

    resolve_timer_slot_insert_index(
        &candidates,
        current_insert_index,
        NativePoint {
            x: dragged_center_x,
            y: dragged_center_y,
        },
        auto_scrolling != JNI_FALSE,
        now_uptime_ms,
        last_switch_uptime_ms,
        minimum_hysteresis_px,
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeStabilizeTimerSlotInsertIndex(
    env: JNIEnv,
    _class: JClass,
    current_insert_index: jint,
    raw_insert_index: jint,
    pending_insert_index: jint,
    pending_insert_index_since_uptime_ms: jlong,
    now_uptime_ms: jlong,
    hold_millis: jlong,
) -> jlongArray {
    let result = stabilize_timer_slot_insert_index(
        current_insert_index,
        raw_insert_index,
        if pending_insert_index >= 0 {
            Some(pending_insert_index)
        } else {
            None
        },
        pending_insert_index_since_uptime_ms,
        now_uptime_ms,
        hold_millis,
    );
    long_array_from_slice(
        env,
        &[
            result.insert_index as jlong,
            result.pending_insert_index.unwrap_or(-1) as jlong,
            result.pending_insert_index_since_uptime_ms,
        ],
    )
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeReorderTimerSlotIds(
    env: JNIEnv,
    _class: JClass,
    base_slot_order: JIntArray,
    dragged_slot_id: jint,
    insert_index: jint,
) -> jintArray {
    let base_slot_order = match int_array_values(&env, &base_slot_order) {
        Some(values) => values,
        None => return empty_int_array_from_env(env),
    };
    let reordered = reorder_timer_slot_ids(&base_slot_order, dragged_slot_id, insert_index)
        .unwrap_or(base_slot_order);
    int_array_from_slice(env, &reordered)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeTimerSlotDisplacementTargetPairs(
    env: JNIEnv,
    _class: JClass,
    display_slot_order: JIntArray,
    preview_slot_order: JIntArray,
    dragged_slot_id: jint,
) -> jintArray {
    let display_slot_order = match int_array_values(&env, &display_slot_order) {
        Some(values) => values,
        None => return empty_int_array_from_env(env),
    };
    let preview_slot_order = match int_array_values(&env, &preview_slot_order) {
        Some(values) => values,
        None => return empty_int_array_from_env(env),
    };
    let pairs = timer_slot_displacement_target_pairs(
        &display_slot_order,
        &preview_slot_order,
        dragged_slot_id,
    );
    int_array_from_slice(env, &pairs)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeTimerTileAutoScrollStep(
    _env: JNIEnv,
    _class: JClass,
    overscroll: jfloat,
    edge_size_px: jfloat,
) -> jfloat {
    timer_tile_auto_scroll_step(overscroll, edge_size_px)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_notifications_NativeNotificationBridge_nativeActionRequestCode(
    env: JNIEnv,
    _class: JClass,
    seed: jint,
    slot_ids: JIntArray,
) -> jint {
    let slot_id_count = match env.get_array_length(&slot_ids) {
        Ok(value) => value,
        Err(_) => return seed.wrapping_abs(),
    };
    let mut values = vec![0; slot_id_count as usize];
    if slot_id_count > 0 && env.get_int_array_region(&slot_ids, 0, &mut values).is_err() {
        return seed.wrapping_abs();
    }
    compute_action_request_code(seed, &values)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_notifications_NativeNotificationBridge_nativeBuildXiaomiPayload(
    mut env: JNIEnv,
    _class: JClass,
    title: JString,
    text: JString,
    big_text: JString,
    sub_text: JString,
    primary_title: JString,
    primary_elapsed: JString,
    share_content: JString,
    running_count: jint,
    include_island: jboolean,
) -> jstring {
    let state = match xiaomi_payload_state_from_java(
        &mut env,
        title,
        text,
        big_text,
        sub_text,
        primary_title,
        primary_elapsed,
        share_content,
        running_count,
    ) {
        Some(value) => value,
        None => return std::ptr::null_mut(),
    };
    let payload = build_xiaomi_payload(&state, include_island != JNI_FALSE);
    new_java_string(&mut env, &payload)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_notifications_NativeNotificationBridge_nativeShouldSyncTimerLiveUpdate(
    _env: JNIEnv,
    _class: JClass,
    notifications_granted: jboolean,
    has_running_slots: jboolean,
) -> jboolean {
    boolean_to_jni(should_sync_timer_live_update(
        notifications_granted != JNI_FALSE,
        has_running_slots != JNI_FALSE,
    ))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_notifications_NativeNotificationBridge_nativeShouldRunTimerLiveUpdateRefresh(
    _env: JNIEnv,
    _class: JClass,
    notification_runtime_permission_granted: jboolean,
    has_running_slots: jboolean,
) -> jboolean {
    boolean_to_jni(should_run_timer_live_update_refresh(
        notification_runtime_permission_granted != JNI_FALSE,
        has_running_slots != JNI_FALSE,
    ))
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_notifications_NativeNotificationBridge_nativeShouldRequestTimerNotificationPermission(
    _env: JNIEnv,
    _class: JClass,
    notification_runtime_permission_granted: jboolean,
    has_running_slots: jboolean,
    app_in_foreground: jboolean,
    runtime_permission_required: jboolean,
) -> jboolean {
    boolean_to_jni(should_request_timer_notification_permission(
        notification_runtime_permission_granted != JNI_FALSE,
        has_running_slots != JNI_FALSE,
        app_in_foreground != JNI_FALSE,
        runtime_permission_required != JNI_FALSE,
    ))
}

fn xiaomi_payload_state_from_java(
    env: &mut JNIEnv,
    title: JString,
    text: JString,
    big_text: JString,
    sub_text: JString,
    primary_title: JString,
    primary_elapsed: JString,
    share_content: JString,
    running_count: jint,
) -> Option<XiaomiPayloadState> {
    Some(XiaomiPayloadState {
        title: string_from_java(env, &title)?,
        text: string_from_java(env, &text)?,
        big_text: string_from_java(env, &big_text)?,
        sub_text: string_from_java(env, &sub_text)?,
        primary_title: string_from_java(env, &primary_title)?,
        primary_elapsed: string_from_java(env, &primary_elapsed)?,
        share_content: string_from_java(env, &share_content)?,
        running_count,
    })
}

fn string_from_java(env: &mut JNIEnv, value: &JString) -> Option<String> {
    env.get_string(value).ok().map(String::from)
}

fn format_notification_duration(duration_millis: i64) -> String {
    let total_seconds = (duration_millis / 1_000).max(0);
    let hours = total_seconds / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;

    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

fn format_long_duration(duration_millis: i64) -> String {
    let total_seconds = (duration_millis / 1_000).max(0);
    let hours = total_seconds / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;

    if hours >= 100 {
        format!("{hours}h {minutes:02}m")
    } else {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    }
}

fn format_compact_timer_duration(duration_millis: i64) -> String {
    let total_seconds = (duration_millis / 1_000).max(0);
    let hours = total_seconds / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;

    if hours >= 100 {
        format!("{}天", hours / 24)
    } else if hours > 0 {
        format!("{hours:02}:{minutes:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

fn format_compact_duration_label(duration_millis: i64) -> String {
    let total_minutes = duration_millis / 60_000;
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;

    if hours > 0 {
        if minutes > 0 {
            format!("{hours}小时 {minutes}分")
        } else {
            format!("{hours}小时")
        }
    } else {
        format!("{minutes}分")
    }
}

fn timer_tile_today_stats_label(today_duration: i64, today_total_duration: i64) -> String {
    let ratio = if today_total_duration > 0 {
        (today_duration as f32 / today_total_duration as f32).clamp(0.0, 1.0)
    } else {
        0.0
    };
    format!(
        "今日 {} · 占 {}",
        format_compact_duration_label(today_duration),
        format_ratio_percent(ratio)
    )
}

fn format_countdown_duration(duration_millis: i64) -> String {
    let total_seconds = (duration_millis / 1_000).max(0);
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes:02}:{seconds:02}")
}

fn slot_label(slot_id: i32) -> String {
    let value = slot_id.to_string();
    if value.len() >= 2 {
        value
    } else {
        format!("0{value}")
    }
}

fn timer_slot_display_title(title: &str, slot_id: i32) -> String {
    let title = title.trim();
    if title.is_empty() {
        format!("\u{4efb}\u{52a1} {}", slot_label(slot_id))
    } else {
        title.to_string()
    }
}

fn archived_task_display_title(title: &str, original_slot_id: i32) -> String {
    let title = title.trim();
    if title.is_empty() {
        format!("\u{4efb}\u{52a1} {}", slot_label(original_slot_id))
    } else {
        title.to_string()
    }
}

fn latest_finish_badge_label(moment_label: &str) -> String {
    format!("\u{6700}\u{8fd1}\u{7ed3}\u{675f} {}", moment_label)
}

fn longest_session_badge_label(duration_millis: i64) -> String {
    format!(
        "\u{6700}\u{957f}\u{5355}\u{6b21} {}",
        format_compact_duration_label(duration_millis)
    )
}

fn busiest_slot_badge_label(slot_id: i32, duration_millis: i64) -> String {
    format!(
        "\u{4eca}\u{65e5}\u{6700}\u{9ad8} \u{683c}\u{5b50} {} {}",
        slot_label(slot_id),
        format_compact_duration_label(duration_millis)
    )
}

fn archived_task_count_badge_label(count: i32) -> String {
    if count == 1 {
        "1 \u{4e2a}\u{5f52}\u{6863}\u{4efb}\u{52a1}".to_string()
    } else {
        format!("{count} \u{4e2a}\u{5f52}\u{6863}\u{4efb}\u{52a1}")
    }
}

fn history_week_summary_note(archived_task_count: i32) -> String {
    if archived_task_count == 0 {
        "\u{8fd9}\u{662f}\u{6700}\u{8fd1}\u{4e03}\u{5929}\u{7684}\u{603b}\u{4e13}\u{6ce8}\u{65f6}\u{957f}\u{3002}".to_string()
    } else {
        format!(
            "\u{4e0b}\u{65b9}\u{6709} {archived_task_count} \u{4e2a}\u{53ef}\u{6062}\u{590d}\u{7684}\u{5f52}\u{6863}\u{4efb}\u{52a1}\u{3002}"
        )
    }
}

fn build_history_search_haystack(fields: &[String]) -> String {
    let mut haystack = String::new();
    for field in fields {
        let normalized = field.trim().to_lowercase();
        if !normalized.is_empty() {
            haystack.push_str(&normalized);
            haystack.push('\n');
        }
    }
    haystack
}

fn elapsed_badge_label(duration_millis: i64) -> String {
    format!(
        "\u{5df2}\u{7d2f}\u{8ba1} {}",
        format_compact_duration_label(duration_millis)
    )
}

fn today_log_summary_badge_label(duration_millis: i64, log_count: i32) -> String {
    let log_label = if log_count == 1 {
        "1 \u{6b21}\u{8bb0}\u{5f55}".to_string()
    } else {
        format!("{log_count} \u{6b21}\u{8bb0}\u{5f55}")
    };
    format!(
        "\u{4eca}\u{65e5} {}\u{ff0c}\u{5171} {}",
        format_compact_duration_label(duration_millis),
        log_label
    )
}

fn restore_into_slot_label(slot_id: i32) -> String {
    format!(
        "\u{6062}\u{590d}\u{5230}\u{683c}\u{5b50} {}",
        slot_label(slot_id)
    )
}

fn slot_reference_text(slot_id: i32) -> String {
    format!("\u{683c}\u{5b50} {}", slot_label(slot_id))
}

fn is_timer_slot_blank_slate(
    title: &str,
    has_category: bool,
    note: &str,
    accumulated_millis: i64,
    running_since_epoch_millis: i64,
    phase_code: i32,
    cycle_index: i32,
    phase_progress_millis: i64,
) -> bool {
    title.trim().is_empty()
        && !has_category
        && note.trim().is_empty()
        && accumulated_millis == 0
        && running_since_epoch_millis < 0
        && phase_code == MICRO_BREAK_PHASE_FOCUS
        && cycle_index == 0
        && phase_progress_millis == 0
}

fn restore_target_slot_id(
    original_slot_id: i32,
    slot_ids: &[i32],
    blank_flags: &[jboolean],
) -> Option<i32> {
    if slot_ids.len() != blank_flags.len() {
        return None;
    }
    slot_ids
        .iter()
        .zip(blank_flags.iter())
        .find(|(slot_id, blank)| **slot_id == original_slot_id && **blank == JNI_TRUE)
        .map(|(slot_id, _)| *slot_id)
        .or_else(|| {
            slot_ids
                .iter()
                .zip(blank_flags.iter())
                .find(|(_, blank)| **blank == JNI_TRUE)
                .map(|(slot_id, _)| *slot_id)
        })
}

fn has_finance_row_draft_content(name: &str, amount: i64, note: Option<&str>) -> bool {
    name.trim().len() > 0
        || amount > 0
        || note.map(str::trim).is_some_and(|value| !value.is_empty())
}

fn normalize_repository_text(
    value: &str,
    max_length: usize,
    trim_start_only: bool,
    compact_whitespace: bool,
) -> String {
    let trimmed = if trim_start_only {
        value.trim_start()
    } else {
        value.trim()
    };
    let normalized = if compact_whitespace {
        compact_repository_whitespace(trimmed)
    } else {
        trimmed.to_string()
    };
    normalized.chars().take(max_length).collect()
}

fn compact_repository_whitespace(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut previous_was_whitespace = false;
    for ch in value.chars() {
        if ch.is_whitespace() {
            if !previous_was_whitespace {
                result.push(' ');
                previous_was_whitespace = true;
            }
        } else {
            result.push(ch);
            previous_was_whitespace = false;
        }
    }
    result.trim().to_string()
}

fn missing_existing_attachment_indices(
    existing_ids: &[String],
    incoming_ids: &[String],
) -> Vec<i32> {
    let incoming_lookup = incoming_ids.iter().collect::<HashSet<_>>();
    existing_ids
        .iter()
        .enumerate()
        .filter_map(|(index, id)| {
            if incoming_lookup.contains(id) {
                None
            } else {
                Some(index as i32)
            }
        })
        .collect()
}

fn should_capture_note_revision(
    structure_changed: bool,
    metadata_changed: bool,
    last_captured_at_epoch_millis: Option<i64>,
    timestamp: i64,
) -> bool {
    if !structure_changed {
        return false;
    }
    let Some(last_captured_at) = last_captured_at_epoch_millis else {
        return true;
    };
    timestamp.saturating_sub(last_captured_at) >= 45_000 || metadata_changed
}

fn build_note_revision_plan(
    revision_ids: &[String],
    captured_at_epoch_millis: &[i64],
    matches_previous_flags: &[jboolean],
    previous_exists: bool,
    should_capture: bool,
    snapshot_id: &str,
    max_count: usize,
) -> Vec<i32> {
    let count = revision_ids.len();
    if captured_at_epoch_millis.len() != count
        || matches_previous_flags.len() != count
        || max_count == 0
    {
        return Vec::new();
    }

    let mut seen = HashSet::<&str>::new();
    let mut base_indices = Vec::<usize>::new();
    for (index, id) in revision_ids.iter().enumerate() {
        if seen.insert(id.as_str()) {
            base_indices.push(index);
        }
    }
    base_indices.sort_by(|left, right| {
        captured_at_epoch_millis[*right]
            .cmp(&captured_at_epoch_millis[*left])
            .then_with(|| left.cmp(right))
    });

    let mut plan = Vec::<i32>::with_capacity(max_count);
    let include_snapshot = previous_exists && should_capture;
    if include_snapshot {
        plan.push(-1);
    }
    for index in base_indices {
        if include_snapshot
            && (revision_ids[index] == snapshot_id || matches_previous_flags[index] == JNI_TRUE)
        {
            continue;
        }
        plan.push(index as i32);
        if plan.len() >= max_count {
            break;
        }
    }
    plan.truncate(max_count);
    plan
}

fn sort_note_indices(
    sort_mode_code: i32,
    pinned_flags: &[jboolean],
    created_at_epoch_millis: &[i64],
    updated_at_epoch_millis: &[i64],
    titles: &[String],
) -> Option<Vec<i32>> {
    let count = pinned_flags.len();
    if created_at_epoch_millis.len() != count
        || updated_at_epoch_millis.len() != count
        || titles.len() != count
    {
        return None;
    }

    let mut indices: Vec<usize> = (0..count).collect();
    indices.sort_by(|left, right| {
        let left_pinned = pinned_flags[*left] == JNI_TRUE;
        let right_pinned = pinned_flags[*right] == JNI_TRUE;
        right_pinned.cmp(&left_pinned).then_with(|| {
            compare_note_sort_fields(
                sort_mode_code,
                *left,
                *right,
                created_at_epoch_millis,
                updated_at_epoch_millis,
                titles,
            )
        })
    });
    Some(indices.into_iter().map(|index| index as i32).collect())
}

fn compare_note_sort_fields(
    sort_mode_code: i32,
    left: usize,
    right: usize,
    created_at_epoch_millis: &[i64],
    updated_at_epoch_millis: &[i64],
    titles: &[String],
) -> Ordering {
    match sort_mode_code {
        1 => created_at_epoch_millis[right]
            .cmp(&created_at_epoch_millis[left])
            .then_with(|| updated_at_epoch_millis[right].cmp(&updated_at_epoch_millis[left])),
        2 => created_at_epoch_millis[left]
            .cmp(&created_at_epoch_millis[right])
            .then_with(|| compare_case_insensitive(&titles[left], &titles[right])),
        3 => compare_case_insensitive(&titles[left], &titles[right])
            .then_with(|| updated_at_epoch_millis[left].cmp(&updated_at_epoch_millis[right])),
        _ => updated_at_epoch_millis[right]
            .cmp(&updated_at_epoch_millis[left])
            .then_with(|| created_at_epoch_millis[right].cmp(&created_at_epoch_millis[left])),
    }
}

fn sort_trashed_note_indices(
    deleted_at_epoch_millis: &[i64],
    updated_at_epoch_millis: &[i64],
) -> Option<Vec<i32>> {
    if deleted_at_epoch_millis.len() != updated_at_epoch_millis.len() {
        return None;
    }
    let mut indices: Vec<usize> = (0..deleted_at_epoch_millis.len()).collect();
    indices.sort_by(|left, right| {
        deleted_at_epoch_millis[*right]
            .cmp(&deleted_at_epoch_millis[*left])
            .then_with(|| updated_at_epoch_millis[*right].cmp(&updated_at_epoch_millis[*left]))
    });
    Some(indices.into_iter().map(|index| index as i32).collect())
}

fn compare_case_insensitive(left: &str, right: &str) -> Ordering {
    left.to_lowercase().cmp(&right.to_lowercase())
}

fn format_ratio_percent(value: f32) -> String {
    format!("{}%", (value * 100.0).round() as i32)
}

fn compute_micro_break_target_millis(slot_id: i32, cycle_index: i32) -> i64 {
    let mixed = mix_micro_break_seed(slot_id, cycle_index);
    let variant_index = (mixed % MICRO_BREAK_FOCUS_VARIANT_COUNT as u64) as i64;
    MICRO_BREAK_FOCUS_MIN_MILLIS + (variant_index * MICRO_BREAK_FOCUS_STEP_MILLIS)
}

fn mix_micro_break_seed(slot_id: i32, cycle_index: i32) -> u64 {
    let mut value = ((slot_id as i64 as u64) << 32) ^ (cycle_index as i64 as u64);
    value ^= value >> 33;
    value = value.wrapping_mul(0xff51afd7ed558ccd);
    value ^= value >> 33;
    value = value.wrapping_mul(0xc4ceb9fe1a85ec53);
    value ^= value >> 33;
    value & 0x7fff_ffff_ffff_ffff
}

fn int_array_values(env: &JNIEnv, values: &JIntArray) -> Option<Vec<jint>> {
    let value_count = env.get_array_length(values).ok()? as usize;
    let mut result = vec![0; value_count];
    if value_count > 0 && env.get_int_array_region(values, 0, &mut result).is_err() {
        return None;
    }
    Some(result)
}

fn long_array_values(env: &JNIEnv, values: &JLongArray) -> Option<Vec<jlong>> {
    let value_count = env.get_array_length(values).ok()? as usize;
    let mut result = vec![0; value_count];
    if value_count > 0 && env.get_long_array_region(values, 0, &mut result).is_err() {
        return None;
    }
    Some(result)
}

fn boolean_array_values(env: &JNIEnv, values: &JBooleanArray) -> Option<Vec<jboolean>> {
    let value_count = env.get_array_length(values).ok()? as usize;
    let mut result = vec![JNI_FALSE; value_count];
    if value_count > 0
        && env
            .get_boolean_array_region(values, 0, &mut result)
            .is_err()
    {
        return None;
    }
    Some(result)
}

fn float_array_values(env: &JNIEnv, values: &JFloatArray) -> Option<Vec<f32>> {
    let value_count = env.get_array_length(values).ok()? as usize;
    let mut result = vec![0.0_f32; value_count];
    if value_count > 0 && env.get_float_array_region(values, 0, &mut result).is_err() {
        return None;
    }
    Some(result)
}

fn aggregate_finance_ledgers(
    period_code: i32,
    reference_year: i32,
    reference_month: i32,
    reference_day: i32,
    ledger_day_codes: &[i32],
    ledger_note_flags: &[i32],
    income_day_codes: &[i32],
    income_kind_codes: &[i32],
    income_amounts: &[i64],
    expense_day_codes: &[i32],
    expense_bucket_codes: &[i32],
    expense_amounts: &[i64],
) -> Option<FinanceLedgerAggregateNative> {
    if ledger_day_codes.len() != ledger_note_flags.len()
        || income_day_codes.len() != income_kind_codes.len()
        || income_day_codes.len() != income_amounts.len()
        || expense_day_codes.len() != expense_bucket_codes.len()
        || expense_day_codes.len() != expense_amounts.len()
    {
        return None;
    }

    let selected_days: Vec<i32> = ledger_day_codes
        .iter()
        .copied()
        .filter(|day_code| {
            finance_day_matches_period(
                *day_code,
                period_code,
                reference_year,
                reference_month,
                reference_day,
            )
        })
        .collect();
    if selected_days.is_empty() {
        return Some(FinanceLedgerAggregateNative::empty());
    }

    let mut aggregate = FinanceLedgerAggregateNative::empty();
    let mut day_has_data: Vec<bool> = selected_days
        .iter()
        .map(|day_code| {
            ledger_day_codes
                .iter()
                .position(|candidate| candidate == day_code)
                .and_then(|index| ledger_note_flags.get(index))
                .copied()
                .unwrap_or(0)
                != 0
        })
        .collect();

    for (index, day_code) in income_day_codes.iter().enumerate() {
        let Some(day_index) = selected_days
            .iter()
            .position(|candidate| candidate == day_code)
        else {
            continue;
        };
        let amount = income_amounts[index].max(0);
        if amount > 0 {
            day_has_data[day_index] = true;
        }
        match income_kind_codes[index] {
            FINANCE_INCOME_KIND_ACTIVE => {
                aggregate.active_income_total = aggregate.active_income_total.saturating_add(amount)
            }
            FINANCE_INCOME_KIND_ASSET => {
                aggregate.asset_income_total = aggregate.asset_income_total.saturating_add(amount)
            }
            FINANCE_INCOME_KIND_OTHER => {
                aggregate.other_income_total = aggregate.other_income_total.saturating_add(amount)
            }
            _ => {}
        }
    }

    for (index, day_code) in expense_day_codes.iter().enumerate() {
        let Some(day_index) = selected_days
            .iter()
            .position(|candidate| candidate == day_code)
        else {
            continue;
        };
        let amount = expense_amounts[index].max(0);
        if amount > 0 {
            day_has_data[day_index] = true;
        }
        aggregate.add_expense(expense_bucket_codes[index], amount);
    }

    aggregate.days_with_entries = day_has_data.iter().filter(|has_data| **has_data).count() as i64;
    Some(aggregate)
}

fn summarize_finance_month_snapshot(
    asset_kind_codes: &[i32],
    asset_amounts: &[i64],
    liability_amounts: &[i64],
) -> Option<[i64; 4]> {
    if asset_kind_codes.len() != asset_amounts.len() {
        return None;
    }
    let mut asset_total = 0_i64;
    let mut cash_reserve_total = 0_i64;
    let mut productive_asset_total = 0_i64;
    for (index, kind_code) in asset_kind_codes.iter().enumerate() {
        let amount = asset_amounts[index].max(0);
        asset_total = asset_total.saturating_add(amount);
        match *kind_code {
            FINANCE_NAMED_AMOUNT_CASH_RESERVE => {
                cash_reserve_total = cash_reserve_total.saturating_add(amount)
            }
            FINANCE_NAMED_AMOUNT_PRODUCTIVE_ASSET => {
                productive_asset_total = productive_asset_total.saturating_add(amount)
            }
            _ => {}
        }
    }

    let liability_total = liability_amounts
        .iter()
        .fold(0_i64, |acc, amount| acc.saturating_add((*amount).max(0)));
    Some([
        asset_total,
        liability_total,
        cash_reserve_total,
        productive_asset_total,
    ])
}

fn aggregate_finance_day_ledger_values(
    income_kind_codes: &[i32],
    income_amounts: &[i64],
    expense_bucket_codes: &[i32],
    expense_amounts: &[i64],
    has_note: bool,
) -> Option<[i64; 10]> {
    if income_kind_codes.len() != income_amounts.len()
        || expense_bucket_codes.len() != expense_amounts.len()
    {
        return None;
    }

    let mut aggregate = FinanceLedgerAggregateNative::empty();
    let mut has_data = has_note;
    for (index, kind_code) in income_kind_codes.iter().enumerate() {
        let amount = income_amounts[index].max(0);
        if amount > 0 {
            has_data = true;
        }
        match *kind_code {
            FINANCE_INCOME_KIND_ACTIVE => {
                aggregate.active_income_total = aggregate.active_income_total.saturating_add(amount)
            }
            FINANCE_INCOME_KIND_ASSET => {
                aggregate.asset_income_total = aggregate.asset_income_total.saturating_add(amount)
            }
            FINANCE_INCOME_KIND_OTHER => {
                aggregate.other_income_total = aggregate.other_income_total.saturating_add(amount)
            }
            _ => {}
        }
    }

    for (index, bucket_code) in expense_bucket_codes.iter().enumerate() {
        let amount = expense_amounts[index].max(0);
        if amount > 0 {
            has_data = true;
        }
        aggregate.add_expense(*bucket_code, amount);
    }
    aggregate.days_with_entries = if has_data { 1 } else { 0 };
    Some(finance_ledger_aggregate_values(aggregate))
}

fn recent_finance_template_indices(
    day_codes: &[i32],
    kind_codes: &[i32],
    names: &[String],
    limit: usize,
) -> Vec<i32> {
    if limit == 0 || day_codes.len() != kind_codes.len() || day_codes.len() != names.len() {
        return Vec::new();
    }

    let mut indices: Vec<usize> = (0..names.len())
        .filter(|index| !names[*index].trim().is_empty())
        .collect();
    indices.sort_by(|left, right| {
        day_codes[*right]
            .cmp(&day_codes[*left])
            .then_with(|| left.cmp(right))
    });

    let mut seen = Vec::<(i32, String)>::new();
    let mut selected = Vec::<i32>::new();
    for index in indices {
        let key = (kind_codes[index], names[index].to_lowercase());
        if seen.iter().any(|existing| existing == &key) {
            continue;
        }
        seen.push(key);
        selected.push(index as i32);
        if selected.len() >= limit {
            break;
        }
    }
    selected
}

fn finance_missing_template_indices(
    row_kind_codes: &[i32],
    row_names: &[String],
    template_kind_codes: &[i32],
    template_names: &[String],
) -> Vec<i32> {
    if row_kind_codes.len() != row_names.len() || template_kind_codes.len() != template_names.len()
    {
        return Vec::new();
    }

    let mut present = HashSet::<(i32, String)>::new();
    for (kind, name) in row_kind_codes.iter().zip(row_names.iter()) {
        let normalized = normalized_finance_template_name(name);
        if !normalized.is_empty() {
            present.insert((*kind, normalized));
        }
    }

    let mut missing_indices = Vec::new();
    for (index, (kind, name)) in template_kind_codes
        .iter()
        .zip(template_names.iter())
        .enumerate()
    {
        let normalized = normalized_finance_template_name(name);
        if normalized.is_empty() {
            continue;
        }
        let key = (*kind, normalized);
        if present.insert(key) {
            missing_indices.push(index as i32);
        }
    }
    missing_indices
}

fn normalized_finance_template_name(value: &str) -> String {
    value.trim().to_lowercase()
}

fn finance_day_matches_period(
    day_code: i32,
    period_code: i32,
    reference_year: i32,
    reference_month: i32,
    reference_day: i32,
) -> bool {
    let year = day_code / 10_000;
    let month = (day_code / 100) % 100;
    let day = day_code % 100;
    if year <= 0 || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return false;
    }
    match period_code {
        0 => year == reference_year && month == reference_month && day == reference_day,
        1 => year == reference_year && month == reference_month,
        2 => {
            if !(1..=12).contains(&reference_month) {
                return false;
            }
            year == reference_year && ((month - 1) / 3) == ((reference_month - 1) / 3)
        }
        3 => year == reference_year,
        _ => false,
    }
}

fn finance_ledger_aggregate_values(aggregate: FinanceLedgerAggregateNative) -> [i64; 10] {
    [
        aggregate.active_income_total,
        aggregate.asset_income_total,
        aggregate.other_income_total,
        aggregate.debt_total,
        aggregate.food_total,
        aggregate.btc_total,
        aggregate.living_total,
        aggregate.learning_total,
        aggregate.other_expense_total,
        aggregate.days_with_entries,
    ]
}

fn finance_snapshot_values(
    values: finance_profile::FinanceSnapshotValues,
    include_recorded_days: bool,
) -> Vec<f64> {
    let mut fields = vec![
        values.total_income as f64,
        values.total_outflow as f64,
        values.net_cashflow as f64,
        values.freedom_gap as f64,
        values.passive_coverage_ratio as f64,
        values.wage_dependence_ratio as f64,
        values.liability_pressure_ratio as f64,
        values.net_worth as f64,
        values.defensive_coverage.map(f64::from).unwrap_or(f64::NAN),
        values.asset_yield_ratio as f64,
    ];
    if include_recorded_days {
        fields.push(values.recorded_days as f64);
    }
    fields
}

fn finance_narrative_code(
    has_entries: bool,
    passive_coverage_ratio: f32,
    total_outflow: i64,
    asset_yield_ratio: f32,
    liability_pressure_ratio: f32,
    freedom_gap: i64,
) -> i32 {
    if !has_entries {
        0
    } else if passive_coverage_ratio >= 1.0 && total_outflow > 0 {
        1
    } else if asset_yield_ratio <= 0.0 && passive_coverage_ratio <= 0.05 {
        2
    } else if liability_pressure_ratio > 0.35 {
        3
    } else if freedom_gap > 0 {
        4
    } else {
        5
    }
}

fn resolve_micro_break(
    now: i64,
    slot_id: i32,
    accumulated_millis: i64,
    running_since_epoch_millis: i64,
    phase_code: i32,
    cycle_index: i32,
    phase_progress_millis: i64,
    updated_at: i64,
) -> Option<NativeMicroBreakResolution> {
    let mut phase = match phase_code {
        MICRO_BREAK_PHASE_FOCUS => MICRO_BREAK_PHASE_FOCUS,
        MICRO_BREAK_PHASE_BREAK => MICRO_BREAK_PHASE_BREAK,
        _ => return None,
    };
    let mut cycle_index = cycle_index.max(0);
    let running_since_epoch_millis = running_since_epoch_millis.max(0);
    let safe_now = now.max(running_since_epoch_millis);
    let mut accumulated_millis = sanitize_tracked_duration(accumulated_millis);
    let phase_target = micro_break_phase_target_millis(slot_id, phase, cycle_index);
    let mut phase_progress =
        sanitize_tracked_duration(phase_progress_millis).clamp(0, phase_target);
    let mut active_segment_start = running_since_epoch_millis;
    let mut remaining_elapsed = (safe_now - running_since_epoch_millis).max(0);
    let mut latest_transition_at: Option<i64> = None;
    let mut sessions = Vec::<NativeMicroBreakSession>::new();
    let mut transitions = Vec::<NativeMicroBreakTransition>::new();

    loop {
        let phase_target = micro_break_phase_target_millis(slot_id, phase, cycle_index);
        phase_progress = phase_progress.clamp(0, phase_target);
        let remaining_in_phase = (phase_target - phase_progress).max(0);

        if remaining_in_phase == 0 {
            if phase == MICRO_BREAK_PHASE_FOCUS {
                phase = MICRO_BREAK_PHASE_BREAK;
                phase_progress = 0;
                transitions.push(NativeMicroBreakTransition {
                    transition_type: MICRO_BREAK_TRANSITION_BREAK_STARTED,
                    occurred_at_epoch_millis: active_segment_start,
                });
                latest_transition_at = Some(active_segment_start);
            } else {
                phase = MICRO_BREAK_PHASE_FOCUS;
                phase_progress = 0;
                cycle_index = cycle_index.saturating_add(1);
                transitions.push(NativeMicroBreakTransition {
                    transition_type: MICRO_BREAK_TRANSITION_FOCUS_RESUMED,
                    occurred_at_epoch_millis: active_segment_start,
                });
                latest_transition_at = Some(active_segment_start);
            }
            continue;
        }

        if remaining_elapsed < remaining_in_phase {
            break;
        }

        let transition_at = active_segment_start.saturating_add(remaining_in_phase);
        if phase == MICRO_BREAK_PHASE_FOCUS {
            accumulated_millis =
                sanitize_tracked_duration(accumulated_millis.saturating_add(remaining_in_phase));
            if remaining_in_phase > 0 {
                sessions.push(NativeMicroBreakSession {
                    started_at_epoch_millis: active_segment_start,
                    ended_at_epoch_millis: transition_at,
                    duration_millis: remaining_in_phase,
                });
            }
            phase = MICRO_BREAK_PHASE_BREAK;
            transitions.push(NativeMicroBreakTransition {
                transition_type: MICRO_BREAK_TRANSITION_BREAK_STARTED,
                occurred_at_epoch_millis: transition_at,
            });
        } else {
            phase = MICRO_BREAK_PHASE_FOCUS;
            cycle_index = cycle_index.saturating_add(1);
            transitions.push(NativeMicroBreakTransition {
                transition_type: MICRO_BREAK_TRANSITION_FOCUS_RESUMED,
                occurred_at_epoch_millis: transition_at,
            });
        }

        latest_transition_at = Some(transition_at);
        phase_progress = 0;
        active_segment_start = transition_at;
        remaining_elapsed = remaining_elapsed.saturating_sub(remaining_in_phase);
    }

    Some(NativeMicroBreakResolution {
        accumulated_millis,
        running_since_epoch_millis: active_segment_start,
        phase,
        cycle_index,
        phase_progress_millis: phase_progress,
        updated_at: latest_transition_at
            .map(|transition_at| transition_at.max(updated_at.max(0)))
            .unwrap_or_else(|| updated_at.max(0)),
        sessions,
        transitions,
    })
}

fn pause_micro_break(
    now: i64,
    slot_id: i32,
    accumulated_millis: i64,
    running_since_epoch_millis: i64,
    phase_code: i32,
    cycle_index: i32,
    phase_progress_millis: i64,
    updated_at: i64,
) -> Option<[i64; 10]> {
    let phase = match phase_code {
        MICRO_BREAK_PHASE_FOCUS => MICRO_BREAK_PHASE_FOCUS,
        MICRO_BREAK_PHASE_BREAK => MICRO_BREAK_PHASE_BREAK,
        _ => return None,
    };
    let cycle_index = cycle_index.max(0);
    let phase_target = micro_break_phase_target_millis(slot_id, phase, cycle_index);
    let phase_progress = sanitize_tracked_duration(phase_progress_millis).clamp(0, phase_target);
    let normalized_accumulated = sanitize_tracked_duration(accumulated_millis);
    let _normalized_updated_at = updated_at.clamp(0, now.max(0));
    let elapsed = safe_elapsed_since(running_since_epoch_millis, now);

    if phase == MICRO_BREAK_PHASE_FOCUS {
        let duration = elapsed.max(0);
        let next_accumulated =
            sanitize_tracked_duration(normalized_accumulated.saturating_add(duration));
        let next_progress = phase_progress
            .saturating_add(duration)
            .clamp(0, phase_target);
        let has_session = if duration > 0 { 1 } else { 0 };
        Some([
            next_accumulated,
            -1,
            MICRO_BREAK_PHASE_FOCUS as i64,
            cycle_index as i64,
            next_progress,
            now,
            has_session,
            if has_session == 1 {
                running_since_epoch_millis
            } else {
                0
            },
            if has_session == 1 { now } else { 0 },
            if has_session == 1 { duration } else { 0 },
        ])
    } else {
        let next_progress = phase_progress
            .saturating_add(elapsed)
            .clamp(0, MICRO_BREAK_REST_MILLIS);
        Some([
            normalized_accumulated,
            -1,
            MICRO_BREAK_PHASE_BREAK as i64,
            cycle_index as i64,
            next_progress,
            now,
            0,
            0,
            0,
            0,
        ])
    }
}

fn safe_elapsed_since(started_at_epoch_millis: i64, now: i64) -> i64 {
    if started_at_epoch_millis < 0 || started_at_epoch_millis > now {
        0
    } else {
        now - started_at_epoch_millis
    }
}

fn finance_structure_headline(
    income_total: i64,
    expense_total: i64,
    net_cashflow: i64,
) -> &'static str {
    if income_total <= 0 || expense_total <= 0 {
        "等待底稿"
    } else if net_cashflow > 0 {
        "现金流为正"
    } else {
        "继续收口"
    }
}

fn finance_income_kind_label(kind_code: i32) -> &'static str {
    match kind_code {
        FINANCE_INCOME_KIND_ACTIVE => "劳动型",
        FINANCE_INCOME_KIND_ASSET => "资产型",
        _ => "其他",
    }
}

fn finance_named_amount_kind_label(kind_code: i32) -> &'static str {
    match kind_code {
        0 => "现金储备",
        1 => "产出型",
        2 => "其他资产",
        3 => "负债余额",
        _ => "其他负债",
    }
}

fn finance_default_template_kind_codes(template_kind_code: i32) -> &'static [i32] {
    match template_kind_code {
        0 => &[
            FINANCE_BUCKET_DEBT,
            FINANCE_BUCKET_FOOD,
            FINANCE_BUCKET_LIVING,
            FINANCE_BUCKET_BTC,
            FINANCE_BUCKET_LEARNING,
        ],
        1 => &[
            FINANCE_INCOME_KIND_ACTIVE,
            FINANCE_INCOME_KIND_ACTIVE,
            FINANCE_INCOME_KIND_ASSET,
            FINANCE_INCOME_KIND_OTHER,
        ],
        2 => &[
            FINANCE_NAMED_AMOUNT_CASH_RESERVE,
            FINANCE_NAMED_AMOUNT_PRODUCTIVE_ASSET,
            FINANCE_NAMED_AMOUNT_PRODUCTIVE_ASSET,
            FINANCE_NAMED_AMOUNT_OTHER_ASSET,
        ],
        3 => &[
            FINANCE_NAMED_AMOUNT_LIABILITY_BALANCE,
            FINANCE_NAMED_AMOUNT_LIABILITY_BALANCE,
            FINANCE_NAMED_AMOUNT_LIABILITY_BALANCE,
            FINANCE_NAMED_AMOUNT_OTHER_LIABILITY,
        ],
        _ => &[],
    }
}

fn finance_asset_kind_codes() -> &'static [i32] {
    &[
        FINANCE_NAMED_AMOUNT_CASH_RESERVE,
        FINANCE_NAMED_AMOUNT_PRODUCTIVE_ASSET,
        FINANCE_NAMED_AMOUNT_OTHER_ASSET,
    ]
}

fn finance_liability_kind_codes() -> &'static [i32] {
    &[
        FINANCE_NAMED_AMOUNT_LIABILITY_BALANCE,
        FINANCE_NAMED_AMOUNT_OTHER_LIABILITY,
    ]
}

fn sanitize_finance_draft(value: &str, max_length: usize) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_digit() || *ch == '-')
        .take(max_length)
        .collect()
}

fn parse_finance_amount_draft(value: &str, max_digits: usize, max_amount: i64) -> i64 {
    let digits: String = value
        .chars()
        .filter(char::is_ascii_digit)
        .take(max_digits)
        .collect();
    digits
        .parse::<i64>()
        .ok()
        .map(|amount| amount.min(max_amount.max(0)).max(0))
        .unwrap_or(0)
}

fn parse_finance_target_share(value: &str) -> Option<f32> {
    let digits: String = value.chars().filter(char::is_ascii_digit).take(3).collect();
    if digits.is_empty() {
        return None;
    }
    let percent = digits.parse::<i32>().ok()?.clamp(0, 100);
    Some(percent as f32 / 100.0)
}

fn finance_target_percent_text(value: f32) -> String {
    ((value * 100.0).round() as i32).to_string()
}

fn finance_scale_long(period_code: i32, amount: i64, mode_code: i32) -> i64 {
    let factor = finance_period_scale(period_code);
    match mode_code {
        0 => finance_scale_unsigned_amount(amount, factor),
        1 => finance_scale_signed_amount(amount, factor),
        2 => monthly_amount_from_display(amount, factor),
        _ => amount,
    }
}

fn finance_scale_float(period_code: i32, value: f32, mode_code: i32) -> f32 {
    if value.is_nan() {
        return f32::NAN;
    }
    let factor = finance_period_scale(period_code) as f32;
    match mode_code {
        0 if factor != 0.0 => value / factor,
        1 => value * factor,
        _ => value,
    }
}

fn finance_scale_unsigned_amount(amount: i64, factor: f64) -> i64 {
    ((amount as f64) * factor).round().max(0.0) as i64
}

fn finance_scale_signed_amount(amount: i64, factor: f64) -> i64 {
    ((amount as f64) * factor).round() as i64
}

fn monthly_amount_from_display(display_amount: i64, factor: f64) -> i64 {
    if display_amount <= 0 || factor <= 0.0 {
        return 0;
    }
    (((display_amount as f64) / factor).round() as i64).clamp(0, 999_999_999)
}

fn finance_bucket_share(
    bucket_code: i32,
    share_kind_code: i32,
    income_total: i64,
    expense_total: i64,
    debt_total: i64,
    food_total: i64,
    btc_total: i64,
    living_total: i64,
    learning_total: i64,
    other_total: i64,
) -> f32 {
    let bucket_amount = match bucket_code {
        FINANCE_BUCKET_DEBT => debt_total,
        FINANCE_BUCKET_FOOD => food_total,
        FINANCE_BUCKET_BTC => btc_total,
        FINANCE_BUCKET_LIVING => living_total,
        FINANCE_BUCKET_LEARNING => learning_total,
        FINANCE_BUCKET_OTHER => other_total,
        _ => 0,
    }
    .max(0);
    let denominator = if share_kind_code == 0 {
        expense_total
    } else {
        income_total
    };
    if bucket_amount <= 0 || denominator <= 0 {
        0.0
    } else {
        bucket_amount as f32 / denominator as f32
    }
}

fn finance_settings_target_summary(
    bucket_codes: &[i32],
    labels: &[String],
    target_shares: &[f32],
) -> String {
    bucket_codes
        .iter()
        .zip(labels.iter())
        .zip(target_shares.iter())
        .filter_map(|((bucket_code, label), target_share)| {
            if !is_primary_finance_bucket(*bucket_code) || target_share.is_nan() {
                return None;
            }
            let safe_label = label.trim();
            if safe_label.is_empty() {
                return None;
            }
            Some(format!(
                "{safe_label} {}",
                format_rounded_percent(*target_share)
            ))
        })
        .collect::<Vec<_>>()
        .join(" / ")
}

fn is_primary_finance_bucket(bucket_code: i32) -> bool {
    matches!(
        bucket_code,
        FINANCE_BUCKET_DEBT
            | FINANCE_BUCKET_FOOD
            | FINANCE_BUCKET_BTC
            | FINANCE_BUCKET_LIVING
            | FINANCE_BUCKET_LEARNING
    )
}

fn format_finance_currency(amount: i64) -> String {
    format!("¥{}", format_grouped_i64(amount.max(0)))
}

fn format_finance_signed_currency(amount: i64) -> String {
    let base = format!("¥{}", format_grouped_i64(amount.saturating_abs()));
    if amount > 0 {
        format!("+{base}")
    } else if amount < 0 {
        format!("-{base}")
    } else {
        base
    }
}

fn format_grouped_i64(value: i64) -> String {
    let digits = value.to_string();
    let mut result = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            result.push(',');
        }
        result.push(ch);
    }
    result.chars().rev().collect()
}

fn format_finance_percent(value: f32) -> String {
    format!("{:.1}%", value * 100.0)
}

fn finance_bucket_ratio_line(income_share: f32, expense_share: f32, target_share: f32) -> String {
    if target_share.is_nan() {
        format!("占支出 {}", format_finance_percent(expense_share))
    } else {
        format!(
            "占收入 {} · 目标 {}",
            format_finance_percent(income_share),
            format_finance_percent(target_share)
        )
    }
}

fn finance_bucket_drift_line(income_share: f32, target_share: f32) -> String {
    let drift = income_share - target_share;
    let distance = format_finance_percent(drift.abs());
    if drift <= 0.0 {
        format!("低于目标 {distance}")
    } else {
        format!("高于目标 {distance}")
    }
}

fn is_finance_bucket_drift_healthy(bucket_code: i32, income_share: f32, target_share: f32) -> bool {
    match bucket_code {
        FINANCE_BUCKET_DEBT | FINANCE_BUCKET_FOOD | FINANCE_BUCKET_LIVING => {
            income_share <= target_share
        }
        FINANCE_BUCKET_BTC | FINANCE_BUCKET_LEARNING => income_share >= target_share,
        FINANCE_BUCKET_OTHER => (income_share - target_share).abs() <= target_share * 0.15,
        _ => false,
    }
}

fn matches_note_quick_filter(
    filter_code: i32,
    pinned: bool,
    todo_remaining: i32,
    image_attachment_count: i32,
    has_folder: bool,
) -> bool {
    match filter_code {
        0 => true,
        1 => pinned,
        2 => todo_remaining > 0,
        3 => image_attachment_count > 0,
        4 => !has_folder,
        _ => false,
    }
}

fn format_currency_amount(amount: i64, prefix: &str) -> String {
    format!("{}{}", prefix, format_grouped_i64(amount.max(0)))
}

fn format_signed_currency_amount(amount: i64, prefix: &str) -> String {
    let base = format!("{}{}", prefix, format_grouped_i64(amount.saturating_abs()));
    if amount > 0 {
        format!("+{base}")
    } else if amount < 0 {
        format!("-{base}")
    } else {
        base
    }
}

fn format_rounded_percent(value: f32) -> String {
    format!("{}%", (value * 100.0).round() as i32)
}

fn format_signed_percent(value: f32) -> String {
    let base = format_finance_percent(value.abs());
    if value > 0.0 {
        format!("+{base}")
    } else if value < 0.0 {
        format!("-{base}")
    } else {
        base
    }
}

fn format_defensive_coverage(coverage: f32, unit_label: &str) -> String {
    if coverage.is_nan() {
        "已覆盖".to_string()
    } else if coverage <= 0.0 {
        format!("0.0 {unit_label}")
    } else if coverage >= 99.0 {
        format!("99+ {unit_label}")
    } else {
        format!("{coverage:.1} {unit_label}")
    }
}

fn format_asset_liability_ratio(asset_total: i64, liability_total: i64) -> String {
    if liability_total > 0 {
        format!("{:.2}", asset_total as f64 / liability_total as f64)
    } else if asset_total > 0 {
        "\u{221e}".to_string()
    } else {
        "0.00".to_string()
    }
}

fn finance_coverage_caption(period_code: i32) -> &'static str {
    match period_code {
        0 => "现金储备还能顶多少天",
        1 => "现金储备还能顶多少个月",
        2 => "现金储备还能顶多少个季度",
        3 => "现金储备还能顶多少年",
        _ => "现金储备还能顶多久",
    }
}

fn micro_break_resolution_values(resolution: &NativeMicroBreakResolution) -> Vec<i64> {
    let mut values =
        Vec::with_capacity(8 + resolution.sessions.len() * 3 + resolution.transitions.len() * 2);
    values.push(resolution.accumulated_millis);
    values.push(resolution.running_since_epoch_millis);
    values.push(resolution.phase as i64);
    values.push(resolution.cycle_index as i64);
    values.push(resolution.phase_progress_millis);
    values.push(resolution.updated_at);
    values.push(resolution.sessions.len() as i64);
    values.push(resolution.transitions.len() as i64);
    for session in &resolution.sessions {
        values.push(session.started_at_epoch_millis);
        values.push(session.ended_at_epoch_millis);
        values.push(session.duration_millis);
    }
    for transition in &resolution.transitions {
        values.push(transition.transition_type);
        values.push(transition.occurred_at_epoch_millis);
    }
    values
}

fn micro_break_phase_target_millis(slot_id: i32, phase: i32, cycle_index: i32) -> i64 {
    if phase == MICRO_BREAK_PHASE_FOCUS {
        compute_micro_break_target_millis(slot_id, cycle_index)
            .clamp(MICRO_BREAK_FOCUS_MIN_MILLIS, MICRO_BREAK_FOCUS_MAX_MILLIS)
    } else {
        MICRO_BREAK_REST_MILLIS
    }
}

fn sanitize_tracked_duration(value: i64) -> i64 {
    value.clamp(0, MAX_TRACKED_DURATION_MILLIS)
}

fn build_timer_session_ui_index_stats(
    session_slot_ids: &[i32],
    session_ended_at_epoch_millis: &[i64],
    session_duration_millis: &[i64],
    archived_original_slot_ids: &[i32],
    slot_ids: &[i32],
) -> Option<Vec<i64>> {
    if session_slot_ids.len() != session_ended_at_epoch_millis.len()
        || session_slot_ids.len() != session_duration_millis.len()
    {
        return None;
    }

    let mut latest_session_index = -1_i64;
    let mut latest_session_ended_at = i64::MIN;
    let mut peak_session_index = -1_i64;
    let mut peak_session_duration = i64::MIN;
    let mut session_count_by_slot = vec![0_i64; slot_ids.len()];
    let mut latest_session_index_by_slot = vec![-1_i64; slot_ids.len()];
    let mut latest_session_ended_at_by_slot = vec![i64::MIN; slot_ids.len()];
    let mut archived_count_by_slot = vec![0_i64; slot_ids.len()];

    for (index, slot_id) in session_slot_ids.iter().enumerate() {
        let ended_at = session_ended_at_epoch_millis[index];
        if ended_at > latest_session_ended_at {
            latest_session_ended_at = ended_at;
            latest_session_index = index as i64;
        }

        let duration = session_duration_millis[index];
        if duration > peak_session_duration {
            peak_session_duration = duration;
            peak_session_index = index as i64;
        }

        if let Some(slot_index) = slot_ids.iter().position(|candidate| candidate == slot_id) {
            session_count_by_slot[slot_index] = session_count_by_slot[slot_index].saturating_add(1);
            if ended_at > latest_session_ended_at_by_slot[slot_index] {
                latest_session_ended_at_by_slot[slot_index] = ended_at;
                latest_session_index_by_slot[slot_index] = index as i64;
            }
        }
    }

    for slot_id in archived_original_slot_ids {
        if let Some(slot_index) = slot_ids.iter().position(|candidate| candidate == slot_id) {
            archived_count_by_slot[slot_index] =
                archived_count_by_slot[slot_index].saturating_add(1);
        }
    }

    let mut stats = Vec::with_capacity(2 + slot_ids.len() * 4);
    stats.push(latest_session_index);
    stats.push(peak_session_index);
    for (slot_index, slot_id) in slot_ids.iter().enumerate() {
        stats.push(*slot_id as i64);
        stats.push(session_count_by_slot[slot_index]);
        stats.push(latest_session_index_by_slot[slot_index]);
        stats.push(archived_count_by_slot[slot_index]);
    }
    Some(stats)
}

fn resolve_timer_slot_insert_index(
    candidates: &[TimerSlotInsertCandidate],
    current_insert_index: i32,
    dragged_center: NativePoint,
    auto_scrolling: bool,
    now_uptime_ms: i64,
    last_switch_uptime_ms: i64,
    minimum_hysteresis_px: f32,
) -> i32 {
    if candidates.is_empty() {
        return current_insert_index;
    }

    if candidates
        .iter()
        .find(|candidate| candidate.slot_index == current_insert_index)
        .map(|candidate| timer_slot_keep_bounds(*candidate, minimum_hysteresis_px, auto_scrolling))
        .is_some_and(|bounds| bounds.contains(dragged_center))
    {
        return current_insert_index;
    }

    let next_candidate = candidates
        .iter()
        .filter(|candidate| {
            timer_slot_activation_bounds(**candidate, minimum_hysteresis_px, auto_scrolling)
                .contains(dragged_center)
        })
        .min_by(|left, right| {
            let left_distance = left.bounds.center().distance_squared_to(dragged_center);
            let right_distance = right.bounds.center().distance_squared_to(dragged_center);
            compare_f32(left_distance, right_distance)
                .then_with(|| left.slot_index.cmp(&right.slot_index))
        });

    if auto_scrolling && now_uptime_ms - last_switch_uptime_ms < 60 {
        return current_insert_index;
    }

    next_candidate
        .map(|candidate| candidate.slot_index)
        .unwrap_or(current_insert_index)
}

fn stabilize_timer_slot_insert_index(
    current_insert_index: i32,
    raw_insert_index: i32,
    pending_insert_index: Option<i32>,
    pending_insert_index_since_uptime_ms: i64,
    now_uptime_ms: i64,
    hold_millis: i64,
) -> TimerSlotInsertIndexStabilizationResult {
    if raw_insert_index == current_insert_index {
        return TimerSlotInsertIndexStabilizationResult {
            insert_index: current_insert_index,
            pending_insert_index: None,
            pending_insert_index_since_uptime_ms: 0,
        };
    }

    if pending_insert_index != Some(raw_insert_index) {
        return TimerSlotInsertIndexStabilizationResult {
            insert_index: current_insert_index,
            pending_insert_index: Some(raw_insert_index),
            pending_insert_index_since_uptime_ms: now_uptime_ms,
        };
    }

    if now_uptime_ms - pending_insert_index_since_uptime_ms < hold_millis {
        return TimerSlotInsertIndexStabilizationResult {
            insert_index: current_insert_index,
            pending_insert_index,
            pending_insert_index_since_uptime_ms,
        };
    }

    TimerSlotInsertIndexStabilizationResult {
        insert_index: raw_insert_index,
        pending_insert_index: None,
        pending_insert_index_since_uptime_ms: 0,
    }
}

fn timer_slot_keep_bounds(
    candidate: TimerSlotInsertCandidate,
    minimum_hysteresis_px: f32,
    auto_scrolling: bool,
) -> NativeRect {
    candidate.bounds.inflated_by(timer_slot_hysteresis_px(
        candidate,
        minimum_hysteresis_px,
        auto_scrolling,
    ))
}

fn timer_slot_activation_bounds(
    candidate: TimerSlotInsertCandidate,
    minimum_hysteresis_px: f32,
    auto_scrolling: bool,
) -> NativeRect {
    candidate.bounds.deflated_by(timer_slot_hysteresis_px(
        candidate,
        minimum_hysteresis_px,
        auto_scrolling,
    ))
}

fn timer_slot_hysteresis_px(
    candidate: TimerSlotInsertCandidate,
    minimum_hysteresis_px: f32,
    auto_scrolling: bool,
) -> f32 {
    let base_hysteresis =
        minimum_hysteresis_px.max(candidate.bounds.width().min(candidate.bounds.height()) * 0.12);
    if auto_scrolling {
        base_hysteresis * 1.5
    } else {
        base_hysteresis
    }
}

fn reorder_timer_slot_ids(
    base_slot_order: &[i32],
    dragged_slot_id: i32,
    insert_index: i32,
) -> Option<Vec<i32>> {
    let insert_index = usize::try_from(insert_index).ok()?;
    if insert_index >= base_slot_order.len() {
        return None;
    }
    let from_index = base_slot_order
        .iter()
        .position(|slot_id| *slot_id == dragged_slot_id)?;
    if from_index == insert_index {
        return Some(base_slot_order.to_vec());
    }

    let mut reordered = base_slot_order.to_vec();
    let moved_slot_id = reordered.remove(from_index);
    let target_index = insert_index.min(reordered.len());
    reordered.insert(target_index, moved_slot_id);
    Some(reordered)
}

fn timer_slot_displacement_target_pairs(
    display_slot_order: &[i32],
    preview_slot_order: &[i32],
    dragged_slot_id: i32,
) -> Vec<i32> {
    if display_slot_order.len() != preview_slot_order.len()
        || display_slot_order == preview_slot_order
    {
        return Vec::new();
    }

    let mut pairs = Vec::new();
    for (target_index, slot_id) in preview_slot_order.iter().enumerate() {
        if *slot_id == dragged_slot_id {
            continue;
        }
        let Some(current_index) = display_slot_order.iter().position(|value| value == slot_id)
        else {
            continue;
        };
        let Some(target_slot_id) = display_slot_order.get(target_index) else {
            continue;
        };
        if current_index != target_index {
            pairs.push(*slot_id);
            pairs.push(*target_slot_id);
        }
    }
    pairs
}

fn timer_tile_auto_scroll_step(overscroll: f32, edge_size_px: f32) -> f32 {
    if overscroll == 0.0 || edge_size_px <= 0.0 {
        return 0.0;
    }

    let direction = if overscroll < 0.0 { -1.0 } else { 1.0 };
    let normalized_strength = (overscroll.abs() / edge_size_px).clamp(0.0, 1.0);
    if normalized_strength < 0.12 {
        return 0.0;
    }
    let eased_strength = ((normalized_strength - 0.12) / 0.88).clamp(0.0, 1.0);
    direction * (4.0 + 24.0 * eased_strength * eased_strength)
}

fn should_sync_timer_live_update(notifications_granted: bool, has_running_slots: bool) -> bool {
    notifications_granted && has_running_slots
}

fn should_run_timer_live_update_refresh(
    notification_runtime_permission_granted: bool,
    has_running_slots: bool,
) -> bool {
    notification_runtime_permission_granted && has_running_slots
}

fn should_request_timer_notification_permission(
    notification_runtime_permission_granted: bool,
    has_running_slots: bool,
    app_in_foreground: bool,
    runtime_permission_required: bool,
) -> bool {
    runtime_permission_required
        && has_running_slots
        && app_in_foreground
        && !notification_runtime_permission_granted
}

fn resolve_timer_slot_drop_target_id(
    candidates: &[TimerSlotDropTargetCandidate],
    dragged_bounds: NativeRect,
    pointer_position: NativePoint,
    dragged_center: NativePoint,
    excluded_slot_id: i32,
) -> Option<i32> {
    let eligible_candidates = candidates
        .iter()
        .copied()
        .filter(|candidate| candidate.slot_id != excluded_slot_id)
        .collect::<Vec<_>>();

    let row_aware_column_candidate = column_aware(&eligible_candidates, pointer_position)
        .filter(|candidate| has_row_peer(&eligible_candidates, *candidate));

    nearest_containing(&eligible_candidates, pointer_position)
        .map(|candidate| candidate.slot_id)
        .or_else(|| row_aware_column_candidate.map(|candidate| candidate.slot_id))
        .or_else(|| {
            largest_overlap_with(&eligible_candidates, dragged_bounds)
                .map(|candidate| candidate.slot_id)
        })
        .or_else(|| {
            nearest_expanded_around(&eligible_candidates, pointer_position)
                .map(|candidate| candidate.slot_id)
        })
        .or_else(|| {
            nearest_containing(&eligible_candidates, dragged_center)
                .map(|candidate| candidate.slot_id)
        })
        .or_else(|| {
            eligible_candidates
                .iter()
                .filter(|candidate| dragged_bounds.contains(candidate.bounds.center()))
                .min_by(|left, right| {
                    compare_f32(
                        left.bounds
                            .center()
                            .distance_squared_to(dragged_bounds.center()),
                        right
                            .bounds
                            .center()
                            .distance_squared_to(dragged_bounds.center()),
                    )
                })
                .map(|candidate| candidate.slot_id)
        })
        .or_else(|| {
            column_aware(&eligible_candidates, pointer_position).map(|candidate| candidate.slot_id)
        })
        .or_else(|| {
            nearest_expanded_around(&eligible_candidates, dragged_center)
                .map(|candidate| candidate.slot_id)
        })
}

fn has_row_peer(
    candidates: &[TimerSlotDropTargetCandidate],
    candidate: TimerSlotDropTargetCandidate,
) -> bool {
    let row_center_y = candidate.bounds.center().y;
    let row_tolerance = (candidate.bounds.height() * 0.18_f32).max(1.0);
    candidates.iter().any(|other| {
        other.slot_id != candidate.slot_id
            && (other.bounds.center().y - row_center_y).abs() <= row_tolerance
    })
}

fn column_aware(
    candidates: &[TimerSlotDropTargetCandidate],
    position: NativePoint,
) -> Option<TimerSlotDropTargetCandidate> {
    let anchor_candidate = candidates.iter().min_by(|left, right| {
        compare_f32(
            left.bounds.horizontal_distance_to(position.x),
            right.bounds.horizontal_distance_to(position.x),
        )
    })?;
    let column_center_x = anchor_candidate.bounds.center().x;
    let column_tolerance = (anchor_candidate.bounds.width() * 0.18_f32).max(1.0);
    let mut column_candidates = candidates
        .iter()
        .copied()
        .filter(|candidate| {
            (candidate.bounds.center().x - column_center_x).abs() <= column_tolerance
        })
        .collect::<Vec<_>>();
    if column_candidates.len() < 2 {
        return None;
    }
    column_candidates
        .sort_by(|left, right| compare_f32(left.bounds.center().y, right.bounds.center().y));
    column_candidates
        .iter()
        .copied()
        .find(|candidate| position.y <= candidate.bounds.center().y)
        .or_else(|| column_candidates.last().copied())
}

fn nearest_containing(
    candidates: &[TimerSlotDropTargetCandidate],
    position: NativePoint,
) -> Option<TimerSlotDropTargetCandidate> {
    candidates
        .iter()
        .copied()
        .filter(|candidate| candidate.bounds.contains(position))
        .min_by(|left, right| {
            compare_f32(
                left.bounds.center().distance_squared_to(position),
                right.bounds.center().distance_squared_to(position),
            )
        })
}

fn nearest_expanded_around(
    candidates: &[TimerSlotDropTargetCandidate],
    position: NativePoint,
) -> Option<TimerSlotDropTargetCandidate> {
    candidates
        .iter()
        .copied()
        .filter(|candidate| candidate.bounds.expanded_contains(position))
        .min_by(|left, right| {
            compare_f32(
                left.bounds.center().distance_squared_to(position),
                right.bounds.center().distance_squared_to(position),
            )
        })
}

fn largest_overlap_with(
    candidates: &[TimerSlotDropTargetCandidate],
    dragged_bounds: NativeRect,
) -> Option<TimerSlotDropTargetCandidate> {
    let minimum_coverage = 0.14_f32;
    let mut best_candidate = None;
    let mut best_coverage = 0.0_f32;
    let mut best_distance = 0.0_f32;

    for candidate in candidates.iter().copied() {
        let overlap_area = candidate.bounds.intersection_area_with(dragged_bounds);
        if overlap_area <= 0.0 {
            continue;
        }
        let coverage = overlap_area / candidate.bounds.area().min(dragged_bounds.area());
        if coverage < minimum_coverage {
            continue;
        }
        let distance = candidate
            .bounds
            .center()
            .distance_squared_to(dragged_bounds.center());
        let better_candidate = match best_candidate {
            None => true,
            Some(_) => {
                coverage > best_coverage || (coverage == best_coverage && distance < best_distance)
            }
        };
        if better_candidate {
            best_candidate = Some(candidate);
            best_coverage = coverage;
            best_distance = distance;
        }
    }

    best_candidate
}

fn compare_f32(left: f32, right: f32) -> Ordering {
    left.partial_cmp(&right).unwrap_or(Ordering::Equal)
}

impl NativePoint {
    fn from_slice(values: &[f32]) -> Option<Self> {
        if values.len() != 2 {
            return None;
        }
        Some(Self {
            x: values[0],
            y: values[1],
        })
    }

    fn distance_squared_to(self, other: Self) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        dx * dx + dy * dy
    }
}

impl NativeRect {
    fn from_slice(values: &[f32]) -> Option<Self> {
        if values.len() != 4 {
            return None;
        }
        Some(Self {
            left: values[0],
            top: values[1],
            right: values[2],
            bottom: values[3],
        })
    }

    fn width(self) -> f32 {
        self.right - self.left
    }

    fn height(self) -> f32 {
        self.bottom - self.top
    }

    fn center(self) -> NativePoint {
        NativePoint {
            x: self.left + (self.width() / 2.0),
            y: self.top + (self.height() / 2.0),
        }
    }

    fn area(self) -> f32 {
        self.width() * self.height()
    }

    fn contains(self, position: NativePoint) -> bool {
        position.x >= self.left
            && position.x <= self.right
            && position.y >= self.top
            && position.y <= self.bottom
    }

    fn inflated_by(self, distance: f32) -> Self {
        Self {
            left: self.left - distance,
            top: self.top - distance,
            right: self.right + distance,
            bottom: self.bottom + distance,
        }
    }

    fn deflated_by(self, distance: f32) -> Self {
        let safe_distance = distance.min(self.width().min(self.height()) * 0.45);
        Self {
            left: self.left + safe_distance,
            top: self.top + safe_distance,
            right: self.right - safe_distance,
            bottom: self.bottom - safe_distance,
        }
    }

    fn expanded_contains(self, position: NativePoint) -> bool {
        let horizontal_expansion = self.width() * 0.18_f32;
        let vertical_expansion = self.height() * 0.18_f32;
        let left = self.left - horizontal_expansion;
        let top = self.top - vertical_expansion;
        let right = self.right + horizontal_expansion;
        let bottom = self.bottom + vertical_expansion;
        position.x >= left && position.x <= right && position.y >= top && position.y <= bottom
    }

    fn horizontal_distance_to(self, x: f32) -> f32 {
        if x < self.left {
            self.left - x
        } else if x > self.right {
            x - self.right
        } else {
            0.0
        }
    }

    fn intersection_area_with(self, other: Self) -> f32 {
        let left = self.left.max(other.left);
        let top = self.top.max(other.top);
        let right = self.right.min(other.right);
        let bottom = self.bottom.min(other.bottom);
        let width = (right - left).max(0.0);
        let height = (bottom - top).max(0.0);
        width * height
    }

    fn translated_by(self, translation: NativePoint) -> Self {
        Self {
            left: self.left + translation.x,
            top: self.top + translation.y,
            right: self.right + translation.x,
            bottom: self.bottom + translation.y,
        }
    }
}

fn compute_action_request_code(seed: i32, slot_ids: &[i32]) -> i32 {
    slot_ids
        .iter()
        .fold(seed, |acc, slot_id| {
            acc.wrapping_mul(31).wrapping_add(*slot_id)
        })
        .wrapping_abs()
}

fn build_xiaomi_payload(state: &XiaomiPayloadState, include_island: bool) -> String {
    let title_28 = trim_to_utf16(&state.title, 28);
    let text_24 = trim_to_utf16(&state.text, 24);
    let big_text_72 = trim_to_utf16(&state.big_text, 72);
    let sub_text_24 = trim_to_utf16(&state.sub_text, 24);
    let share_content_48 = trim_to_utf16(&state.share_content, 48);

    let mut param_v2 = json!({
        "protocol": 1,
        "business": "timer",
        "enableFloat": true,
        "updatable": true,
        "reopen": "reopen",
        "timeout": 720,
        "filterWhenNoPermission": false,
        "ticker": title_28.clone(),
        "tickerPic": MIUI_FOCUS_TIMER_PIC_KEY,
        "tickerPicDark": MIUI_FOCUS_TIMER_PIC_DARK_KEY,
        "aodTitle": title_28.clone(),
        "aodPic": MIUI_FOCUS_TIMER_PIC_KEY,
        "baseInfo": {
            "title": title_28.clone(),
            "content": big_text_72,
            "colorTitle": MIUI_ISLAND_HIGHLIGHT_COLOR,
            "type": 2
        },
        "hintInfo": {
            "type": 1,
            "title": sub_text_24,
            "actionInfo": {
                "action": MIUI_FOCUS_OPEN_ACTION_KEY
            }
        },
        "actions": [
            {
                "action": MIUI_FOCUS_PAUSE_ACTION_KEY
            }
        ]
    });

    if let Some(param_v2_object) = param_v2.as_object_mut() {
        if include_island {
            param_v2_object.insert("islandFirstFloat".to_string(), Value::Bool(true));
            param_v2_object.insert(
                "param_island".to_string(),
                build_xiaomi_island_param(state, title_28, text_24, share_content_48),
            );
        }
    }

    json!({
        "param_v2": param_v2
    })
    .to_string()
}

fn build_xiaomi_island_param(
    state: &XiaomiPayloadState,
    title_28: String,
    text_24: String,
    share_content_48: String,
) -> Value {
    json!({
        "islandProperty": 1,
        "islandOrder": true,
        "islandTimeout": 3_600,
        "dismissIsland": false,
        "highlightColor": MIUI_ISLAND_HIGHLIGHT_COLOR,
        "bigIslandArea": build_xiaomi_island_big_area(state),
        "smallIslandArea": build_xiaomi_island_small_area(state),
        "shareData": {
            "pic": MIUI_FOCUS_TIMER_PIC_KEY,
            "title": title_28,
            "content": text_24,
            "shareContent": share_content_48
        }
    })
}

fn build_xiaomi_island_big_area(state: &XiaomiPayloadState) -> Value {
    json!({
        "imageTextInfoLeft": build_xiaomi_island_image_text_info(1, build_xiaomi_island_primary_text_info(state)),
        "picInfo": build_xiaomi_static_pic_info()
    })
}

fn build_xiaomi_island_small_area(_state: &XiaomiPayloadState) -> Value {
    json!({
        "picInfo": build_xiaomi_static_pic_info()
    })
}

fn build_xiaomi_island_image_text_info(component_type: i32, text_info: Value) -> Value {
    json!({
        "type": component_type,
        "picInfo": build_xiaomi_static_pic_info(),
        "textInfo": text_info
    })
}

fn build_xiaomi_static_pic_info() -> Value {
    json!({
        "type": 1,
        "pic": MIUI_FOCUS_TIMER_PIC_KEY,
        "picDark": MIUI_FOCUS_TIMER_PIC_DARK_KEY
    })
}

fn build_xiaomi_island_primary_text_info(state: &XiaomiPayloadState) -> Value {
    let front_title = if state.running_count == 1 {
        "正在计时"
    } else {
        "并行计时"
    };
    let content = if state.running_count == 1 {
        state.primary_title.clone()
    } else {
        format!("共{}项", state.running_count)
    };
    build_xiaomi_island_text_info(
        front_title,
        trim_to_utf16(&state.primary_elapsed, 16),
        trim_to_utf16(&content, 10),
        true,
    )
}

fn build_xiaomi_island_text_info(
    front_title: &str,
    title: String,
    content: String,
    narrow_font: bool,
) -> Value {
    json!({
        "frontTitle": front_title,
        "title": title,
        "content": content,
        "narrowFont": narrow_font,
        "useHighLight": false
    })
}

fn trim_to_utf16(value: &str, max_length: usize) -> String {
    if value.encode_utf16().count() <= max_length {
        return value.to_string();
    }

    let mut consumed_utf16 = 0;
    let mut byte_end = 0;
    for (byte_index, ch) in value.char_indices() {
        let char_utf16_len = ch.len_utf16();
        if consumed_utf16 + char_utf16_len > max_length {
            break;
        }
        consumed_utf16 += char_utf16_len;
        byte_end = byte_index + ch.len_utf8();
    }
    value[..byte_end].to_string()
}

fn safe_ratio(numerator: i64, denominator: i64) -> f64 {
    if numerator <= 0 || denominator <= 0 {
        return 0.0;
    }
    numerator as f64 / denominator as f64
}

fn within_target_ratio(actual: f64, target: f64) -> bool {
    if actual <= 0.0 || target <= 0.0 {
        return false;
    }
    (actual - target).abs() <= target * 0.15
}

fn finance_period_scale(period_code: jint) -> f64 {
    match period_code {
        0 => 1.0 / 30.0,
        1 => 1.0,
        2 => 3.0,
        3 => 12.0,
        _ => 1.0,
    }
}

fn scale_unsigned_amount(amount: f64, factor: f64) -> f64 {
    (amount * factor).round().max(0.0)
}

fn scale_signed_amount(amount: f64, factor: f64) -> f64 {
    (amount * factor).round()
}

fn scale_defensive_coverage(defensive_months: f64, period_scale: f64) -> f64 {
    if defensive_months.is_nan() {
        return f64::NAN;
    }
    defensive_months / period_scale
}

fn boolean_to_jni(value: bool) -> jboolean {
    if value {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

impl NoteTextAction {
    fn from_native_code(code: jint) -> Option<Self> {
        match code {
            1 => Some(Self::Heading),
            2 => Some(Self::Center),
            3 => Some(Self::BulletList),
            4 => Some(Self::Bold),
            5 => Some(Self::Quote),
            6 => Some(Self::Todo),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        aggregate_finance_day_ledger_values, aggregate_finance_ledgers,
        archived_task_display_title, build_history_search_haystack, build_note_collection_sections,
        build_note_revision_plan, build_timer_session_ui_index_stats, build_xiaomi_payload,
        compute_action_request_code, compute_finance_ledger_hint,
        compute_finance_ledger_hint_with_targets, compute_finance_snapshot,
        compute_micro_break_target_millis, count_note_attachments, count_note_checklist_stats,
        derive_smart_note_digest, display_note_line, finance_bucket_drift_line,
        finance_bucket_ratio_line, finance_bucket_share, finance_coverage_caption,
        finance_ledger_aggregate_values, finance_missing_template_indices, finance_profile,
        finance_scale_float, finance_scale_long, finance_settings_target_summary,
        finance_structure_headline, format_asset_liability_ratio, format_compact_duration_label,
        format_compact_timer_duration, format_countdown_duration, format_currency_amount,
        format_defensive_coverage, format_finance_currency, format_finance_percent,
        format_finance_signed_currency, format_long_duration, format_notification_duration,
        format_rounded_percent, format_signed_currency_amount, format_signed_percent,
        has_finance_row_draft_content, history_query_matches, history_week_summary_note,
        home_focus_suggestion_values, is_finance_bucket_drift_healthy, is_timer_slot_blank_slate,
        is_valid_finance_date_key, is_valid_finance_month_key, latest_finish_badge_label,
        matches_note_quick_filter, micro_break_resolution_values,
        missing_existing_attachment_indices, normalize_query, normalize_repository_text,
        normalize_slot_order, parse_finance_amount_draft, parse_finance_target_share,
        pause_micro_break, rank_knowledge_source_indices, recent_finance_template_indices,
        remove_rich_text_attachment_reference, reorder_timer_slot_ids, resolve_micro_break,
        resolve_timer_slot_drop_target_id, restore_target_slot_id, rich_text_attachment_ids,
        safe_elapsed_millis, sanitize_finance_draft, shift_finance_day_key,
        shift_finance_month_key, should_capture_note_revision,
        should_request_timer_notification_permission, should_run_timer_live_update_refresh,
        should_sync_timer_live_update, slot_label, sort_note_indices,
        sort_trashed_note_indices, stabilize_timer_slot_insert_index,
        summarize_finance_month_snapshot, timer_slot_displacement_target_pairs,
        timer_slot_display_title, timer_tile_auto_scroll_step, timer_tile_today_stats_label,
        transform_note_content, NativePoint, NativeRect, NoteTextAction,
        TimerSlotDropTargetCandidate, TimerSlotInsertCandidate, XiaomiPayloadState,
        CENTER_WRAP_START, FINANCE_BUCKET_BTC, FINANCE_BUCKET_DEBT, FINANCE_BUCKET_FOOD,
        FINANCE_BUCKET_LIVING, FINANCE_BUCKET_OTHER, HEADING_PREFIX, JNI_FALSE, JNI_TRUE,
        LIST_PREFIX, MICRO_BREAK_PHASE_BREAK, MICRO_BREAK_PHASE_FOCUS,
        MICRO_BREAK_TRANSITION_BREAK_STARTED, MICRO_BREAK_TRANSITION_FOCUS_RESUMED, QUOTE_PREFIX,
        TODO_PREFIX,
    };
    use serde_json::Value;

    #[test]
    fn heading_prefixes_current_line_when_selection_is_collapsed() {
        let edit = transform_note_content(NoteTextAction::Heading, "第一行", 0, 0);
        assert_eq!(format!("{HEADING_PREFIX}第一行"), edit.content);
        assert_eq!(2, edit.selection_start_utf16);
        assert_eq!(2, edit.selection_end_utf16);
    }

    #[test]
    fn center_prefix_handles_chinese_selection_offsets() {
        let edit = transform_note_content(NoteTextAction::Center, "中文\n下一行", 1, 1);
        assert_eq!("[中文]\n下一行", edit.content);
        assert_eq!(
            CENTER_WRAP_START.encode_utf16().count() + 1,
            edit.selection_start_utf16
        );
    }

    #[test]
    fn list_toggle_removes_prefixes_from_multiple_lines() {
        let source = format!("{LIST_PREFIX}一\n{LIST_PREFIX}二");
        let edit = transform_note_content(
            NoteTextAction::BulletList,
            &source,
            0,
            source.encode_utf16().count(),
        );
        assert_eq!("一\n二", edit.content);
        assert_eq!(0, edit.selection_start_utf16);
        assert_eq!("一\n二".encode_utf16().count(), edit.selection_end_utf16);
    }

    #[test]
    fn quote_prefixes_whole_selected_block() {
        let source = "甲\n乙";
        let edit = transform_note_content(
            NoteTextAction::Quote,
            source,
            0,
            source.encode_utf16().count(),
        );
        assert_eq!(format!("{QUOTE_PREFIX}甲\n{QUOTE_PREFIX}乙"), edit.content);
    }

    #[test]
    fn todo_prefixes_current_line() {
        let edit = transform_note_content(NoteTextAction::Todo, "完成发布", 0, 0);
        assert_eq!(format!("{TODO_PREFIX}完成发布"), edit.content);
    }

    #[test]
    fn bold_wraps_and_unwraps_selection() {
        let wrapped = transform_note_content(NoteTextAction::Bold, "hello world", 0, 5);
        assert_eq!("**hello** world", wrapped.content);
        assert_eq!(2, wrapped.selection_start_utf16);
        assert_eq!(7, wrapped.selection_end_utf16);

        let unwrapped = transform_note_content(
            NoteTextAction::Bold,
            wrapped.content.as_str(),
            wrapped.selection_start_utf16,
            wrapped.selection_end_utf16,
        );
        assert_eq!("hello world", unwrapped.content);
        assert_eq!(0, unwrapped.selection_start_utf16);
        assert_eq!(5, unwrapped.selection_end_utf16);
    }

    #[test]
    fn rich_text_attachment_ids_preserve_data_then_src_order_without_duplicates() {
        let html = r#"
            <p>before</p>
            <figure data-note-image="att-2"><img src="note-image://att-2" /></figure>
            <img src='note-image://att-1'>
            <figure DATA-NOTE-IMAGE='att-3'></figure>
        "#;

        assert_eq!(
            vec![
                "att-2".to_string(),
                "att-3".to_string(),
                "att-1".to_string()
            ],
            rich_text_attachment_ids(html)
        );
    }

    #[test]
    fn remove_rich_text_attachment_reference_removes_matching_figure_and_img_only() {
        let html = r#"
            <p>before</p>
            <figure data-note-image="att-1">
                <img src="note-image://att-1" alt="one" />
            </figure>
            <p>after</p>
            <img src="note-image://att-1" />
            <figure data-note-image="att-2">
                <img src="note-image://att-2" alt="two" />
            </figure>
        "#;

        let updated = remove_rich_text_attachment_reference(html, "att-1");

        assert!(!updated.contains("att-1"));
        assert!(updated.contains("att-2"));
        assert!(updated.contains("before"));
        assert!(updated.contains("after"));
    }

    #[test]
    fn note_collection_sections_pin_then_group_regular_notes_by_descending_day() {
        let encoded = build_note_collection_sections(
            &[20, 22, 21, 22, 19],
            &[JNI_FALSE, JNI_TRUE, JNI_FALSE, JNI_FALSE, JNI_TRUE],
        );

        assert_eq!(
            vec![
                4,
                0,
                i64::MIN,
                2,
                1,
                4,
                1,
                22,
                1,
                3,
                1,
                21,
                1,
                2,
                1,
                20,
                1,
                0,
            ],
            encoded
        );
    }

    #[test]
    fn normalize_slot_order_keeps_valid_first_occurrence_then_fills_missing_slots() {
        assert_eq!(
            vec![4, 2, 1, 3, 5],
            normalize_slot_order(&[4, 99, 2, 4, 1], 1, 5)
        );
        assert!(normalize_slot_order(&[1, 2], 5, 1).is_empty());
    }

    #[test]
    fn timer_slot_blank_slate_matches_repository_defaults() {
        assert!(is_timer_slot_blank_slate(
            "  ",
            false,
            "",
            0,
            -1,
            MICRO_BREAK_PHASE_FOCUS,
            0,
            0,
        ));
        assert!(!is_timer_slot_blank_slate(
            "Focus",
            false,
            "",
            0,
            -1,
            MICRO_BREAK_PHASE_FOCUS,
            0,
            0,
        ));
        assert!(!is_timer_slot_blank_slate(
            "",
            false,
            "",
            0,
            1_000,
            MICRO_BREAK_PHASE_FOCUS,
            0,
            0,
        ));
    }

    #[test]
    fn restore_target_prefers_original_blank_slot_then_first_blank() {
        assert_eq!(
            Some(3),
            restore_target_slot_id(3, &[1, 2, 3], &[JNI_TRUE, JNI_FALSE, JNI_TRUE])
        );
        assert_eq!(
            Some(1),
            restore_target_slot_id(4, &[1, 2, 3], &[JNI_TRUE, JNI_FALSE, JNI_TRUE])
        );
        assert_eq!(
            None,
            restore_target_slot_id(4, &[1, 2, 3], &[JNI_FALSE, JNI_FALSE, JNI_FALSE])
        );
    }

    #[test]
    fn finance_date_key_validation_matches_iso_calendar_edges() {
        assert!(is_valid_finance_date_key("2026-02-28"));
        assert!(is_valid_finance_date_key("2024-02-29"));
        assert!(is_valid_finance_date_key("0000-02-29"));
        assert!(!is_valid_finance_date_key("2026-02-29"));
        assert!(!is_valid_finance_date_key("2026-13-01"));
        assert!(!is_valid_finance_date_key("2026-1-01"));

        assert!(is_valid_finance_month_key("2026-12"));
        assert!(is_valid_finance_month_key("0000-01"));
        assert!(!is_valid_finance_month_key("2026-00"));
        assert!(!is_valid_finance_month_key("2026-1"));
    }

    #[test]
    fn note_checklist_stats_counts_open_and_completed_items() {
        assert_eq!(
            (4, 2),
            count_note_checklist_stats(" - [ ] one\r\n- [x] two\n- [X] three\n- [ ] four\n- nope")
        );
        assert_eq!((0, 0), count_note_checklist_stats("plain\ntext"));
    }

    #[test]
    fn note_attachment_count_matches_kind_filter() {
        assert_eq!(3, count_note_attachments(&[0, 0, 0], -1));
        assert_eq!(2, count_note_attachments(&[0, 1, 0], 0));
        assert_eq!(1, count_note_attachments(&[0, 1, 0], 1));
        assert_eq!(0, count_note_attachments(&[0, 0], 2));
    }

    #[test]
    fn display_note_line_matches_kotlin_note_plain_text_rules() {
        assert_eq!("Title", display_note_line("**# Title**", true));
        assert_eq!("Task", display_note_line("- [ ] Task", false));
        assert_eq!("\u{2610} Task", display_note_line("- [ ] Task", true));
        assert_eq!("\u{2611} Done", display_note_line("- [X] Done", true));
        assert_eq!("\u{201c}Quote\u{201d}", display_note_line("> Quote", true));
        assert_eq!("\u{2022} Item", display_note_line("- Item", true));
        assert_eq!("Centered", display_note_line("[Centered]", true));
        assert_eq!(
            "\u{2610} \u{7b2c}\u{4e00}\u{4ef6}\u{4e8b}",
            display_note_line("\u{2610} \u{7b2c}\u{4e00}\u{4ef6}\u{4e8b}", true)
        );
        assert_eq!(
            "Legacy",
            display_note_line("\u{3010}\u{5c45}\u{4e2d}\u{3011}Legacy", true)
        );
    }

    #[test]
    fn smart_note_digest_accepts_displayed_unicode_todo_lines() {
        let (title, preview) = derive_smart_note_digest(
            "",
            "\u{2610} \u{7b2c}\u{4e00}\u{4ef6}\u{4e8b}\n\u{1f4a1} \u{5148}\u{8bb0}\u{4e0b}\u{6765}",
            0,
        );

        assert_eq!("\u{2610} \u{7b2c}\u{4e00}\u{4ef6}\u{4e8b}", title);
        assert_eq!("\u{1f4a1} \u{5148}\u{8bb0}\u{4e0b}\u{6765}", preview);
    }

    #[test]
    fn home_focus_suggestion_prefers_running_then_recent_then_first_empty() {
        assert_eq!(
            Some([3, 0]),
            home_focus_suggestion_values(&[1, 2, 3], &[1, 0, 0], &[-1, 20, 30], &[1, 2, 3])
        );
        assert_eq!(
            Some([2, 1]),
            home_focus_suggestion_values(&[1, 2, 3], &[1, 0, 0], &[-1, -1, -1], &[3, 9, 4])
        );
        assert_eq!(
            Some([1, 2]),
            home_focus_suggestion_values(&[3, 1, 2], &[1, 1, 1], &[-1, -1, -1], &[3, 9, 4])
        );
    }

    #[test]
    fn safe_elapsed_millis_clamps_invalid_and_huge_values() {
        assert_eq!(
            15_000,
            safe_elapsed_millis(10_000, 95_000, MICRO_BREAK_PHASE_FOCUS, 100_000)
        );
        assert_eq!(
            10_000,
            safe_elapsed_millis(10_000, 95_000, MICRO_BREAK_PHASE_BREAK, 100_000)
        );
        assert_eq!(
            0,
            safe_elapsed_millis(-1, 110_000, MICRO_BREAK_PHASE_FOCUS, 100_000)
        );
        assert_eq!(
            super::MAX_TRACKED_DURATION_MILLIS,
            safe_elapsed_millis(i64::MAX, 0, MICRO_BREAK_PHASE_FOCUS, i64::MAX)
        );
    }

    #[test]
    fn format_notification_duration_matches_expected_lengths() {
        assert_eq!("00:15", format_notification_duration(15_000));
        assert_eq!("01:40", format_notification_duration(100_000));
        assert_eq!("01:00:00", format_notification_duration(3_600_000));
        assert_eq!("00:00", format_notification_duration(-1));
    }

    #[test]
    fn ui_duration_formatters_match_kotlin_outputs() {
        assert_eq!("01:01:01", format_long_duration(3_661_000));
        assert_eq!(
            "123h 45m",
            format_long_duration((123 * 3_600 + 45 * 60) * 1_000)
        );
        assert_eq!(
            "03:43",
            format_compact_timer_duration((3 * 60 + 43) * 1_000)
        );
        assert_eq!("4天", format_compact_timer_duration(100 * 3_600 * 1_000));
        assert_eq!("1小时 30分", format_compact_duration_label(90 * 60 * 1_000));
        assert_eq!(
            "今日 30分 · 占 25%",
            timer_tile_today_stats_label(30 * 60 * 1_000, 120 * 60 * 1_000)
        );
    }

    #[test]
    fn timer_slot_reorder_helpers_match_kotlin_logic() {
        assert_eq!(
            Some(vec![1, 3, 5, 2, 6]),
            reorder_timer_slot_ids(&[1, 5, 3, 2, 6], 5, 2)
        );
        assert_eq!(
            vec![2, 1, 3, 2],
            timer_slot_displacement_target_pairs(&[1, 2, 3, 4], &[2, 3, 1, 4], 1)
        );
        assert_eq!(0.0, timer_tile_auto_scroll_step(10.0, 104.0));
        assert!(
            timer_tile_auto_scroll_step(80.0, 104.0) > timer_tile_auto_scroll_step(20.0, 104.0)
        );
    }

    #[test]
    fn timer_session_ui_index_stats_preserve_first_max_semantics() {
        let stats = build_timer_session_ui_index_stats(
            &[2, 1, 2, 1],
            &[100, 300, 300, 50],
            &[10, 80, 120, 200],
            &[1, 2, 2],
            &[1, 2],
        )
        .expect("valid stats");

        assert_eq!(vec![1, 3, 1, 2, 1, 1, 2, 2, 2, 2], stats);
    }

    #[test]
    fn finance_detail_aggregation_matches_month_quarter_and_year_totals() {
        let ledger_days = [20260401, 20260402, 20260501];
        let ledger_notes = [0, 0, 0];
        let income_days = [20260401, 20260402, 20260501];
        let income_kinds = [0, 0, 0];
        let income_amounts = [1_000, 500, 700];
        let expense_days = [20260401, 20260401, 20260402, 20260501];
        let expense_buckets = [0, 1, 4, 3];
        let expense_amounts = [300, 100, 50, 200];

        let month = aggregate_finance_ledgers(
            1,
            2026,
            4,
            1,
            &ledger_days,
            &ledger_notes,
            &income_days,
            &income_kinds,
            &income_amounts,
            &expense_days,
            &expense_buckets,
            &expense_amounts,
        )
        .expect("valid month aggregate");
        let quarter = aggregate_finance_ledgers(
            2,
            2026,
            4,
            1,
            &ledger_days,
            &ledger_notes,
            &income_days,
            &income_kinds,
            &income_amounts,
            &expense_days,
            &expense_buckets,
            &expense_amounts,
        )
        .expect("valid quarter aggregate");

        assert_eq!(
            [1_500, 0, 0, 300, 100, 0, 0, 50, 0, 2],
            finance_ledger_aggregate_values(month)
        );
        assert_eq!(
            [2_200, 0, 0, 300, 100, 0, 200, 50, 0, 3],
            finance_ledger_aggregate_values(quarter)
        );
    }

    #[test]
    fn finance_month_snapshot_summary_groups_assets_by_kind() {
        let summary =
            summarize_finance_month_snapshot(&[0, 1, 2], &[5_000, 20_000, 800], &[4_000, -1])
                .expect("valid month summary");

        assert_eq!([25_800, 4_000, 5_000, 20_000], summary);
    }

    #[test]
    fn finance_day_ledger_aggregate_groups_rows_and_note_presence() {
        let summary = aggregate_finance_day_ledger_values(
            &[0, 1, 2],
            &[800, 120, -3],
            &[0, 1, 5],
            &[200, 50, 25],
            true,
        )
        .expect("valid day ledger");

        assert_eq!([800, 120, 0, 200, 50, 0, 0, 0, 25, 1], summary);
    }

    #[test]
    fn finance_snapshot_saturates_extreme_amounts() {
        let snapshot = compute_finance_snapshot(
            i64::MAX,
            i64::MAX,
            i64::MAX,
            i64::MAX,
            i64::MAX,
            i64::MAX,
            i64::MAX,
        );

        assert_eq!(i64::MAX, snapshot[0] as i64);
        assert_eq!(i64::MAX, snapshot[1] as i64);
        assert_eq!(0, snapshot[2] as i64);
        assert_eq!(0, snapshot[3] as i64);
        assert_eq!(0, snapshot[7] as i64);
    }

    #[test]
    fn finance_snapshot_ignores_negative_imported_amounts() {
        let snapshot = compute_finance_snapshot(-10, -20, -30, -40, -50, -60, -70);

        assert_eq!(0, snapshot[0] as i64);
        assert_eq!(0, snapshot[1] as i64);
        assert_eq!(0, snapshot[2] as i64);
        assert_eq!(0, snapshot[7] as i64);
    }

    #[test]
    fn finance_ledger_hint_prioritizes_high_debt_before_near_count() {
        assert_eq!(
            2,
            compute_finance_ledger_hint(0, 1_000, 720, 430, 190, 90, 0, 100, 280, 0, 0,)
        );
        assert_eq!(
            1,
            compute_finance_ledger_hint(1, 1_000, 700, 300, 200, 100, 300, 100, 300, 0, 0,)
        );
    }

    #[test]
    fn recent_finance_template_indices_sort_and_distinct_by_kind_and_name() {
        let names = vec![
            "Salary".to_string(),
            "Book".to_string(),
            "salary".to_string(),
            "  ".to_string(),
            "Book".to_string(),
        ];

        let indices = recent_finance_template_indices(
            &[20260401, 20260403, 20260404, 20260405, 20260402],
            &[0, 4, 0, 1, 5],
            &names,
            3,
        );

        assert_eq!(vec![2, 1, 4], indices);
    }

    #[test]
    fn finance_missing_template_indices_skip_present_and_duplicate_templates() {
        let row_names = vec![" salary ".to_string(), "Rent".to_string()];
        let template_names = vec![
            "Salary".to_string(),
            "Book".to_string(),
            "book".to_string(),
            "  ".to_string(),
            "Rent".to_string(),
        ];

        assert_eq!(
            vec![1, 4],
            finance_missing_template_indices(
                &[0, 3],
                &row_names,
                &[0, 4, 4, 1, 0],
                &template_names,
            )
        );
    }

    #[test]
    fn finance_profile_json_trend_values_compare_current_and_previous_periods() {
        let profile_json = r#"{
            "settings": {
                "expenseCategories": [
                    {"bucket": "DEBT", "label": "Debt", "targetShareOfIncome": 0.30},
                    {"bucket": "FOOD", "label": "Food", "targetShareOfIncome": 0.20},
                    {"bucket": "BTC", "label": "Btc", "targetShareOfIncome": 0.10},
                    {"bucket": "LIVING", "label": "Living", "targetShareOfIncome": 0.30},
                    {"bucket": "LEARNING", "label": "Learning", "targetShareOfIncome": 0.10}
                ]
            },
            "dailyLedgers": {
                "2026-03-20": {
                    "incomes": [{"name": "Old", "kind": "ACTIVE", "amount": 800}],
                    "expenses": [{"name": "Old rent", "bucket": "LIVING", "amount": 300}]
                },
                "2026-04-18": {
                    "incomes": [{"name": "Salary", "kind": "ACTIVE", "amount": 1000}],
                    "expenses": [
                        {"name": "Debt", "bucket": "DEBT", "amount": 650},
                        {"name": "Food", "bucket": "FOOD", "amount": 100}
                    ]
                }
            },
            "monthlySnapshots": {
                "2026-03": {
                    "assets": [{"name": "Cash", "kind": "CASH_RESERVE", "amount": 2000}],
                    "liabilities": [{"name": "Card", "kind": "LIABILITY_BALANCE", "amount": 500}]
                },
                "2026-04": {
                    "assets": [
                        {"name": "Cash", "kind": "CASH_RESERVE", "amount": 3000},
                        {"name": "Fund", "kind": "PRODUCTIVE_ASSET", "amount": 5000}
                    ],
                    "liabilities": [{"name": "Card", "kind": "LIABILITY_BALANCE", "amount": 1000}]
                }
            }
        }"#;

        let trend = finance_profile::build_finance_trend_values(profile_json, 1, 2026, 4, 18)
            .expect("trend should parse");

        assert_eq!(200, trend.income_delta);
        assert_eq!(450, trend.outflow_delta);
        assert_eq!(-250, trend.net_cashflow_delta);
        assert_eq!(5500, trend.net_worth_delta);
        assert_eq!(1, trend.current_recorded_days);
        assert_eq!(1, trend.previous_recorded_days);
        assert_eq!(FINANCE_BUCKET_DEBT, trend.most_off_target_bucket_code);
        assert!(trend.most_off_target_ratio_delta > 0.30);
    }

    #[test]
    fn finance_profile_json_alert_plan_orders_warnings_before_info() {
        let profile_json = r#"{
            "dailyLedgers": {
                "2026-04-18": {
                    "incomes": [{"name": "Salary", "kind": "ACTIVE", "amount": 1000}],
                    "expenses": [
                        {"name": "Debt", "bucket": "DEBT", "amount": 700},
                        {"name": "Food", "bucket": "FOOD", "amount": 600}
                    ]
                }
            },
            "monthlySnapshots": {
                "2026-04": {
                    "assets": [{"name": "Cash", "kind": "CASH_RESERVE", "amount": 100}]
                }
            }
        }"#;

        let plan = finance_profile::build_finance_alert_plan_values(profile_json, 1, 2026, 4, 18)
            .expect("plan should parse");

        assert_eq!(9, plan.len());
        assert_eq!(1, plan[0]);
        assert_eq!(2, plan[3]);
        assert_eq!(4, plan[6]);
        assert_eq!(FINANCE_BUCKET_DEBT, plan[7]);
    }

    #[test]
    fn detailed_finance_snapshot_values_match_kotlin_detail_rules() {
        let profile_json = r#"{
            "dailyLedgers": {
                "2026-04-18": {
                    "incomes": [
                        {"name": "Salary", "kind": "ACTIVE", "amount": 1000},
                        {"name": "Asset", "kind": "ASSET", "amount": 100}
                    ],
                    "expenses": [
                        {"name": "Debt", "bucket": "DEBT", "amount": 300},
                        {"name": "Food", "bucket": "FOOD", "amount": 200}
                    ]
                }
            },
            "monthlySnapshots": {
                "2026-04": {
                    "assets": [
                        {"name": "Cash", "kind": "CASH_RESERVE", "amount": 800},
                        {"name": "Fund", "kind": "PRODUCTIVE_ASSET", "amount": 2000}
                    ],
                    "liabilities": [
                        {"name": "Loan", "kind": "LIABILITY_BALANCE", "amount": 500}
                    ]
                }
            }
        }"#;

        let snapshot =
            finance_profile::build_detailed_finance_snapshot_values(profile_json, 2026, 4)
                .expect("snapshot should parse");
        let report =
            finance_profile::build_detailed_finance_report_values(profile_json, 1, 2026, 4, 18)
                .expect("report should parse");

        assert_eq!(1_100, snapshot.total_income);
        assert_eq!(500, snapshot.total_outflow);
        assert_eq!(600, snapshot.net_cashflow);
        assert_eq!(400, snapshot.freedom_gap);
        assert_eq!(2_300, snapshot.net_worth);
        assert_eq!(1, report.recorded_days);
        assert!((snapshot.defensive_coverage.unwrap() - 2.0).abs() < 0.0001);
        assert!((snapshot.asset_yield_ratio - 0.05).abs() < 0.0001);
    }

    #[test]
    fn finance_ledger_hint_uses_custom_target_shares() {
        let default_hint =
            compute_finance_ledger_hint(1, 1_000, 700, 300, 200, 100, 0, 100, 300, 0, 0);
        let custom_hint = compute_finance_ledger_hint_with_targets(
            1,
            1_000,
            700,
            300,
            200,
            100,
            0,
            100,
            300,
            0,
            0,
            &[0.15, 0.20, 0.10, 0.30, 0.10],
        );

        assert_eq!(1, default_hint);
        assert_eq!(2, custom_hint);
    }

    #[test]
    fn finance_profile_json_record_key_helpers_match_sorted_kotlin_rules() {
        let profile_json = r#"{
            "dailyLedgers": {
                "bad": {"note": "skip"},
                "2026-04-01": {"note": "one"},
                "2026-04-10": {"note": "two"}
            },
            "monthlySnapshots": {
                "2026-02": {"assets": [{"name": "Cash", "kind": "CASH_RESERVE", "amount": 1000}]},
                "2026-04": {"assets": [{"name": "Cash", "kind": "CASH_RESERVE", "amount": 1500}]}
            }
        }"#;

        assert_eq!(
            Some("2026-04-01"),
            finance_profile::previous_recorded_day_key(profile_json, "2026-04-09").as_deref()
        );
        assert_eq!(
            Some("2026-02"),
            finance_profile::previous_recorded_month_key(profile_json, "2026-04").as_deref()
        );
        assert_eq!(
            Some("2026-02"),
            finance_profile::latest_snapshot_month_key_up_to(profile_json, "2026-03").as_deref()
        );
        assert_eq!(
            Some([1000, 1500]),
            finance_profile::year_net_worth_summary_values(profile_json, 2026)
        );
    }

    #[test]
    fn finance_backup_decoder_accepts_payload_and_raw_profile() {
        let payload_json = r#"{
            "schemaVersion": 1,
            "appVersionName": "2.10.6",
            "financeProfile": {
                "activeIncomeMonthly": 1000,
                "assetIncomeMonthly": -5,
                "dailyLedgers": {
                    "2026-04-21": {
                        "incomes": [{"name": " Salary ", "kind": "ACTIVE", "amount": 500}]
                    }
                }
            }
        }"#;
        let raw_profile_json = r#"{
            "activeIncomeMonthly": 1200,
            "monthlySnapshots": {
                "2026-04": {
                    "assets": [{"name": "Cash", "kind": "CASH_RESERVE", "amount": 3000}]
                }
            }
        }"#;

        let payload_profile = finance_profile::decode_finance_backup_profile_json(payload_json)
            .expect("payload should decode");
        let raw_profile = finance_profile::decode_finance_backup_profile_json(raw_profile_json)
            .expect("raw profile should decode");

        assert!(payload_profile.contains("\"activeIncomeMonthly\":1000"));
        assert!(payload_profile.contains("\"assetIncomeMonthly\":0"));
        assert!(raw_profile.contains("\"activeIncomeMonthly\":1200"));
        assert!(finance_profile::decode_finance_backup_profile_json(
            r#"{"schemaVersion": 99, "financeProfile": {}}"#
        )
        .is_none());
    }

    #[test]
    fn finance_settings_json_helpers_match_kotlin_config_rules() {
        let settings_json = r#"{
            "expenseCategories": [
                {"bucket": "FOOD", "label": "  DiningBudgetLongName  ", "targetShareOfIncome": 1.5},
                {"bucket": "OTHER", "label": "Ignored misc", "targetShareOfIncome": null}
            ]
        }"#;
        let sanitized = finance_profile::sanitize_finance_settings_json(settings_json)
            .expect("settings should sanitize");
        let parsed: serde_json::Value =
            serde_json::from_str(&sanitized).expect("settings json should be valid");
        let categories = parsed["expenseCategories"]
            .as_array()
            .expect("settings should contain categories");

        assert_eq!(6, categories.len());
        assert_eq!("DEBT", categories[0]["bucket"]);
        assert_eq!("FOOD", categories[1]["bucket"]);
        assert_eq!("DiningBudget", categories[1]["label"]);
        assert_eq!(1.0, categories[1]["targetShareOfIncome"]);

        let config = finance_profile::sanitize_finance_expense_category_config_json(
            r#"{"bucket":"OTHER","label":"  DebtCustomName  ","targetShareOfIncome":-0.2}"#,
            FINANCE_BUCKET_DEBT,
        )
        .expect("config should sanitize");
        let config_value: serde_json::Value =
            serde_json::from_str(&config).expect("config json should be valid");
        assert_eq!("DEBT", config_value["bucket"]);
        assert_eq!("DebtCustomNa", config_value["label"]);
        assert_eq!(0.0, config_value["targetShareOfIncome"]);

        let default_config =
            finance_profile::default_finance_expense_category_config_json(FINANCE_BUCKET_BTC)
                .expect("default config should encode");
        assert!(default_config.contains(r#""bucket":"BTC""#));
        assert!(finance_profile::default_finance_expense_category_config_json(99).is_none());
    }

    #[test]
    fn finance_period_scaling_and_bucket_shares_match_model_fallbacks() {
        assert_eq!(600, finance_scale_long(0, 18_000, 0));
        assert_eq!(-400, finance_scale_long(0, -12_000, 1));
        assert_eq!(999_999_999, finance_scale_long(1, i64::MAX, 2));
        assert_eq!(54_000, finance_scale_long(2, 18_000, 0));
        assert_eq!(216_000, finance_scale_long(3, 18_000, 0));
        assert!((finance_scale_float(0, 4.0, 0) - 120.0).abs() < 0.0001);
        assert!((finance_scale_float(2, 0.05, 1) - 0.15).abs() < 0.0001);
        assert!(finance_scale_float(1, f32::NAN, 0).is_nan());

        let income_share =
            finance_bucket_share(FINANCE_BUCKET_DEBT, 1, 1_000, 650, 300, 100, 50, 200, 0, 0);
        let expense_share = finance_bucket_share(
            FINANCE_BUCKET_LIVING,
            0,
            1_000,
            650,
            300,
            100,
            50,
            200,
            0,
            0,
        );

        assert!((income_share - 0.30).abs() < 0.0001);
        assert!((expense_share - (200.0 / 650.0)).abs() < 0.0001);
        assert_eq!(
            0.0,
            finance_bucket_share(FINANCE_BUCKET_FOOD, 1, 0, 0, 0, 10, 0, 0, 0, 0)
        );
    }

    #[test]
    fn finance_target_summary_joins_only_primary_configured_buckets() {
        let summary = finance_settings_target_summary(
            &[
                FINANCE_BUCKET_DEBT,
                FINANCE_BUCKET_FOOD,
                FINANCE_BUCKET_OTHER,
                FINANCE_BUCKET_BTC,
                99,
            ],
            &[
                "Debt".to_string(),
                " Food ".to_string(),
                "Other".to_string(),
                "BTC".to_string(),
                "Bad".to_string(),
            ],
            &[0.30, 0.20, 0.50, f32::NAN, 0.10],
        );

        assert_eq!("Debt 30% / Food 20%", summary);
    }

    #[test]
    fn repository_text_and_attachment_helpers_match_kotlin_rules() {
        assert_eq!(
            "Title   keeps  spaces  ",
            normalize_repository_text("   Title   keeps  spaces  ", 32, true, false)
        );
        assert_eq!(
            "Project Alpha",
            normalize_repository_text("  Project\t\tAlpha  ", 32, false, true)
        );
        assert_eq!(
            "Project",
            normalize_repository_text("  Project Alpha  ", 7, false, true)
        );

        let missing = missing_existing_attachment_indices(
            &[
                "a".to_string(),
                "b".to_string(),
                "a".to_string(),
                "c".to_string(),
            ],
            &["b".to_string()],
        );

        assert_eq!(vec![0, 2, 3], missing);
    }

    #[test]
    fn repository_revision_capture_keeps_time_and_metadata_policy() {
        assert!(!should_capture_note_revision(false, true, None, 10_000));
        assert!(should_capture_note_revision(true, false, None, 10_000));
        assert!(should_capture_note_revision(true, false, Some(0), 45_000));
        assert!(!should_capture_note_revision(
            true,
            false,
            Some(1_000),
            20_000
        ));
        assert!(should_capture_note_revision(
            true,
            true,
            Some(1_000),
            20_000
        ));
    }

    #[test]
    fn repository_revision_plan_orders_dedupes_and_inserts_snapshot() {
        let ids = vec![
            "a".to_string(),
            "b".to_string(),
            "a".to_string(),
            "snapshot".to_string(),
            "c".to_string(),
        ];
        let plan = build_note_revision_plan(
            &ids,
            &[10, 30, 50, 40, 20],
            &[JNI_FALSE, JNI_TRUE, JNI_FALSE, JNI_FALSE, JNI_FALSE],
            true,
            true,
            "snapshot",
            3,
        );
        let no_capture = build_note_revision_plan(
            &ids,
            &[10, 30, 50, 40, 20],
            &[JNI_FALSE, JNI_TRUE, JNI_FALSE, JNI_FALSE, JNI_FALSE],
            true,
            false,
            "snapshot",
            3,
        );

        assert_eq!(vec![-1, 4, 0], plan);
        assert_eq!(vec![3, 1, 4], no_capture);
    }

    #[test]
    fn micro_break_reducer_creates_session_and_transitions() {
        let target = compute_micro_break_target_millis(1, 0);
        let resolution = resolve_micro_break(
            1_000_000 + target + 20_000,
            1,
            0,
            1_000_000,
            MICRO_BREAK_PHASE_FOCUS,
            0,
            0,
            1_000_000,
        )
        .expect("valid micro break resolution");
        let values = micro_break_resolution_values(&resolution);

        assert_eq!(target, resolution.accumulated_millis);
        assert_eq!(MICRO_BREAK_PHASE_FOCUS, resolution.phase);
        assert_eq!(1, resolution.cycle_index);
        assert_eq!(
            1_000_000 + target + 15_000,
            resolution.running_since_epoch_millis
        );
        assert_eq!(1, resolution.sessions.len());
        assert_eq!(target, resolution.sessions[0].duration_millis);
        assert_eq!(2, resolution.transitions.len());
        assert_eq!(
            MICRO_BREAK_TRANSITION_BREAK_STARTED,
            resolution.transitions[0].transition_type
        );
        assert_eq!(
            MICRO_BREAK_TRANSITION_FOCUS_RESUMED,
            resolution.transitions[1].transition_type
        );
        assert_eq!(8 + 3 + 4, values.len());
    }

    #[test]
    fn micro_break_pause_creates_focus_session() {
        let values = pause_micro_break(
            1_015_000,
            1,
            100_000,
            1_000_000,
            MICRO_BREAK_PHASE_FOCUS,
            0,
            20_000,
            1_000_000,
        )
        .expect("valid pause result");

        assert_eq!(115_000, values[0]);
        assert_eq!(-1, values[1]);
        assert_eq!(MICRO_BREAK_PHASE_FOCUS as i64, values[2]);
        assert_eq!(35_000, values[4]);
        assert_eq!(1_015_000, values[5]);
        assert_eq!(1, values[6]);
        assert_eq!(1_000_000, values[7]);
        assert_eq!(1_015_000, values[8]);
        assert_eq!(15_000, values[9]);
    }

    #[test]
    fn micro_break_pause_break_phase_does_not_create_session() {
        let values = pause_micro_break(
            2_005_000,
            1,
            180_000,
            2_000_000,
            MICRO_BREAK_PHASE_BREAK,
            0,
            6_000,
            2_000_000,
        )
        .expect("valid pause result");

        assert_eq!(180_000, values[0]);
        assert_eq!(-1, values[1]);
        assert_eq!(MICRO_BREAK_PHASE_BREAK as i64, values[2]);
        assert_eq!(11_000, values[4]);
        assert_eq!(0, values[6]);
    }

    #[test]
    fn finance_ui_helpers_match_kotlin_outputs() {
        assert_eq!("等待底稿", finance_structure_headline(0, 100, -100));
        assert_eq!("现金流为正", finance_structure_headline(200, 100, 100));
        assert_eq!("继续收口", finance_structure_headline(100, 200, -100));
        assert_eq!("2026-04-21", sanitize_finance_draft("20xx26-04-21zzz", 10));
        assert_eq!(
            129_999_999,
            parse_finance_amount_draft("12x9999999999", 9, 999_999_999)
        );
        assert_eq!(Some(1.0), parse_finance_target_share("128"));
        assert_eq!("¥1,234,567", format_finance_currency(1_234_567));
        assert_eq!("-¥42", format_finance_signed_currency(-42));
        assert_eq!("12.3%", format_finance_percent(0.1234));
        assert_eq!(
            "占收入 20.0% · 目标 30.0%",
            finance_bucket_ratio_line(0.2, 0.4, 0.3)
        );
        assert_eq!(
            "占支出 40.0%",
            finance_bucket_ratio_line(0.2, 0.4, f32::NAN)
        );
        assert_eq!("高于目标 10.0%", finance_bucket_drift_line(0.4, 0.3));
        assert!(is_finance_bucket_drift_healthy(
            FINANCE_BUCKET_BTC,
            0.12,
            0.10
        ));
        assert!(!is_finance_bucket_drift_healthy(
            FINANCE_BUCKET_DEBT,
            0.42,
            0.30
        ));
    }

    #[test]
    fn note_quick_filter_helpers_match_expected_filters() {
        assert!(matches_note_quick_filter(0, false, 0, 0, true));
        assert!(matches_note_quick_filter(1, true, 0, 0, true));
        assert!(!matches_note_quick_filter(1, false, 0, 0, true));
        assert!(matches_note_quick_filter(2, false, 1, 0, true));
        assert!(matches_note_quick_filter(3, false, 0, 1, true));
        assert!(matches_note_quick_filter(4, false, 0, 0, false));
        assert!(!matches_note_quick_filter(4, false, 0, 0, true));
    }

    #[test]
    fn finance_home_formatters_match_kotlin_outputs() {
        assert_eq!("¥12,345", format_currency_amount(12_345, "¥"));
        assert_eq!("¥0", format_currency_amount(-1, "¥"));
        assert_eq!("+¥12", format_signed_currency_amount(12, "¥"));
        assert_eq!("-¥12", format_signed_currency_amount(-12, "¥"));
        assert_eq!("0%", format_rounded_percent(0.004));
        assert_eq!("13%", format_rounded_percent(0.125));
        assert_eq!("+12.5%", format_signed_percent(0.125));
        assert_eq!("-12.5%", format_signed_percent(-0.125));
        assert_eq!("已覆盖", format_defensive_coverage(f32::NAN, "个月"));
        assert_eq!("0.0 个月", format_defensive_coverage(0.0, "个月"));
        assert_eq!("99+ 个月", format_defensive_coverage(120.0, "个月"));
        assert_eq!("2.5 个月", format_defensive_coverage(2.5, "个月"));
        assert_eq!("现金储备还能顶多少天", finance_coverage_caption(0));
        assert_eq!("现金储备还能顶多少个季度", finance_coverage_caption(2));
    }

    #[test]
    fn asset_liability_ratio_formatter_matches_finance_detail_caption() {
        assert_eq!("2.50", format_asset_liability_ratio(10_000, 4_000));
        assert_eq!("\u{221e}", format_asset_liability_ratio(10_000, 0));
        assert_eq!("0.00", format_asset_liability_ratio(0, 0));
    }

    #[test]
    fn finance_key_shifters_match_local_date_edges() {
        assert_eq!(
            Some("2024-02-29".to_string()),
            shift_finance_day_key("2024-02-28", 1)
        );
        assert_eq!(
            Some("2026-03-01".to_string()),
            shift_finance_day_key("2026-02-28", 1)
        );
        assert_eq!(
            Some("2025-12".to_string()),
            shift_finance_month_key("2026-01", -1)
        );
        assert_eq!(
            Some("2027-01".to_string()),
            shift_finance_month_key("2026-12", 1)
        );
        assert_eq!(None, shift_finance_day_key("2026-02-29", 1));
        assert_eq!(None, shift_finance_month_key("2026-00", 1));
    }

    #[test]
    fn timer_and_history_text_helpers_match_kotlin_fallbacks() {
        assert_eq!("03:05", format_countdown_duration(185_000));
        assert_eq!("00:00", format_countdown_duration(-1));
        assert_eq!("04", slot_label(4));
        assert_eq!("\u{4efb}\u{52a1} 07", timer_slot_display_title("   ", 7));
        assert_eq!("Deep Work", archived_task_display_title(" Deep Work ", 7));
        assert_eq!(
            "\u{6700}\u{8fd1}\u{7ed3}\u{675f} 09:30",
            latest_finish_badge_label("09:30")
        );
        assert_eq!(
            "\u{4e0b}\u{65b9}\u{6709} 3 \u{4e2a}\u{53ef}\u{6062}\u{590d}\u{7684}\u{5f52}\u{6863}\u{4efb}\u{52a1}\u{3002}",
            history_week_summary_note(3)
        );
        assert_eq!(
            "focus\nplan\n",
            build_history_search_haystack(&[
                " Focus ".to_string(),
                " ".to_string(),
                "PLAN".to_string(),
            ])
        );
        assert!(has_finance_row_draft_content(" ", 1, None));
        assert!(has_finance_row_draft_content(" ", 0, Some(" note ")));
        assert!(!has_finance_row_draft_content(" ", 0, Some(" ")));
    }

    #[test]
    fn note_sort_indices_preserve_model_ordering_rules() {
        let pinned = [JNI_FALSE, JNI_TRUE, JNI_FALSE, JNI_FALSE];
        let created = [30, 10, 20, 40];
        let updated = [60, 50, 70, 10];
        let titles = vec![
            "zeta".to_string(),
            "Pinned".to_string(),
            "alpha".to_string(),
            "beta".to_string(),
        ];

        assert_eq!(
            Some(vec![1, 2, 0, 3]),
            sort_note_indices(0, &pinned, &created, &updated, &titles)
        );
        assert_eq!(
            Some(vec![1, 2, 0, 3]),
            sort_note_indices(2, &pinned, &created, &updated, &titles)
        );
        assert_eq!(
            Some(vec![1, 2, 3, 0]),
            sort_note_indices(3, &pinned, &created, &updated, &titles)
        );
        assert_eq!(
            None,
            sort_note_indices(0, &pinned, &created[..3], &updated, &titles)
        );
    }

    #[test]
    fn trashed_note_sort_indices_use_deleted_then_updated_descending() {
        assert_eq!(
            Some(vec![2, 1, 0]),
            sort_trashed_note_indices(&[10, 30, 30], &[99, 1, 50])
        );
        assert_eq!(None, sort_trashed_note_indices(&[10], &[10, 20]));
    }

    #[test]
    fn timer_insert_stabilization_matches_hold_policy() {
        let first = stabilize_timer_slot_insert_index(0, 1, None, 0, 1_000, 80);
        let second = stabilize_timer_slot_insert_index(
            first.insert_index,
            1,
            first.pending_insert_index,
            first.pending_insert_index_since_uptime_ms,
            1_079,
            80,
        );
        let committed = stabilize_timer_slot_insert_index(
            second.insert_index,
            1,
            second.pending_insert_index,
            second.pending_insert_index_since_uptime_ms,
            1_080,
            80,
        );

        assert_eq!(0, first.insert_index);
        assert_eq!(Some(1), first.pending_insert_index);
        assert_eq!(0, second.insert_index);
        assert_eq!(1, committed.insert_index);
        assert_eq!(None, committed.pending_insert_index);
    }

    #[test]
    fn timer_insert_index_uses_activation_bounds() {
        let candidates = [
            TimerSlotInsertCandidate {
                slot_index: 0,
                bounds: rect(0.0, 0.0, 100.0, 100.0),
            },
            TimerSlotInsertCandidate {
                slot_index: 1,
                bounds: rect(120.0, 0.0, 220.0, 100.0),
            },
        ];

        assert_eq!(
            0,
            super::resolve_timer_slot_insert_index(
                &candidates,
                0,
                point(126.0, 50.0),
                false,
                10_000,
                0,
                12.0,
            )
        );
        assert_eq!(
            1,
            super::resolve_timer_slot_insert_index(
                &candidates,
                0,
                point(136.0, 50.0),
                false,
                10_080,
                0,
                12.0,
            )
        );
    }

    #[test]
    fn notification_policy_helpers_match_boolean_rules() {
        assert!(should_sync_timer_live_update(true, true));
        assert!(!should_sync_timer_live_update(true, false));
        assert!(should_run_timer_live_update_refresh(true, true));
        assert!(should_request_timer_notification_permission(
            false, true, true, true
        ));
        assert!(!should_request_timer_notification_permission(
            false, true, true, false
        ));
    }

    #[test]
    fn action_request_code_matches_kotlin_math() {
        assert_eq!(125_153_017, compute_action_request_code(4_201, &[1, 2, 3]));
        assert_eq!(250_275_217, compute_action_request_code(8_401, &[1, 2, 3]));
    }

    #[test]
    fn history_query_matching_is_case_insensitive() {
        let keywords = normalize_query("FOCUS plan");

        assert!(history_query_matches("Focus PLAN archive", &keywords));
        assert!(!history_query_matches("Focus archive", &keywords));
    }

    #[test]
    fn knowledge_ranking_prefers_matching_titles_and_bodies() {
        let titles = vec!["Rust 项目说明".to_string(), "会议记录".to_string()];
        let bodies = vec![
            "知识库检索和 AI 回答都在本地先收窄来源。".to_string(),
            "只记录排期。".to_string(),
        ];
        let folders = vec!["知识库".to_string(), "杂项".to_string()];

        let indices = rank_knowledge_source_indices("知识库 AI", &titles, &bodies, &folders);

        assert_eq!(Some(&0), indices.first());
        assert!(!indices.contains(&1));
    }

    #[test]
    fn micro_break_target_stays_within_expected_window() {
        let first = compute_micro_break_target_millis(1, 0);
        let later = compute_micro_break_target_millis(1, 5);

        assert!((180_000..=300_000).contains(&first));
        assert!((180_000..=300_000).contains(&later));
        assert_ne!(first, later);
    }

    #[test]
    fn timer_drop_target_uses_preview_translated_bounds() {
        let result = resolve_timer_slot_drop_target_id(
            &[
                TimerSlotDropTargetCandidate {
                    slot_id: 1,
                    bounds: rect(0.0, 0.0, 100.0, 100.0),
                },
                TimerSlotDropTargetCandidate {
                    slot_id: 2,
                    bounds: rect(0.0, 0.0, 100.0, 100.0)
                        .translated_by(point(120.0, 0.0))
                        .translated_by(point(-120.0, 0.0)),
                },
                TimerSlotDropTargetCandidate {
                    slot_id: 3,
                    bounds: rect(240.0, 0.0, 340.0, 100.0),
                },
            ],
            rect(10.0, 0.0, 110.0, 100.0),
            point(56.0, 50.0),
            point(60.0, 50.0),
            1,
        );

        assert_eq!(Some(2), result);
    }

    #[test]
    fn timer_drop_target_prefers_next_tile_in_same_column() {
        let result = resolve_timer_slot_drop_target_id(
            &[
                TimerSlotDropTargetCandidate {
                    slot_id: 1,
                    bounds: rect(0.0, 0.0, 100.0, 100.0),
                },
                TimerSlotDropTargetCandidate {
                    slot_id: 3,
                    bounds: rect(120.0, 0.0, 220.0, 100.0),
                },
                TimerSlotDropTargetCandidate {
                    slot_id: 4,
                    bounds: rect(120.0, 120.0, 220.0, 220.0),
                },
                TimerSlotDropTargetCandidate {
                    slot_id: 5,
                    bounds: rect(0.0, 120.0, 100.0, 220.0),
                },
            ],
            rect(126.0, 14.0, 226.0, 114.0),
            point(170.0, 112.0),
            point(176.0, 64.0),
            9,
        );

        assert_eq!(Some(4), result);
    }

    #[test]
    fn timer_drop_target_prefers_overlap_before_column_heuristic() {
        let result = resolve_timer_slot_drop_target_id(
            &[
                TimerSlotDropTargetCandidate {
                    slot_id: 1,
                    bounds: rect(0.0, 0.0, 100.0, 100.0),
                },
                TimerSlotDropTargetCandidate {
                    slot_id: 3,
                    bounds: rect(120.0, 0.0, 220.0, 100.0),
                },
                TimerSlotDropTargetCandidate {
                    slot_id: 4,
                    bounds: rect(120.0, 120.0, 220.0, 220.0),
                },
            ],
            rect(126.0, 20.0, 226.0, 120.0),
            point(170.0, 112.0),
            point(176.0, 70.0),
            9,
        );

        assert_eq!(Some(3), result);
    }

    #[test]
    fn xiaomi_focus_payload_omits_island_block() {
        let payload = build_xiaomi_payload(&sample_state(), false);
        let root: Value = serde_json::from_str(&payload).expect("valid payload json");
        let param_v2 = &root["param_v2"];

        assert_eq!("timer", param_v2["business"].as_str().unwrap());
        assert_eq!(true, param_v2["enableFloat"].as_bool().unwrap());
        assert_eq!(
            "miui.focus.pic_timer",
            param_v2["tickerPic"].as_str().unwrap()
        );
        assert!(param_v2.get("param_island").is_none());
    }

    #[test]
    fn xiaomi_island_payload_contains_big_and_small_areas() {
        let payload = build_xiaomi_payload(&sample_state(), true);
        let root: Value = serde_json::from_str(&payload).expect("valid payload json");
        let param_v2 = &root["param_v2"];
        let param_island = &param_v2["param_island"];

        assert_eq!(true, param_v2["enableFloat"].as_bool().unwrap());
        assert_eq!(true, param_v2["islandFirstFloat"].as_bool().unwrap());
        assert_eq!(
            1,
            param_island["bigIslandArea"]["imageTextInfoLeft"]["type"]
                .as_i64()
                .unwrap()
        );
        assert!(param_island["bigIslandArea"]["imageTextInfoRight"].is_null());
        assert_eq!(
            1,
            param_island["bigIslandArea"]["picInfo"]["type"]
                .as_i64()
                .unwrap()
        );
        assert_eq!(
            "miui.focus.pic_timer",
            param_island["smallIslandArea"]["picInfo"]["pic"]
                .as_str()
                .unwrap()
        );
    }

    #[test]
    fn xiaomi_island_payload_keeps_timer_visible_in_standard_text_template() {
        let mut state = sample_state();
        state.title = "LongTaskTitleForIsland timer is running".to_string();
        state.big_text = "LongTaskTitleForIsland 00:42".to_string();
        state.primary_title = "LongTaskTitleForIsland".to_string();

        let payload = build_xiaomi_payload(&state, true);
        let root: Value = serde_json::from_str(&payload).expect("valid payload json");
        let param_island = &root["param_v2"]["param_island"];
        let big_text_info = &param_island["bigIslandArea"]["imageTextInfoLeft"]["textInfo"];

        assert_eq!("00:42", big_text_info["title"].as_str().unwrap());
        assert_eq!("LongTaskTi", big_text_info["content"].as_str().unwrap());
        assert_eq!(true, big_text_info["narrowFont"].as_bool().unwrap());
        assert_eq!(false, big_text_info["useHighLight"].as_bool().unwrap());
    }

    fn sample_state() -> XiaomiPayloadState {
        XiaomiPayloadState {
            title: "Writing timer is running".to_string(),
            text: "Elapsed 00:42".to_string(),
            big_text: "Writing 00:42 | Review 00:18".to_string(),
            sub_text: "Return to the app to keep editing this timer".to_string(),
            primary_title: "Writing".to_string(),
            primary_elapsed: "00:42".to_string(),
            share_content: "Writing 已累计 00:42".to_string(),
            running_count: 1,
        }
    }

    fn point(x: f32, y: f32) -> NativePoint {
        NativePoint { x, y }
    }

    fn rect(left: f32, top: f32, right: f32, bottom: f32) -> NativeRect {
        NativeRect {
            left,
            top,
            right,
            bottom,
        }
    }
}
