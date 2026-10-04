// Windows - Verify unfinished money through real controls and durable save boundaries.

fn money_test_field(monthly: bool) -> DesktopMoneyField {
    DesktopMoneyField::Row {
        monthly,
        period: if monthly { "2026-10" } else { "2026-10-02" }.into(),
        list: if monthly { "assets" } else { "expenses" }.into(),
        id: if monthly { "cash" } else { "expense" }.into(),
        index: 0,
    }
}

fn money_test_client(root: &Path) -> TimerWindowsClient {
    let mut client = test_client_for_account_scope(
        root,
        "money-owner",
        app_state_with_note("money-note", "before", "body", None),
    );
    client.finance_draft = serde_json::from_value(json!({
        "dailyLedgers": {"2026-10-02": {
            "incomes":[], "expenses":[{"id":"expense", "name":"subscription", "bucket":"LIVING", "amount":58, "amountMinor":5890, "updatedAtEpochMillis":1}],
            "confirmedAtEpochMillis":1
        }},
        "monthlySnapshots": {"2026-10": {
            "assets":[{"id":"cash", "name":"cash", "kind":"CASH_RESERVE", "amount":101, "amountMinor":10121, "updatedAtEpochMillis":1}],
            "liabilities":[], "confirmedAtEpochMillis":1
        }}
    })).unwrap();
    client.mark_finance_dirty();
    client.flush_finance_draft().unwrap();
    client.finance_workbench.day_key = "2026-10-02".into();
    client.finance_workbench.month_key = "2026-10".into();
    client.finance_workbench.tab = 0;
    client
}

fn money_test_frame(
    client: &mut TimerWindowsClient,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) {
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1240.0, 1400.0),
            )),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| client.ui_finance(ui));
        },
    );
}

fn money_test_replace(
    client: &mut TimerWindowsClient,
    ctx: &egui::Context,
    field: &DesktopMoneyField,
    text: &str,
) {
    let id = ctx
        .data(|data| data.get_temp::<egui::Id>(egui::Id::new(("finance_money_input", field))))
        .unwrap();
    ctx.memory_mut(|memory| memory.request_focus(id));
    let command = egui::Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    };
    let mut events = vec![egui::Event::Key {
        key: egui::Key::A,
        physical_key: Some(egui::Key::A),
        pressed: true,
        repeat: false,
        modifiers: command,
    }];
    if text.is_empty() {
        events.push(egui::Event::Key {
            key: egui::Key::Backspace,
            physical_key: Some(egui::Key::Backspace),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
    } else {
        events.push(egui::Event::Text(text.into()));
    }
    money_test_frame(client, ctx, events);
}

#[test]
fn money_drafts_keyboard_rejects_empty_and_third_decimal_then_saves_exact_whole_amount() {
    let root = temp_test_dir("money_draft_keyboard");
    let mut client = money_test_client(&root);
    let ctx = workspace_test_context();
    let field = money_test_field(false);
    money_test_frame(&mut client, &ctx, vec![]);
    for invalid in ["", ".", "58.901"] {
        money_test_replace(&mut client, &ctx, &field, invalid);
        assert_eq!(
            client.finance_workbench.money_drafts.fields[&field].text,
            invalid
        );
        assert!(!client.finance_money_inputs_ready());
        assert_eq!(field.source(&client.finance_draft), Some((58, Some(5890))));
        assert!(client.flush_finance_draft().is_err());
    }
    money_test_replace(&mut client, &ctx, &field, "58.00");
    assert!(client.finance_money_inputs_ready());
    client.flush_finance_draft().unwrap();
    let disk = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    assert_eq!(field.source(&disk.finance_profile), Some((58, Some(5800))));
    assert_eq!(
        disk.finance_profile.extra["dailyLedgers"]["2026-10-02"]["confirmedAtEpochMillis"],
        0
    );
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn money_drafts_block_hidden_save_review_and_close_without_blocking_note_autosave() {
    let root = temp_test_dir("money_draft_boundaries");
    let mut client = money_test_client(&root);
    let ctx = workspace_test_context();
    money_test_frame(&mut client, &ctx, vec![]);
    money_test_replace(&mut client, &ctx, &money_test_field(false), "58.901");
    client.finance_draft.extra.get_mut("dailyLedgers").unwrap()["2026-10-02"]["expenses"][0]
        ["name"] = json!("pending name");
    client.mark_finance_dirty();
    let disk_before = fs::read(&client.state_path).unwrap();
    client.finance_workbench.tab = 5;
    client.finance_workbench.day_key = "2026-10-03".into();
    assert!(client.flush_finance_draft().is_err());
    client.submit_draft_snapshot(&ctx, true);
    assert!(
        !client.persistence.pending(),
        "invalid finance must not enter the worker"
    );
    assert_eq!(fs::read(&client.state_path).unwrap(), disk_before);
    client.ensure_desktop_finance_review();
    client.desktop_finance_apply_review_action(
        DesktopFinanceReviewAction::Confirm(DesktopFinanceReviewLane::Cash, false),
        "2026-10-02",
        "2026-10-02",
    );
    assert!(client
        .finance_workbench
        .review
        .as_ref()
        .unwrap()
        .receipts
        .is_empty());
    assert!(!desktop_finance_review_path(&client.state_path).exists());

    client.select_note_by_id("money-note");
    client.note_title_draft = "saved despite unfinished finance".into();
    client.mark_note_dirty();
    client.submit_draft_snapshot(&ctx, true);
    assert!(client.persistence.pending());
    drain_draft_writer(&mut client);
    assert!(!client.note_dirty);
    assert!(client.finance_dirty);
    let disk = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    assert_eq!(
        disk.notes
            .iter()
            .find(|note| note.id == "money-note")
            .unwrap()
            .title,
        "saved despite unfinished finance"
    );
    assert_eq!(
        disk.finance_profile.extra["dailyLedgers"]["2026-10-02"]["expenses"][0]["name"],
        "subscription"
    );
    assert_eq!(
        money_test_field(false).source(&disk.finance_profile),
        Some((58, Some(5890)))
    );

    client.close_with_save_barrier(&ctx);
    assert!(matches!(
        client.shutdown_state,
        ClientShutdownState::SaveFailed { .. }
    ));
    assert!(!client.persistence.waiting_to_close);
    assert!(client.finance_workbench.money_drafts.has_invalid());
    client.shutdown_state = ClientShutdownState::Running;
    client.persistence.waiting_to_close = true;
    client.advance_safe_shutdown(&ctx);
    assert!(matches!(
        client.shutdown_state,
        ClientShutdownState::SaveFailed { .. }
    ));
    assert!(!client.persistence.waiting_to_close);
    client.shutdown_state = ClientShutdownState::Running;
    client.finance_workbench.money_drafts.cancel_invalid();
    client.flush_finance_draft().unwrap();
    let disk = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    assert_eq!(
        disk.finance_profile.extra["dailyLedgers"]["2026-10-02"]["expenses"][0]["name"],
        "pending name"
    );
    assert_eq!(
        money_test_field(false).source(&disk.finance_profile),
        Some((58, Some(5890)))
    );
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn money_drafts_prevent_day_and_month_confirmation_even_after_period_navigation() {
    for monthly in [false, true] {
        let root = temp_test_dir("money_draft_confirmation");
        let mut client = money_test_client(&root);
        client.finance_workbench.tab = if monthly { 1 } else { 0 };
        let ctx = workspace_test_context();
        let field = money_test_field(monthly);
        money_test_frame(&mut client, &ctx, vec![]);
        money_test_replace(&mut client, &ctx, &field, "58.901");
        let collection = if monthly {
            "monthlySnapshots"
        } else {
            "dailyLedgers"
        };
        let period = if monthly { "2026-10" } else { "2026-10-02" };
        assert_eq!(
            client.finance_draft.extra[collection][period]["confirmedAtEpochMillis"],
            0
        );
        let confirmation = egui::Id::new(("finance_complete", monthly, period));
        let (rect, enabled) = ctx
            .data(|data| data.get_temp::<(egui::Rect, bool)>(confirmation))
            .unwrap();
        assert!(!enabled);
        for pressed in [true, false] {
            money_test_frame(
                &mut client,
                &ctx,
                vec![
                    egui::Event::PointerMoved(rect.center()),
                    egui::Event::PointerButton {
                        pos: rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert_eq!(
            client.finance_draft.extra[collection][period]["confirmedAtEpochMillis"],
            0
        );
        if monthly {
            client.finance_workbench.month_key = "2026-09".into();
        } else {
            client.finance_workbench.day_key = "2026-10-01".into();
        }
        money_test_frame(&mut client, &ctx, vec![]);
        let other = if monthly { "2026-09" } else { "2026-10-01" };
        let other_confirmation = egui::Id::new(("finance_complete", monthly, other));
        assert!(
            !ctx.data(|data| data.get_temp::<(egui::Rect, bool)>(other_confirmation))
                .unwrap()
                .1
        );
        assert!(
            !client.finance_money_inputs_ready(),
            "hidden input must survive navigation"
        );
        client.finance_workbench.money_drafts.cancel_invalid();
        money_test_frame(&mut client, &ctx, vec![]);
        assert!(
            ctx.data(|data| data.get_temp::<(egui::Rect, bool)>(other_confirmation))
                .unwrap()
                .1
        );
        assert_eq!(
            field.source(&client.finance_draft),
            if monthly {
                Some((101, Some(10121)))
            } else {
                Some((58, Some(5890)))
            }
        );
        drop(client);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn money_drafts_keep_invalid_input_when_only_the_sync_route_changes() {
    let root = temp_test_dir("money_draft_route_change");
    let mut client = money_test_client(&root);
    let ctx = workspace_test_context();
    money_test_frame(&mut client, &ctx, vec![]);
    money_test_replace(&mut client, &ctx, &money_test_field(false), "58.901");
    let owner_scope = client.finance_workbench.money_drafts.scope.clone();
    let disk_before = fs::read(&client.state_path).unwrap();
    for route in [
        "https://first-route.invalid",
        "https://second-route.invalid",
    ] {
        client.sync.server_url = route.into();
        assert!(!client.finance_money_inputs_ready());
        assert_eq!(client.finance_workbench.money_drafts.scope, owner_scope);
        assert!(client.flush_finance_draft().is_err());
        assert_eq!(fs::read(&client.state_path).unwrap(), disk_before);
    }
    client.ensure_desktop_finance_review();
    client.desktop_finance_apply_review_action(
        DesktopFinanceReviewAction::Confirm(DesktopFinanceReviewLane::Cash, false),
        "2026-10-02",
        "2026-10-02",
    );
    assert!(client
        .finance_workbench
        .review
        .as_ref()
        .unwrap()
        .receipts
        .is_empty());
    client.close_with_save_barrier(&ctx);
    assert!(matches!(
        client.shutdown_state,
        ClientShutdownState::SaveFailed { .. }
    ));
    assert!(!client.persistence.waiting_to_close);
    client.sync.server_instance_id = "another-workspace-owner".into();
    assert!(client.finance_money_inputs_ready());
    assert!(client.finance_workbench.money_drafts.fields.is_empty());
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn money_drafts_reconcile_deleted_replaced_and_other_workspace_records() {
    let mut profile: DesktopFinanceProfile = serde_json::from_value(json!({
        "dailyLedgers":{"2026-10-02":{"expenses":[{"id":"expense","amount":58,"amountMinor":5890}]}}
    }))
    .unwrap();
    let field = money_test_field(false);
    let mut drafts = DesktopMoneyDrafts::default();
    drafts.reconcile("a".into(), &profile);
    let invalid = DesktopMoneyDraft {
        source: (58, Some(5890)),
        text: "58.901".into(),
    };
    drafts.fields.insert(field.clone(), invalid.clone());
    drafts.reconcile("a".into(), &profile);
    assert!(drafts.has_invalid());
    profile.extra.get_mut("dailyLedgers").unwrap()["2026-10-02"]["expenses"][0]["amountMinor"] =
        json!(5800);
    drafts.reconcile("a".into(), &profile);
    assert!(!drafts.has_invalid());
    profile.extra.get_mut("dailyLedgers").unwrap()["2026-10-02"]["expenses"][0]["amountMinor"] =
        json!(5890);
    drafts.fields.insert(field.clone(), invalid.clone());
    profile.extra.get_mut("dailyLedgers").unwrap()["2026-10-02"]["expenses"][0]
        ["deletedAtEpochMillis"] = json!(1);
    drafts.reconcile("a".into(), &profile);
    assert!(!drafts.has_invalid());
    profile.extra.get_mut("dailyLedgers").unwrap()["2026-10-02"]["expenses"][0]
        ["deletedAtEpochMillis"] = json!(0);
    drafts.fields.insert(field.clone(), invalid.clone());
    drafts.reconcile("b".into(), &profile);
    assert!(!drafts.has_invalid());
    drafts.fields.insert(field.clone(), invalid);
    profile.extra.get_mut("dailyLedgers").unwrap()["2026-10-02"]["expenses"]
        .as_array_mut()
        .unwrap()
        .insert(0, json!({"id":"another","amount":12,"amountMinor":1234}));
    drafts.reconcile("b".into(), &profile);
    assert!(
        drafts.has_invalid(),
        "reordering the same record must retain unfinished input"
    );
    assert_eq!(field.source(&profile), Some((58, Some(5890))));
    drafts.cancel_invalid();
    assert!(!drafts.has_invalid());
    assert_eq!(field.source(&profile), Some((58, Some(5890))));
}
