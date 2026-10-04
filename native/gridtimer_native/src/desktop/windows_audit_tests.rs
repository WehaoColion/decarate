// v2.22.51 - Keep an already-due local interval when remote totals change.
// v2.22.50 - Exercise synchronization against active desktop editor and reminder state.

fn audit_apply_remote(client: &mut TimerWindowsClient, raw: String) {
    let response = bound_app_data_response(client, raw);
    let identity = bound_result_expectation(client);
    let applied =
        client.apply_sync_result(&serde_json::to_string(&response).unwrap(), false, &identity);
    assert!(applied.ok, "{}", applied.message);
}

#[test]
fn windows_audit_unrelated_sync_preserves_undo_and_redo() {
    let dir = temp_test_dir("audit_undo");
    let mut client = test_client_for_account_scope(
        &dir,
        "undo",
        app_state_with_note("n", "Title", "Before", None),
    );
    client.select_note_by_id("n");
    client.set_note_canvas_text("After");
    assert!(client.save_note());
    assert!(!client.note_undo_stack.is_empty());
    let remote = app_data::update_slot_title_app_data_json(
        &client.state_json,
        2,
        "Remote timer",
        now_millis(),
    )
    .unwrap();
    audit_apply_remote(&mut client, remote);
    client.undo_note_draft();
    assert_eq!(
        "Before", client.note_content_draft,
        "unrelated sync discarded saved editor undo"
    );
    client.redo_note_draft();
    assert_eq!("After", client.note_content_draft);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn windows_audit_unrelated_sync_preserves_read_only_note_version() {
    let dir = temp_test_dir("audit_version");
    let mut client = test_client_for_account_scope(&dir, "version", app_state_with_note_versions());
    client.select_note_by_id("note-versioned");
    assert!(client.open_note_version("version-old"));
    let before = client.note_content_draft.clone();
    let remote = client.state_json.clone();
    audit_apply_remote(&mut client, remote);
    assert_eq!(
        "version-old", client.selected_note_version_id,
        "sync silently left the read-only version"
    );
    assert_eq!(before, client.note_content_draft);
    assert!(client.selected_note_version().is_some());
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn windows_audit_unrelated_sync_keeps_unlocked_note_session() {
    let dir = temp_test_dir("audit_unlock");
    let mut client = test_client_for_account_scope(
        &dir,
        "unlock",
        app_state_with_note("n", "Title", "Private fixture", None),
    );
    client.select_note_by_id("n");
    client.note_crypto_password_draft = "isolated-test-password".into();
    client.enable_note_encryption();
    assert!(!client.note_crypto_session_token.is_empty());
    assert!(!client.selected_note_is_locked());
    let remote = client.state_json.clone();
    audit_apply_remote(&mut client, remote);
    assert!(
        !client.selected_note_is_locked(),
        "unchanged encrypted note was locked by sync"
    );
    assert_eq!("Private fixture", client.note_content_draft);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn windows_audit_synced_totals_do_not_create_a_local_interval_bell() {
    let dir = temp_test_dir("audit_interval");
    let now = now_millis();
    let raw =
        app_data::start_slot_app_data_json(&app_data::default_app_data_json(now), 1, now).unwrap();
    let mut client = test_client_for_account_scope(&dir, "interval", raw);
    client.settings.timer_bell_enabled = false;
    client.settings.timer_bell_interval_minutes = 25;
    client.rebuild_bell_markers();
    assert_eq!(Some(0), client.slot_bell_marker(1));
    let mut remote: Value = serde_json::from_str(&client.state_json).unwrap();
    remote["slots"][0]["accumulatedMillis"] = json!(1_800_000);
    remote["slots"][0]["accumulatedUpdatedAtEpochMillis"] = json!(now + 1);
    audit_apply_remote(&mut client, remote.to_string());
    let slot = client.data.slots.iter().find(|slot| slot.id == 1).unwrap();
    let current_index = timer_bell_interval_index_at(slot, 25, now_millis());
    assert!(current_index >= 1);
    assert_eq!(
        Some(current_index),
        client.slot_bell_marker(1),
        "remote totals armed an immediate local bell"
    );
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn windows_audit_unchanged_total_sync_keeps_a_genuinely_due_interval() {
    let dir = temp_test_dir("audit_real_interval");
    let now = now_millis();
    let raw =
        app_data::start_slot_app_data_json(&app_data::default_app_data_json(now), 1, now).unwrap();
    let mut state: Value = serde_json::from_str(&raw).unwrap();
    state["slots"][0]["accumulatedMillis"] = json!(1_800_000);
    let mut client = test_client_for_account_scope(
        &dir,
        "due",
        app_data::sanitize_app_data_json(&state.to_string(), now).unwrap(),
    );
    client.settings.timer_bell_enabled = false;
    client.settings.timer_bell_interval_minutes = 25;
    client.set_slot_bell_marker(1, 0);
    let remote = client.state_json.clone();
    audit_apply_remote(&mut client, remote);
    assert_eq!(
        Some(0),
        client.slot_bell_marker(1),
        "unchanged sync suppressed a due local reminder"
    );
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn windows_audit_changed_total_sync_keeps_an_already_due_local_interval() {
    for incoming_total in [0, 3_600_000] {
        let dir = temp_test_dir("audit_due_changed_total");
        let now = now_millis();
        let raw = app_data::start_slot_app_data_json(&app_data::default_app_data_json(now), 1, now)
            .unwrap();
        let mut state: Value = serde_json::from_str(&raw).unwrap();
        state["slots"][0]["accumulatedMillis"] = json!(1_800_000);
        let mut client = test_client_for_account_scope(
            &dir,
            "due_changed",
            app_data::sanitize_app_data_json(&state.to_string(), now).unwrap(),
        );
        client.settings.timer_bell_enabled = true;
        client.settings.timer_bell_interval_minutes = 25;
        client.set_slot_bell_marker(1, 0);
        let mut remote: Value = serde_json::from_str(&client.state_json).unwrap();
        remote["slots"][0]["accumulatedMillis"] = json!(incoming_total);
        remote["slots"][0]["accumulatedUpdatedAtEpochMillis"] = json!(now + 1);
        audit_apply_remote(&mut client, remote.to_string());
        let slot = client.data.slots.iter().find(|slot| slot.id == 1).unwrap();
        assert_eq!(incoming_total, slot.accumulated_millis);
        // Exercise the production decision and consumption path without playing
        // audio. Cosmetic text changes cannot change these event counts.
        assert_eq!(
            1,
            client.take_due_interval_timer_bells(now_millis()).len(),
            "remote history swallowed an already-due local reminder"
        );
        assert!(
            client
                .take_due_interval_timer_bells(now_millis())
                .is_empty(),
            "the same local reminder was issued twice"
        );
        drop(client);
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn windows_audit_changed_encrypted_note_invalidates_old_unlock() {
    let dir = temp_test_dir("audit_changed_unlock");
    let mut client = test_client_for_account_scope(
        &dir,
        "changed",
        app_state_with_note("n", "Title", "Before", None),
    );
    client.select_note_by_id("n");
    client.note_crypto_password_draft = "isolated-test-password".into();
    client.enable_note_encryption();
    assert!(!client.selected_note_is_locked());
    let mut edited = client.selected_note().unwrap();
    edited.title = "Changed on another device".into();
    edited.updated_at_epoch_millis = now_millis() + 10;
    let sealed = gridtimer_native::seal_desktop_note_json(
        &serde_json::to_string(&edited).unwrap(),
        &client.note_crypto_session_token,
    )
    .unwrap();
    let remote =
        app_data::upsert_note_app_data_json(&client.state_json, &sealed, now_millis() + 10)
            .unwrap();
    audit_apply_remote(&mut client, remote);
    assert!(
        client.selected_note_is_locked(),
        "changed encrypted record reused the old unlocked contents"
    );
    assert!(client.note_crypto_session_token.is_empty());
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}
