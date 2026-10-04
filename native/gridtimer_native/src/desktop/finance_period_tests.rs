// v2.22.38 - Regress shared calendar navigation, period cutoffs, and financial snapshots.
mod finance_period_tests {
    use super::*;

    #[test]
    fn date_navigation_handles_leap_days_year_rollover_and_invalid_input() {
        for (key, month, offset, expected) in [
            ("2024-02-28", false, 1, Some("2024-02-29")),
            ("2026-03-01", false, -1, Some("2026-02-28")),
            ("2026-12-31", false, 1, Some("2027-01-01")),
            ("2026-01", true, -1, Some("2025-12")),
            ("2026-12", true, 1, Some("2027-01")),
            ("2026-02-29", false, 1, None),
            ("2026-13", true, 1, None),
            ("9999-12", true, 1, None),
            ("9999-12-31", false, 1, None),
        ] {
            assert_eq!(
                expected.map(str::to_string),
                finance_adjacent_key(key, month, offset),
                "{key}"
            );
        }
    }

    #[test]
    fn quarter_and_year_reference_dates_match_android_elapsed_month_rules() {
        assert_eq!(
            Some((2026, 5, 10, 2)),
            finance_period_reference_date("2026", 2, false, "2026-05-10")
        );
        assert_eq!(
            Some((2026, 3, 31, 3)),
            finance_period_reference_date("2026", 1, false, "2026-05-10")
        );
        assert_eq!(
            Some((2026, 5, 10, 5)),
            finance_period_reference_date("2026", 2, true, "2026-05-10")
        );
        assert_eq!(
            Some((2025, 12, 31, 12)),
            finance_period_reference_date("2025", 1, true, "2026-05-10")
        );
        assert_eq!(
            Some((9999, 12, 31, 12)),
            finance_period_reference_date("9999", 1, true, "2026-05-10")
        );
        for (year, quarter) in [
            ("bad", 1),
            ("0000", 1),
            ("10000", 1),
            ("2026", 0),
            ("2026", 5),
        ] {
            assert!(finance_period_reference_date(year, quarter, false, "2026-05-10").is_none());
        }
    }

    fn period_profile() -> Value {
        serde_json::json!({
            "dailyLedgers": {
                "2026-01-01": {"incomes":[{"id":"january","kind":"ACTIVE","amount":1000}],"expenses":[]},
                "2026-04-01": {"incomes":[{"id":"april","kind":"ACTIVE","amount":3000}],"expenses":[{"id":"food","bucket":"FOOD","amount":200}]},
                "2026-05-10": {"incomes":[{"id":"may","kind":"ASSET","amount":600},{"id":"deleted","kind":"ACTIVE","amount":9999,"deletedAtEpochMillis":2}],"expenses":[]},
                "2026-05-11": {"incomes":[{"id":"future","kind":"ACTIVE","amount":8888}],"expenses":[]}
            },
            "monthlySnapshots": {
                "2025-12": {"assets":[{"id":"opening","kind":"CASH_RESERVE","amount":2000}],"liabilities":[]},
                "2026-04": {"assets":[{"id":"closing","kind":"CASH_RESERVE","amount":5000}],"liabilities":[{"id":"loan","kind":"LIABILITY_BALANCE","amount":1000}]},
                "2026-06": {"assets":[{"id":"future-snapshot","kind":"CASH_RESERVE","amount":9000}],"liabilities":[]}
            }
        })
    }

    #[test]
    fn summary_uses_shared_totals_excludes_future_and_deleted_rows_and_preserves_source() {
        let profile = period_profile();
        let before = profile.clone();
        let quarter =
            finance_build_period_summary(&profile, "2026", 2, false, "2026-05-10").unwrap();
        assert_eq!(
            (360000, 20000, 340000),
            finance_period_cashflow(&quarter.totals)
        );
        assert_eq!(2, quarter.totals.days_with_entries);
        assert_eq!(2, quarter.elapsed_months);
        assert!(quarter.net_worth.is_none());
        let shared = gridtimer_native::aggregate_finance_ledger_values(
            &gridtimer_native::finance_money::project_profile_json(&profile.to_string()).unwrap(),
            2,
            2026,
            5,
            10,
        )
        .unwrap();
        assert_eq!(shared, quarter.totals);
        let year = finance_build_period_summary(&profile, "2026", 2, true, "2026-05-10").unwrap();
        assert_eq!(
            (460000, 20000, 440000),
            finance_period_cashflow(&year.totals)
        );
        assert_eq!(Some([200000, 400000, 202512, 202604, 1]), year.net_worth);
        assert_eq!(before, profile);
    }

    #[test]
    fn missing_snapshots_and_unrecorded_days_do_not_create_fake_comparisons() {
        let profile = serde_json::json!({});
        let year = finance_build_period_summary(&profile, "2026", 1, true, "2026-05-10").unwrap();
        assert_eq!(0, year.totals.days_with_entries);
        assert_eq!((0, 0, 0), finance_period_cashflow(&year.totals));
        assert_eq!(Some([0, 0, 0, 0, 0]), year.net_worth);
    }

    #[test]
    fn quarter_and_year_views_fit_narrow_windows_without_changing_financial_data() {
        for tab in [3, 4] {
            let context = egui::Context::default();
            let profile = period_profile();
            let before = profile.clone();
            let mut state = FinanceWorkbenchState {
                tab,
                period_year: "2026".into(),
                quarter: 2,
                ..Default::default()
            };
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(460.0, 900.0),
                )),
                ..Default::default()
            };
            let _ = context.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    finance_period_summary_ui(ui, &mut state, &profile, "2026-05-10", (1, 0));
                    assert!(ui.min_rect().width() <= 460.0, "tab {tab}");
                });
            });
            assert_eq!(before, profile);
            assert!(state.summary_cache.is_some());
        }
    }
}
