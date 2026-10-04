// v1.0.3 Windows - Guard shared overview parity and measure repeated decoding.
use super::*;
use serde_json::{json, Value};
use std::time::Instant;

fn legacy_overview(
    raw: &str,
    period: i32,
    y: i32,
    m: i32,
    d: i32,
) -> Option<DesktopFinanceOverviewValues> {
    Some(DesktopFinanceOverviewValues {
        report: build_detailed_finance_report_values(raw, period, y, m, d)?,
        trend: build_finance_trend_values(raw, period, y, m, d)?,
        health: build_finance_health_score_values(raw, period, y, m, d),
        risk_json: build_finance_risk_cockpit_json(raw, period, y, m, d)?,
        latest_month: latest_snapshot_month_key_up_to(raw, &format!("{y:04}-{m:02}")),
    })
}

fn detailed_fixture() -> Value {
    json!({
        "activeIncomeMonthly": 8000,
        "settings": {"expenseCategories": [
            {"bucket":"FOOD","label":"日常饮食","targetShareOfIncome":0.1},
            {"bucket":"LEARNING","label":" ","targetShareOfIncome":1.5},
            {"bucket":"OTHER","label":"自定义","targetShareOfIncome":-0.3}
        ]},
        "dailyLedgers": {
            "2024-02-29":{"incomes":[{"id":"leap","amount":600}],"confirmedAtEpochMillis":10},
            "2026-08-15":{"incomes":[{"id":"previous","amount":800}],"expenses":[{"bucket":"FOOD","amount":20}]},
            "2026-09-01":{"incomes":[{"id":"active","amount":1000},{"id":"deleted","amount":9999,"deletedAtEpochMillis":2}],"expenses":[{"bucket":"FOOD","amount":200}]},
            "2026-09-02":{"incomes":[{"id":" blank ","amount":-20}],"confirmedAtEpochMillis":10},
            "2026-09-03":{"incomes":[{"id":"tombstone","amount":700,"deletedAtEpochMillis":1}]},
            "2026-09-16":{"incomes":[{"id":"future","amount":8888}]},
            "invalid-key":{"incomes":[{"id":"invalid","amount":1234}]},
            "2026-02-30":{"incomes":[{"id":"invalid-date","amount":9000}]}
        },
        "monthlySnapshots": {
            "2026-08":{"assets":[{"id":"past-cash","kind":"CASH_RESERVE","amount":4000}]},
            "2026-09":{"assets":[{"id":"cash","kind":"CASH_RESERVE","amount":5000},{"id":"deleted-cash","amount":20000,"deletedAtEpochMillis":2}],"liabilities":[{"id":"loan","kind":"LIABILITY_BALANCE","amount":1000}]},
            "2026-10":{"assets":[{"id":"future-cash","kind":"CASH_RESERVE","amount":40000}]},
            "invalid":{"assets":[{"id":"bad","amount":60000}]}
        }
    })
}

#[test]
fn desktop_overview_single_decode_matches_legacy_facades() {
    let fixtures = [
        json!({}),
        json!({"activeIncomeMonthly":7000,"livingExpenseMonthly":1800,"cashReserve":24000}),
        detailed_fixture(),
        json!({"monthlySnapshots":{"0001-01":{"assets":[{"amount":1}]}}}),
        json!({"dailyLedgers":{"2026-09-01":{"confirmedAtEpochMillis":1}},"monthlySnapshots":{"2026-09":{"confirmedAtEpochMillis":1}}}),
    ];
    for (fixture_index, fixture) in fixtures.into_iter().enumerate() {
        let raw = fixture.to_string();
        for period in 0..=3 {
            for (year, month, day) in [(2024, 2, 29), (2026, 9, 15), (2026, 12, 31), (10000, 9, 15)]
            {
                assert_eq!(
                    legacy_overview(&raw, period, year, month, day),
                    build_desktop_finance_overview_values(&raw, period, year, month, day),
                    "fixture={fixture_index}, period={period}, date={year}-{month}-{day}"
                );
            }
        }
    }
}

#[test]
fn desktop_overview_preserves_cutoffs_tombstones_and_unknown_states() {
    let raw = detailed_fixture().to_string();
    let value = build_desktop_finance_overview_values(&raw, PERIOD_MONTH, 2026, 9, 15).unwrap();
    assert_eq!(1000, value.report.total_income);
    assert_eq!(200, value.report.total_outflow);
    assert_eq!(4000, value.report.net_worth);
    assert_eq!(Some("2026-09"), value.latest_month.as_deref());
    assert!(value.health.is_some());
    let empty = build_desktop_finance_overview_values("{}", PERIOD_MONTH, 2026, 9, 15).unwrap();
    assert!(empty.health.is_none());
    assert_eq!(Some(""), empty.latest_month.as_deref());
    let risk: FinanceRiskCockpitSnapshot = serde_json::from_str(&empty.risk_json).unwrap();
    assert_eq!(FinanceRiskState::Unknown, risk.risk_state);
    assert!(!risk.forecast_available);
}

#[test]
fn desktop_overview_rejects_invalid_scope_or_profile() {
    for (period, year, month, day) in [
        (-1, 2026, 9, 15),
        (4, 2026, 9, 15),
        (1, 0, 9, 15),
        (1, 2026, 13, 1),
        (1, 2026, 2, 29),
    ] {
        assert!(build_desktop_finance_overview_values("{}", period, year, month, day).is_none());
    }
    for raw in [
        "invalid",
        "null",
        "{\"dailyLedgers\":[]}",
        "{\"activeIncomeMonthly\":\"invalid\"}",
    ] {
        assert!(build_desktop_finance_overview_values(raw, PERIOD_MONTH, 2026, 9, 15).is_none());
    }
}

fn benchmark_fixture(months: usize) -> String {
    let mut ledgers = serde_json::Map::new();
    let mut snapshots = serde_json::Map::new();
    for index in 0..months {
        let absolute = 2026 * 12 + 8 - index as i32;
        let year = absolute / 12;
        let month = absolute % 12 + 1;
        for day in 1..=28 {
            let key = format!("{year:04}-{month:02}-{day:02}");
            ledgers.insert(key.clone(), json!({
                "incomes":[{"id":format!("income-{key}"),"amount":300,"note":"i".repeat(256)}],
                "expenses":[{"id":format!("expense-{key}"),"bucket":"FOOD","amount":100,"note":"e".repeat(256)}],
                "note":"n".repeat(256)
            }));
        }
        snapshots.insert(format!("{year:04}-{month:02}"), json!({"assets":[{"id":format!("cash-{index}"),"kind":"CASH_RESERVE","amount":10000}],"liabilities":[]}));
    }
    json!({"dailyLedgers":ledgers,"monthlySnapshots":snapshots}).to_string()
}

#[test]
#[ignore = "Explicit synthetic finance overview decode benchmark; no personal data is read"]
fn desktop_overview_single_decode_benchmark() {
    fn measure(mut operation: impl FnMut()) -> Value {
        operation();
        let mut micros = Vec::with_capacity(9);
        for _ in 0..9 {
            let start = Instant::now();
            operation();
            micros.push(start.elapsed().as_micros() as u64);
        }
        micros.sort_unstable();
        json!({"medianMicros":micros[4],"p95Micros":micros[8]})
    }
    let mut results = Vec::new();
    for months in [6, 96] {
        let raw = benchmark_fixture(months);
        assert_eq!(
            legacy_overview(&raw, PERIOD_MONTH, 2026, 9, 22),
            build_desktop_finance_overview_values(&raw, PERIOD_MONTH, 2026, 9, 22)
        );
        let legacy = measure(|| {
            std::hint::black_box(
                legacy_overview(std::hint::black_box(&raw), PERIOD_MONTH, 2026, 9, 22).unwrap(),
            );
        });
        let single_decode = measure(|| {
            std::hint::black_box(
                build_desktop_finance_overview_values(
                    std::hint::black_box(&raw),
                    PERIOD_MONTH,
                    2026,
                    9,
                    22,
                )
                .unwrap(),
            );
        });
        results.push(json!({"months":months,"days":months*28,"profileBytes":raw.len(),"legacyFiveDecodes":legacy,"singleDecode":single_decode}));
    }
    let report = json!({"scope":"synthetic same-binary financial overview preparation; excludes rendering/persistence","results":results});
    println!("FINANCE_OVERVIEW_PERFORMANCE {report}");
    if let Some(path) = std::env::var_os("DESKTOP_FINANCE_PERFORMANCE_OUTPUT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}
