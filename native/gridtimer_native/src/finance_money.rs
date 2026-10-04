//! Exact RMB input and calculation projection. Persisted `amount` remains yuan.
//! Optional minor-unit values preserve cents without changing legacy wire units.

use serde_json::Value;

pub const INPUT_MAX_WHOLE: i64 = 999_999_999;
pub const INPUT_MAX_MINOR: i64 = 99_999_999_999;
pub const INPUT_HINT: &str = "金额可保留两位小数，单笔最多 999,999,999.99 元";
pub const LEGACY_MONEY_FIELDS: [&str; 7] = [
    "activeIncomeMonthly",
    "assetIncomeMonthly",
    "livingExpenseMonthly",
    "liabilityPaymentMonthly",
    "cashReserve",
    "productiveAssetValue",
    "liabilityBalance",
];

/// Empty and incomplete decimal drafts are valid while typing. No float is used.
pub fn parse_minor_draft(value: &str, max_whole_digits: usize, max_whole: i64) -> Option<i64> {
    let value = value.trim();
    if value.is_empty() {
        return Some(0);
    }
    if max_whole_digits == 0 || max_whole < 0 {
        return None;
    }
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.len() > max_whole_digits
        || fraction.len() > 2
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let whole = if whole.is_empty() {
        0
    } else {
        whole.parse::<i64>().ok()?
    };
    if whole > max_whole {
        return None;
    }
    let fraction = if fraction.is_empty() {
        0
    } else {
        fraction
            .parse::<i64>()
            .ok()?
            .checked_mul(if fraction.len() == 1 { 10 } else { 1 })?
    };
    whole.checked_mul(100)?.checked_add(fraction)
}

/// A supplied precision value must agree with the original whole-yuan field.
/// Inconsistent, non-integer and overflowing values are errors, never truncation.
pub fn validated_minor(whole: i64, minor: Option<i64>) -> Option<i64> {
    match minor {
        Some(minor) if minor >= 0 && minor / 100 == whole => Some(minor),
        Some(_) => None,
        None => whole.checked_mul(100),
    }
}

pub fn format_minor(amount: i64, prefix: &str, signed: bool, grouped: bool) -> String {
    let absolute = amount.unsigned_abs();
    let whole = absolute / 100;
    let fraction = absolute % 100;
    let whole_text = whole.to_string();
    let mut grouped_whole = String::with_capacity(whole_text.len() + whole_text.len() / 3);
    for (index, byte) in whole_text.bytes().enumerate() {
        if grouped && index > 0 && (whole_text.len() - index) % 3 == 0 {
            grouped_whole.push(',');
        }
        grouped_whole.push(byte as char);
    }
    let sign = if amount < 0 {
        "-"
    } else if signed && amount > 0 {
        "+"
    } else {
        ""
    };
    format!("{sign}{prefix}{grouped_whole}.{fraction:02}")
}

fn project_value(whole: &Value, minor: Option<&Value>, legacy: bool) -> Option<i64> {
    let whole = if whole.is_null() { 0 } else { whole.as_i64()? };
    let minor = match minor {
        None | Some(Value::Null) => None,
        Some(value) => Some(value.as_i64()?),
    };
    if legacy
        && minor
            .is_some_and(|value| value < 0 && whole < 0 && whole.checked_mul(100) == Some(value))
    {
        minor
    } else {
        validated_minor(whole, minor)
    }
}

/// Produce an ephemeral calculation view in cents. The original JSON is untouched.
/// Calculation views must never be persisted or synchronized as a finance profile.
pub fn project_profile_json(raw: &str) -> Option<String> {
    let mut root = serde_json::from_str::<Value>(raw).ok()?;
    let profile = root.as_object_mut()?;
    if profile.contains_key("calculationMoneyUnit") {
        return None;
    }
    let legacy = match profile.get("legacyAmountMinor") {
        None | Some(Value::Null) => serde_json::Map::new(),
        Some(Value::Object(value)) => value.clone(),
        Some(_) => return None,
    };
    if legacy
        .keys()
        .any(|key| !LEGACY_MONEY_FIELDS.contains(&key.as_str()))
    {
        return None;
    }
    for field in LEGACY_MONEY_FIELDS {
        let whole = profile.get(field).unwrap_or(&Value::Null);
        let minor = project_value(whole, legacy.get(field), true)?;
        profile.insert(field.to_string(), Value::from(minor));
    }
    profile.remove("legacyAmountMinor");
    for (container_name, lanes) in [
        ("dailyLedgers", ["incomes", "expenses"]),
        ("monthlySnapshots", ["assets", "liabilities"]),
    ] {
        let Some(containers) = profile.get_mut(container_name) else {
            continue;
        };
        for container in containers.as_object_mut()?.values_mut() {
            let container = container.as_object_mut()?;
            for lane in lanes {
                let Some(rows) = container.get_mut(lane) else {
                    continue;
                };
                for row in rows.as_array_mut()? {
                    let row = row.as_object_mut()?;
                    let minor = project_value(
                        row.get("amount").unwrap_or(&Value::Null),
                        row.get("amountMinor"),
                        false,
                    )?;
                    row.insert("amount".into(), Value::from(minor));
                    row.remove("amountMinor");
                }
            }
        }
    }
    profile.insert("calculationMoneyUnit".into(), Value::from("rmb-cent-v1"));
    serde_json::to_string(&root).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_decimal_input_preserves_cents_and_rejects_excess_precision() {
        for (text, expected) in [
            ("58", 5800),
            ("58.", 5800),
            ("58.1", 5810),
            ("58.12", 5812),
            (".01", 1),
            ("0.00", 0),
            ("", 0),
            (".", 0),
            ("999999999.99", INPUT_MAX_MINOR),
        ] {
            assert_eq!(
                Some(expected),
                parse_minor_draft(text, 9, INPUT_MAX_WHOLE),
                "{text}"
            );
        }
        for text in [
            "58.123",
            "58..1",
            "58,12",
            "-1",
            "+1",
            "1e2",
            "1000000000",
            "９８",
            "NaN",
        ] {
            assert_eq!(None, parse_minor_draft(text, 9, INPUT_MAX_WHOLE), "{text}");
        }
        assert_eq!("¥58.12", format_minor(5812, "¥", false, true));
        assert_eq!("¥0.01", format_minor(1, "¥", false, true));
        assert_eq!("-¥1.01", format_minor(-101, "¥", true, true));
        assert_eq!(
            "+¥999,999,999.99",
            format_minor(INPUT_MAX_MINOR, "¥", true, true)
        );
    }

    #[test]
    fn precision_projection_keeps_old_units_and_rejects_inconsistent_minor_values() {
        let raw = r#"{"cashReserve":58,"legacyAmountMinor":{"cashReserve":5812},"dailyLedgers":{"2026-10-02":{"incomes":[{"amount":12}],"expenses":[{"amount":58,"amountMinor":5812}]}},"monthlySnapshots":{"2026-10":{"assets":[{"amount":0,"amountMinor":1}],"liabilities":[{"amount":1,"amountMinor":123}]}}}"#;
        let projected: Value = serde_json::from_str(&project_profile_json(raw).unwrap()).unwrap();
        assert_eq!(5812, projected["cashReserve"]);
        assert_eq!(
            1200,
            projected["dailyLedgers"]["2026-10-02"]["incomes"][0]["amount"]
        );
        assert_eq!(
            5812,
            projected["dailyLedgers"]["2026-10-02"]["expenses"][0]["amount"]
        );
        assert_eq!(
            1,
            projected["monthlySnapshots"]["2026-10"]["assets"][0]["amount"]
        );
        assert_eq!(
            123,
            projected["monthlySnapshots"]["2026-10"]["liabilities"][0]["amount"]
        );
        assert_eq!(
            58,
            serde_json::from_str::<Value>(raw).unwrap()["cashReserve"]
        );
        assert!(project_profile_json(&projected.to_string()).is_none());
        for bad in [
            r#"{"amount":58,"amountMinor":5912}"#,
            r#"{"amount":58,"amountMinor":58.12}"#,
            r#"{"amount":58,"amountMinor":"5812"}"#,
        ] {
            let raw = format!(r#"{{"dailyLedgers":{{"2026-10-02":{{"expenses":[{bad}]}}}}}}"#);
            assert!(project_profile_json(&raw).is_none());
        }
        assert_eq!(None, validated_minor(i64::MAX, None));
    }
}
