// v2.22.49 - Verify timer independence through real desktop sync commits.
#[test]
fn device_timer_sync_desktop_commit_keeps_local_controls_for_merge_and_download() {
    for operation in [
        BoundSyncOperation::MergeSync,
        BoundSyncOperation::ExplicitDownload,
        BoundSyncOperation::NamespaceBootstrapDownload,
    ] {
        for local_running in [false, true] {
            for remote_running in [false, true] {
                let dir = temp_test_dir("device_timer_sync_commit");
                let now = now_millis();
                let seed = app_data::default_app_data_json(now - 10000);
                let local = if local_running {
                    app_data::start_slot_app_data_json(&seed, 1, now - 2000).unwrap()
                } else {
                    seed
                };
                let before: Value = serde_json::from_str(&local).unwrap();
                let mut client = test_client_for_account_scope(&dir, "account-a", local);
                client.settings.timer_bell_enabled = false;
                let mut remote: Value =
                    serde_json::from_str(&app_data::default_app_data_json(now)).unwrap();
                remote["slots"][0]["title"] = json!("手机更新的标题");
                remote["slots"][0]["titleUpdatedAtEpochMillis"] = json!(now);
                remote["slots"][0]["runningSinceEpochMillis"] = if remote_running {
                    json!(now - 1000)
                } else {
                    Value::Null
                };
                remote["slots"][0]["activeRunId"] = json!("phone-run");
                remote["slots"][0]["microBreakPhase"] = json!("BREAK");
                remote["slots"][0]["microBreakCycleIndex"] = json!(7);
                remote["slots"][0]["microBreakPhaseProgressMillis"] = json!(1500);
                let response = sync_core::SyncClientResult {
                    ok: true,
                    current_committed: true,
                    user_id: client.sync.user_id.clone(),
                    token_id: client.sync.token_id.clone(),
                    server_instance_id: client.sync.server_instance_id.clone(),
                    account_namespace: client.sync.account_namespace.clone(),
                    app_data_json: Some(remote.to_string()),
                    mode: if operation == BoundSyncOperation::MergeSync {
                        "merged"
                    } else {
                        "downloaded_account"
                    }
                    .into(),
                    ..Default::default()
                };
                let mut identity = bound_result_expectation(&client);
                if let SyncResultIdentityExpectation::BoundAccount {
                    operation: selected,
                    ..
                } = &mut identity
                {
                    *selected = operation;
                }
                let result = client.apply_sync_result(
                    &serde_json::to_string(&response).unwrap(),
                    false,
                    &identity,
                );
                assert!(result.ok, "{:?}: {}", operation, result.message);
                let persisted: Value =
                    serde_json::from_str(&fs::read_to_string(&client.state_path).unwrap()).unwrap();
                for field in [
                    "runningSinceEpochMillis",
                    "activeRunId",
                    "microBreakPhase",
                    "microBreakCycleIndex",
                    "microBreakPhaseProgressMillis",
                ] {
                    assert_eq!(
                        persisted["slots"][0][field], before["slots"][0][field],
                        "{operation:?} {field}"
                    );
                }
                assert_eq!(
                    client.data.slots[0].running_since_epoch_millis.is_some(),
                    local_running
                );
                assert_eq!(client.data.slots[0].title, "手机更新的标题");
                drop(client);
                fs::remove_dir_all(dir).unwrap();
            }
        }
    }
}
