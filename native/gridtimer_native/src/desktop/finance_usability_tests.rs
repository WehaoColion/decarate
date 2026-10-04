// v2.22.39 - Verify finance view navigation, live totals, and draft preservation.

#[test]
fn finance_live_totals_ignore_deleted_rows_and_keep_negative_net() {
    let record = json!({
        "incomes":[{"amount":200},{"amount":9000,"deletedAtEpochMillis":1}],
        "expenses":[{"amount":350},{"amount":-90}],
        "assets":[{"amount":800},{"amount":5}],
        "liabilities":[{"amount":950},{"amount":2000,"deletedAtEpochMillis":1}]
    });
    assert_eq!(
        Some((20000, 35000, -15000)),
        finance_record_totals(&record, false)
    );
    assert_eq!(
        Some((80500, 95000, -14500)),
        finance_record_totals(&record, true)
    );
    assert_eq!(Some((0, 0, 0)), finance_record_totals(&json!({}), false));
    let extreme =
        json!({"incomes":[{"amount":i64::MAX},{"amount":i64::MAX}],"expenses":[{"amount":1}]});
    assert_eq!(None, finance_record_totals(&extreme, false));
}

#[test]
fn finance_views_are_reachable_without_losing_unsaved_entries() {
    for size in [egui::vec2(760.0, 480.0), egui::vec2(1240.0, 800.0)] {
        let dir = temp_test_dir("finance_view_navigation");
        let mut client = test_client_for_account_scope(
            &dir,
            "account-a",
            app_data::default_app_data_json(now_millis()),
        );
        client.switch_tab(AppTab::Finance);
        client.finance_draft.extra.insert("dailyLedgers".into(), json!({"2026-09-11":{"incomes":[{"id":"income","name":"已输入","amount":123,"kind":"ACTIVE","updatedAtEpochMillis":1}],"expenses":[]}}));
        client.mark_finance_dirty();
        let before = serde_json::to_value(&client.finance_draft).unwrap();
        let ctx = workspace_test_context();
        for tab in [1, 3, 4, 2, 5, 6, 0] {
            for _ in 0..3 {
                workspace_test_frame(&mut client, &ctx, size, vec![]);
            }
            let rect = ctx
                .data(|data| data.get_temp::<egui::Rect>(egui::Id::new(("finance_tab", tab))))
                .unwrap();
            assert!(
                rect.top() >= 76.0 && rect.bottom() < 360.0 && rect.right() <= size.x,
                "tab {tab}: {rect:?}"
            );
            workspace_click(&mut client, &ctx, size, rect.center());
            assert_eq!(tab, client.finance_workbench.tab);
            assert_eq!(before, serde_json::to_value(&client.finance_draft).unwrap());
        }
        client.flush_finance_draft().unwrap();
        let saved: Value =
            serde_json::from_str(&fs::read_to_string(&client.state_path).unwrap()).unwrap();
        assert_eq!(
            123,
            saved["financeProfile"]["dailyLedgers"]["2026-09-11"]["incomes"][0]["amount"]
        );
        drop(client);
        fs::remove_dir_all(dir).unwrap();
    }
}
