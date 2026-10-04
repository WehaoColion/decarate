// v2.22.48 - Keep internal timer history out of primary navigation.
// v2.22.46 - Keep global timer actions behind the account replacement barrier.

#[test]
fn workspace_header_pause_respects_account_replacement_barrier() {
    let dir = temp_test_dir("header_pause_barrier");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_data::default_app_data_json(now_millis()),
    );
    client.settings.timer_bell_enabled = false;
    let ctx = workspace_test_context();
    client.toggle_slot(&client.selected_slot().unwrap(), &ctx);
    wait_for_timer_action(&mut client);
    let size = egui::vec2(1240.0, 800.0);
    for kind in [
        SyncTaskKind::Download,
        SyncTaskKind::Login,
        SyncTaskKind::Register,
        SyncTaskKind::Upload,
    ] {
        client.sync_task = Some(SyncTaskState {
            kind,
            phase: SyncTaskPhase::AppData,
            started_at_epoch_millis: now_millis(),
        });
        for _ in 0..3 {
            workspace_test_frame(&mut client, &ctx, size, vec![]);
        }
        let (rect, enabled) = ctx
            .data(|d| d.get_temp::<(egui::Rect, bool)>(egui::Id::new("header_pause")))
            .unwrap();
        assert!(
            !enabled,
            "pause must be disabled while replacing account data"
        );
        workspace_click(&mut client, &ctx, size, rect.center());
        assert!(client
            .selected_slot()
            .unwrap()
            .running_since_epoch_millis
            .is_some());
    }
    client.sync_task = None;
    for _ in 0..3 {
        workspace_test_frame(&mut client, &ctx, size, vec![]);
    }
    let (rect, enabled) = ctx
        .data(|d| d.get_temp::<(egui::Rect, bool)>(egui::Id::new("header_pause")))
        .unwrap();
    assert!(enabled);
    workspace_click(&mut client, &ctx, size, rect.center());
    wait_for_timer_action(&mut client);
    assert!(client
        .selected_slot()
        .unwrap()
        .running_since_epoch_millis
        .is_none());
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_all_navigation_destinations_remain_clickable_in_short_windows() {
    let dir = temp_test_dir("short_window_navigation");
    let mut client = test_client_for_account_scope(
        &dir,
        "account-a",
        app_data::default_app_data_json(now_millis()),
    );
    let ctx = workspace_test_context();
    for size in [egui::vec2(760.0, 480.0), egui::vec2(1000.0, 480.0)] {
        for tab in [
            AppTab::My,
            AppTab::Finance,
            AppTab::Knowledge,
            AppTab::Notes,
            AppTab::Board,
        ] {
            for _ in 0..3 {
                workspace_test_frame(&mut client, &ctx, size, vec![]);
            }
            assert!(
                ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new((
                    "desktop_nav",
                    AppTab::History.title()
                ))))
                .is_none(),
                "timer history must not be a primary navigation target"
            );
            let rect = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("desktop_nav", tab.title()))))
                .unwrap();
            assert!(
                rect.bottom() <= size.y,
                "navigation target must fit inside the window"
            );
            workspace_click(&mut client, &ctx, size, rect.center());
            assert!(
                client.tab == tab,
                "navigation must open the selected destination"
            );
        }
    }
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}
