// v1.0.3 Windows - Preserve category label fallbacks without copying ledger data.
#[test]
fn finance_overview_cached_labels_preserve_editor_category_rules() {
    for profile in [
        json!({}),
        json!({"settings":null}),
        json!({"settings":{"expenseCategories":[
            {"bucket":"FOOD","label":"自定义饮食"},
            {"bucket":"BTC","label":"   "},
            {"bucket":"LIVING","label":" 居住 "},
            {"label":"missing bucket"},
            {"bucket":"OTHER","label":"其他计划"},
            {"bucket":"FOOD","label":"duplicate must not override"}
        ]}}),
    ] {
        let options = finance_expense_options(&profile);
        for code in -1..=6 {
            let expected = usize::try_from(code)
                .ok()
                .and_then(|index| options.get(index))
                .map(|(_, label)| label.clone());
            assert_eq!(
                expected,
                finance_overview_expense_label(profile.get("settings"), code)
            );
        }
    }
}

#[test]
#[ignore = "Explicit cached trend label benchmark; no personal data is read"]
fn finance_overview_cached_label_benchmark() {
    fn measure(mut operation: impl FnMut()) -> Value {
        operation();
        let mut nanos = Vec::with_capacity(31);
        for _ in 0..31 {
            let start = Instant::now();
            operation();
            nanos.push(start.elapsed().as_nanos() as u64);
        }
        nanos.sort_unstable();
        json!({"medianMicros":nanos[15] as f64 / 1000.0,"p95Micros":nanos[29] as f64 / 1000.0})
    }
    let mut results = Vec::new();
    for records in [168, 2_688] {
        let profile = json!({
            "settings":{"expenseCategories":[{"bucket":"FOOD","label":"日常饮食"}]},
            "dailyLedgers":(0..records).map(|index| (format!("synthetic-{index}"), json!({
                "incomes":[{"id":format!("income-{index}"),"amount":300,"note":"n".repeat(512)}],
                "expenses":[{"id":format!("expense-{index}"),"amount":100,"note":"n".repeat(512)}]
            }))).collect::<serde_json::Map<_,_>>()
        });
        let cached_label = finance_overview_expense_label(profile.get("settings"), 1);
        assert_eq!(Some("日常饮食"), cached_label.as_deref());
        let legacy = measure(|| {
            let copied_profile = profile.clone();
            let options = finance_expense_options(std::hint::black_box(&copied_profile));
            std::hint::black_box(&options[1].1);
        });
        let cached = measure(|| {
            std::hint::black_box(cached_label.as_deref());
        });
        results.push(json!({"records":records,"profileBytes":profile.to_string().len(),"legacyProfileCloneAndLabel":legacy,"cachedLabelBorrow":cached}));
    }
    let report = json!({"version":WINDOWS_CLIENT_VERSION,"scope":"synthetic same-binary trend label preparation per frame; excludes rendering","results":results});
    println!("FINANCE_LABEL_PERFORMANCE {report}");
    if let Some(path) = std::env::var_os("DESKTOP_FINANCE_LABEL_PERFORMANCE_OUTPUT") {
        fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}
