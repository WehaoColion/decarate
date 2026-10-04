//! Persistence and peer boundaries for exact RMB fractions.
use serde_json::{Map, Value};

const LEGACY_FIELDS: [&str; 7] = [
    "activeIncomeMonthly",
    "assetIncomeMonthly",
    "livingExpenseMonthly",
    "liabilityPaymentMonthly",
    "cashReserve",
    "productiveAssetValue",
    "liabilityBalance",
];

pub(crate) fn validate_profile(profile: &Value) -> Result<(), String> {
    let object = profile.as_object().ok_or("invalid finance profile")?;
    if object.contains_key("calculationMoneyUnit") {
        return Err("calculation-only money view cannot be saved".into());
    }
    if let Some(value) = object.get("legacyAmountMinor").filter(|v| !v.is_null()) {
        let minor = value.as_object().ok_or("invalid legacy money precision")?;
        for (field, value) in minor {
            if !LEGACY_FIELDS.contains(&field.as_str()) {
                return Err("unknown legacy money precision field".into());
            }
            validate_minor(object.get(field), value, true)?;
        }
    }
    for (containers, lanes) in [
        ("dailyLedgers", ["incomes", "expenses"]),
        ("monthlySnapshots", ["assets", "liabilities"]),
    ] {
        let Some(periods) = object.get(containers).and_then(Value::as_object) else {
            continue;
        };
        for period in periods.values() {
            for lane in lanes {
                let Some(rows) = period.get(lane).and_then(Value::as_array) else {
                    continue;
                };
                for row in rows {
                    if let Some(minor) = row.get("amountMinor").filter(|v| !v.is_null()) {
                        validate_minor(row.get("amount"), minor, false)?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_minor(whole: Option<&Value>, minor: &Value, legacy: bool) -> Result<(), String> {
    let whole = whole
        .and_then(Value::as_i64)
        .ok_or("missing whole-yuan money value")?;
    let minor = minor
        .as_i64()
        .ok_or("minor-unit money value must be an integer")?;
    let negative_unknown = legacy && whole < 0 && whole.checked_mul(100) == Some(minor);
    if minor / 100 != whole || (minor < 0 && !negative_unknown) {
        return Err("money precision does not agree with the whole-yuan value".into());
    }
    Ok(())
}

pub(crate) fn validate_app_data(value: &Value) -> Result<(), String> {
    match value.get("financeProfile") {
        Some(profile) => validate_profile(profile),
        None => Ok(()),
    }
}

/// Unchanged legacy snapshots can omit new optional fields. Restore only the
/// exact value from the same record and revision; newer legacy edits fail closed.
pub(crate) fn reconcile_peer(account: &Value, incoming: &mut Value) -> Result<(), String> {
    reconcile(account, incoming, false)
}

/// Direct saves replace the profile rather than merging row revisions. An old
/// row must therefore still recover its omitted cents or reject the write.
pub(crate) fn reconcile_update(account: &Value, profile: &mut Value) -> Result<(), String> {
    let mut incoming = account.clone();
    incoming["financeProfile"] = profile.clone();
    reconcile(account, &mut incoming, true)?;
    *profile = incoming["financeProfile"].clone();
    Ok(())
}

fn reconcile(account: &Value, incoming: &mut Value, direct_save: bool) -> Result<(), String> {
    validate_app_data(account)?;
    validate_app_data(incoming)?;
    let account_revision = revision(account, "financeProfileUpdatedAtEpochMillis");
    let incoming_revision = revision(incoming, "financeProfileUpdatedAtEpochMillis");
    let incoming_period_revisions = serde_json::json!({
        "financeDayLedgerRevisions": incoming.get("financeDayLedgerRevisions"),
        "financeMonthSnapshotRevisions": incoming.get("financeMonthSnapshotRevisions")
    });
    let Some(before) = account.get("financeProfile").and_then(Value::as_object) else {
        return Ok(());
    };
    let Some(after) = incoming
        .get_mut("financeProfile")
        .and_then(Value::as_object_mut)
    else {
        return Ok(());
    };
    if let Some(minors) = before.get("legacyAmountMinor").and_then(Value::as_object) {
        for (field, minor) in minors {
            if !has_fraction(minor) {
                continue;
            }
            if !direct_save && incoming_revision < account_revision {
                continue;
            }
            let supplied = after
                .get("legacyAmountMinor")
                .and_then(|v| v.get(field))
                .filter(|v| !v.is_null());
            if supplied.is_some() {
                continue;
            }
            if incoming_revision > account_revision || before.get(field) != after.get(field) {
                return Err(
                    "update this client before editing financial amounts with cents".into(),
                );
            }
            let values = after
                .entry("legacyAmountMinor".to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            if values.is_null() {
                *values = Value::Object(Map::new());
            }
            values
                .as_object_mut()
                .ok_or("invalid legacy money precision")?
                .insert(field.clone(), minor.clone());
        }
    }
    for (containers, lanes) in [
        ("dailyLedgers", ["incomes", "expenses"]),
        ("monthlySnapshots", ["assets", "liabilities"]),
    ] {
        let Some(periods) = before.get(containers).and_then(Value::as_object) else {
            continue;
        };
        for (period_key, period) in periods {
            for lane in lanes {
                let Some(rows) = period.get(lane).and_then(Value::as_array) else {
                    continue;
                };
                for row in rows {
                    let Some(minor) = row.get("amountMinor").filter(|v| has_fraction(v)) else {
                        continue;
                    };
                    let Some(id) = row
                        .get("id")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|id| !id.is_empty())
                    else {
                        return Err("precise financial rows need a stable record id".into());
                    };
                    let Some(incoming_rows) = after
                        .get_mut(containers)
                        .and_then(|v| v.get_mut(period_key))
                        .and_then(|v| v.get_mut(lane))
                        .and_then(Value::as_array_mut)
                    else {
                        continue;
                    };
                    for next in incoming_rows.iter_mut().filter(|next| {
                        next.get("id").and_then(Value::as_str).map(str::trim) == Some(id)
                    }) {
                        if next.get("amountMinor").is_some_and(|v| !v.is_null()) {
                            continue;
                        }
                        let row_revision = entry_revision(row, "updatedAtEpochMillis")
                            .max(entry_revision(row, "deletedAtEpochMillis"));
                        let next_revision = entry_revision(next, "updatedAtEpochMillis")
                            .max(entry_revision(next, "deletedAtEpochMillis"));
                        let revision_map = if containers == "dailyLedgers" {
                            "financeDayLedgerRevisions"
                        } else {
                            "financeMonthSnapshotRevisions"
                        };
                        let account_period_revision = account
                            .get(revision_map)
                            .and_then(|v| v.get(period_key))
                            .and_then(Value::as_i64)
                            .filter(|revision| *revision > 0)
                            .unwrap_or(account_revision);
                        let incoming_period_revision = incoming_period_revisions
                            .get(revision_map)
                            .and_then(|v| v.get(period_key))
                            .and_then(Value::as_i64)
                            .filter(|revision| *revision > 0)
                            .unwrap_or(incoming_revision);
                        if !direct_save
                            && (next_revision < row_revision
                                || (row_revision == 0
                                    && next_revision == 0
                                    && incoming_period_revision < account_period_revision))
                        {
                            continue;
                        }
                        let deletion = entry_revision(next, "deletedAtEpochMillis") > 0;
                        if row.get("amount") != next.get("amount")
                            || (!deletion
                                && (next_revision > row_revision
                                    || (row_revision == 0
                                        && incoming_period_revision > account_period_revision)))
                        {
                            return Err(
                                "update this client before editing financial amounts with cents"
                                    .into(),
                            );
                        }
                        next.as_object_mut()
                            .ok_or("invalid finance row")?
                            .insert("amountMinor".into(), minor.clone());
                    }
                }
            }
        }
    }
    validate_app_data(incoming)
}

fn has_fraction(value: &Value) -> bool {
    value.as_i64().is_some_and(|v| v % 100 != 0)
}
fn revision(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(0).max(0)
}

// Keep the peer row revision interpretation identical to the sync merge.
fn entry_revision(value: &Value, key: &str) -> i64 {
    value
        .get(key)
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_u64().map(|v| v.min(i64::MAX as u64) as i64))
                .or_else(|| v.as_str().and_then(|v| v.trim().parse().ok()))
        })
        .unwrap_or(0)
        .max(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snapshot() -> Value {
        json!({"financeProfileUpdatedAtEpochMillis":10,"financeProfile":{
        "cashReserve":1,"legacyAmountMinor":{"cashReserve":101},"dailyLedgers":{"2026-10-02":{
        "expenses":[{"id":"e1","amount":58,"amountMinor":5899,"updatedAtEpochMillis":10}]}}}})
    }

    #[test]
    fn unchanged_legacy_peer_keeps_cents_but_newer_same_whole_edit_is_rejected() {
        let account = snapshot();
        let mut incoming = account.clone();
        incoming["financeProfile"]
            .as_object_mut()
            .unwrap()
            .remove("legacyAmountMinor");
        incoming["financeProfile"]["dailyLedgers"]["2026-10-02"]["expenses"][0]
            .as_object_mut()
            .unwrap()
            .remove("amountMinor");
        let mut newer = incoming.clone();
        newer["financeProfile"]["dailyLedgers"]["2026-10-02"]["expenses"][0]
            ["updatedAtEpochMillis"] = json!(11);
        assert!(reconcile_peer(&account, &mut incoming).is_ok());
        assert_eq!(
            incoming["financeProfile"]["dailyLedgers"]["2026-10-02"]["expenses"][0]["amountMinor"],
            5899
        );
        assert!(reconcile_peer(&account, &mut newer).is_err());
        let mut stale = account.clone();
        stale["financeProfileUpdatedAtEpochMillis"] = json!(9);
        stale["financeProfile"]
            .as_object_mut()
            .unwrap()
            .remove("legacyAmountMinor");
        stale["financeProfile"]["cashReserve"] = json!(0);
        let stale_row = &mut stale["financeProfile"]["dailyLedgers"]["2026-10-02"]["expenses"][0];
        stale_row.as_object_mut().unwrap().remove("amountMinor");
        stale_row["amount"] = json!(57);
        stale_row["updatedAtEpochMillis"] = json!(9);
        assert!(reconcile_peer(&account, &mut stale).is_ok());
        let mut stale_profile = stale["financeProfile"].clone();
        assert!(reconcile_update(&account, &mut stale_profile).is_err());
        stale_profile["cashReserve"] = json!(1);
        stale_profile["dailyLedgers"]["2026-10-02"]["expenses"][0]["amount"] = json!(58);
        assert!(reconcile_update(&account, &mut stale_profile).is_ok());
        assert_eq!(
            stale_profile["dailyLedgers"]["2026-10-02"]["expenses"][0]["amountMinor"],
            5899
        );
        let mut zero_rows = account.clone();
        zero_rows["financeDayLedgerRevisions"] = json!({"2026-10-02":10});
        zero_rows["financeProfile"]["dailyLedgers"]["2026-10-02"]["expenses"][0]
            ["updatedAtEpochMillis"] = json!(0);
        let mut zero_peer = zero_rows.clone();
        zero_peer["financeProfileUpdatedAtEpochMillis"] = json!(11);
        zero_peer["financeDayLedgerRevisions"]["2026-10-02"] = json!(0);
        zero_peer["financeProfile"]["dailyLedgers"]["2026-10-02"]["expenses"][0]
            .as_object_mut()
            .unwrap()
            .remove("amountMinor");
        assert!(reconcile_peer(&zero_rows, &mut zero_peer).is_err());
        let mut trimmed = account.clone();
        let row = &mut trimmed["financeProfile"]["dailyLedgers"]["2026-10-02"]["expenses"][0];
        row.as_object_mut().unwrap().remove("amountMinor");
        row["id"] = json!(" e1 ");
        row["updatedAtEpochMillis"] = json!("11");
        assert!(reconcile_peer(&account, &mut trimmed).is_err());
        let mut duplicate = account.clone();
        let mut incomplete =
            duplicate["financeProfile"]["dailyLedgers"]["2026-10-02"]["expenses"][0].clone();
        incomplete.as_object_mut().unwrap().remove("amountMinor");
        incomplete["updatedAtEpochMillis"] = json!(11);
        duplicate["financeProfile"]["dailyLedgers"]["2026-10-02"]["expenses"]
            .as_array_mut()
            .unwrap()
            .push(incomplete);
        assert!(reconcile_peer(&account, &mut duplicate).is_err());
    }

    #[test]
    fn calculation_views_and_disagreeing_minor_values_cannot_enter_storage() {
        let mut profile = snapshot()["financeProfile"].clone();
        assert!(validate_profile(&profile).is_ok());
        profile["calculationMoneyUnit"] = json!("rmb-cent-v1");
        assert!(validate_profile(&profile).is_err());
        profile
            .as_object_mut()
            .unwrap()
            .remove("calculationMoneyUnit");
        profile["dailyLedgers"]["2026-10-02"]["expenses"][0]["amountMinor"] = json!(5900);
        assert!(validate_profile(&profile).is_err());
        assert!(validate_profile(&json!({"cashReserve":-1})).is_ok());
    }

    #[test]
    fn legacy_deletion_retains_precise_history_and_new_client_can_edit_cents() {
        let account = snapshot();
        let mut deleted = account.clone();
        let row = &mut deleted["financeProfile"]["dailyLedgers"]["2026-10-02"]["expenses"][0];
        row.as_object_mut().unwrap().remove("amountMinor");
        row["deletedAtEpochMillis"] = json!(11);
        assert!(reconcile_peer(&account, &mut deleted).is_ok());
        assert_eq!(
            deleted["financeProfile"]["dailyLedgers"]["2026-10-02"]["expenses"][0]["amountMinor"],
            5899
        );
        let mut edited = account.clone();
        edited["financeProfile"]["dailyLedgers"]["2026-10-02"]["expenses"][0]["amountMinor"] =
            json!(5801);
        edited["financeProfile"]["dailyLedgers"]["2026-10-02"]["expenses"][0]
            ["updatedAtEpochMillis"] = json!(11);
        assert!(reconcile_peer(&account, &mut edited).is_ok());
    }

    #[test]
    fn app_data_save_and_real_sync_merge_preserve_precision_and_block_loss() {
        let base = crate::app_data::default_app_data_json(100);
        let profile = snapshot()["financeProfile"].to_string();
        let account =
            crate::app_data::update_finance_profile_app_data_json(&base, &profile, 100).unwrap();
        let sanitized = crate::app_data::sanitize_app_data_json(&account, 100).unwrap();
        let canonical: Value = serde_json::from_str(&sanitized).unwrap();
        assert_eq!(
            canonical["financeProfile"]["dailyLedgers"]["2026-10-02"]["expenses"][0]["amountMinor"],
            5899
        );
        let mut legacy = canonical.clone();
        legacy["financeProfile"]
            .as_object_mut()
            .unwrap()
            .remove("legacyAmountMinor");
        legacy["financeProfile"]["dailyLedgers"]["2026-10-02"]["expenses"][0]
            .as_object_mut()
            .unwrap()
            .remove("amountMinor");
        let saved_legacy = crate::app_data::update_finance_profile_app_data_json(
            &account,
            &legacy["financeProfile"].to_string(),
            101,
        )
        .unwrap();
        let saved_legacy: Value = serde_json::from_str(&saved_legacy).unwrap();
        assert_eq!(
            saved_legacy["financeProfile"]["dailyLedgers"]["2026-10-02"]["expenses"][0]
                ["amountMinor"],
            5899
        );
        let mut lossy_profile = legacy["financeProfile"].clone();
        lossy_profile["dailyLedgers"]["2026-10-02"]["expenses"][0]["amount"] = json!(57);
        assert!(crate::app_data::update_finance_profile_app_data_json(
            &account,
            &lossy_profile.to_string(),
            101,
        )
        .is_none());
        let merged =
            crate::sync_core::merge_sync_app_data_json(&account, &legacy.to_string(), 100).unwrap();
        let merged: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(
            merged["financeProfile"]["dailyLedgers"]["2026-10-02"]["expenses"][0]["amountMinor"],
            5899
        );
        legacy["financeProfile"]["dailyLedgers"]["2026-10-02"]["expenses"][0]
            ["updatedAtEpochMillis"] = json!(101);
        assert!(
            crate::sync_core::merge_sync_app_data_json(&account, &legacy.to_string(), 101)
                .is_none()
        );
        // A legacy backup can omit IDs. Its canonical identity excludes the
        // optional precision field, so protect it before replacing the profile.
        let no_id_precise = json!({"dailyLedgers":{"2026-10-02":{"expenses":[{
            "name":"fee","bucket":"OTHER","amount":58,"amountMinor":5899,
            "updatedAtEpochMillis":10
        }]}}});
        let precise_account = crate::app_data::update_finance_profile_app_data_json(
            &base,
            &no_id_precise.to_string(),
            100,
        )
        .unwrap();
        let precise_saved: Value = serde_json::from_str(&precise_account).unwrap();
        let stable_id = precise_saved["financeProfile"]["dailyLedgers"]["2026-10-02"]["expenses"]
            [0]["id"]
            .as_str()
            .unwrap();
        assert!(!stable_id.is_empty());
        let mut no_id_legacy = no_id_precise.clone();
        no_id_legacy["dailyLedgers"]["2026-10-02"]["expenses"][0]
            .as_object_mut()
            .unwrap()
            .remove("amountMinor");
        let recovered = crate::app_data::update_finance_profile_app_data_json(
            &precise_account,
            &no_id_legacy.to_string(),
            101,
        )
        .unwrap();
        let recovered_value: Value = serde_json::from_str(&recovered).unwrap();
        let recovered_row =
            &recovered_value["financeProfile"]["dailyLedgers"]["2026-10-02"]["expenses"][0];
        assert_eq!(recovered_row["id"].as_str(), Some(stable_id));
        assert_eq!(recovered_row["amount"], 58);
        assert_eq!(recovered_row["amountMinor"], 5899);

        // Some(5800) is an explicit 58.00 edit, not omitted precision to recover.
        let mut explicit_whole = no_id_precise;
        explicit_whole["dailyLedgers"]["2026-10-02"]["expenses"][0]["amountMinor"] = json!(5800);
        let whole_saved = crate::app_data::update_finance_profile_app_data_json(
            &recovered,
            &explicit_whole.to_string(),
            102,
        )
        .unwrap();
        let whole_value: Value = serde_json::from_str(&whole_saved).unwrap();
        let whole_row = &whole_value["financeProfile"]["dailyLedgers"]["2026-10-02"]["expenses"][0];
        assert_eq!(whole_row["id"].as_str(), Some(stable_id));
        assert_eq!(whole_row["amount"], 58);
        assert_eq!(whole_row["amountMinor"], 5800);

        let projected = crate::finance_money::project_profile_json(&profile).unwrap();
        assert!(
            crate::app_data::update_finance_profile_app_data_json(&base, &projected, 100).is_none()
        );
        let mut calculation_snapshot: Value = serde_json::from_str(&account).unwrap();
        calculation_snapshot["financeProfile"] = serde_json::from_str(&projected).unwrap();
        assert!(
            crate::app_data::sanitize_app_data_json(&calculation_snapshot.to_string(), 100)
                .is_none()
        );
    }
}
