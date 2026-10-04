// v1.0.3.1 Windows - Keep same-ID encryption sessions inside their workspace.

#[test]
fn crypto_scope_uses_verified_owner_and_stable_workspace_identity() {
    let root = temp_test_dir("crypto_scope_identity");
    let other_root = root.join("other-workspace");
    fs::create_dir_all(&other_root).unwrap();
    let state = app_state_with_note("crypto-scope-id", "Title", "Content", None);
    let mut client = test_client_for_account_scope(&root, "owner-a", state.clone());
    let scope_a = client.note_crypto_session_scope().unwrap();
    client.sync.device_name = "Different display name".into();
    client.sync.server_url = "https://other-route.example.test".into();
    client.sync.token = "a-refreshed-session-token".into();
    client.sync.token_id = sync_core::token_identifier(&client.sync.token);
    assert_eq!(scope_a, client.note_crypto_session_scope().unwrap());

    let other = test_client_for_account_scope(&other_root, "owner-a", state);
    assert_ne!(scope_a, other.note_crypto_session_scope().unwrap());
    client.sync.user_id = "owner-b".into();
    client.sync.account_namespace = test_account_namespace("owner-b");
    assert_ne!(scope_a, client.note_crypto_session_scope().unwrap());
    client.sync.account_namespace = test_account_namespace("owner-a");
    assert!(client.note_crypto_session_scope().is_err());

    client.sync.user_id.clear();
    client.sync.server_instance_id.clear();
    client.sync.account_namespace.clear();
    let guest = client.note_crypto_session_scope().unwrap();
    assert_ne!(scope_a, guest);
    assert_eq!(guest, client.note_crypto_session_scope().unwrap());
    client.sync.user_id = "legacy-owner".into();
    let legacy = client.note_crypto_session_scope().unwrap();
    assert_ne!(guest, legacy);
    assert_eq!(legacy, client.note_crypto_session_scope().unwrap());
    client.sync.server_instance_id = TEST_SERVER_INSTANCE_ID.into();
    assert!(client.note_crypto_session_scope().is_err());
    drop(client);
    drop(other);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn crypto_scope_windows_entrypoints_isolate_same_id_password_changes() {
    let root = temp_test_dir("crypto_scope_entrypoints");
    let original = app_state_with_note("shared-note-id", "Title", "Content", None);
    let mut higher: Value = serde_json::from_str(&original).unwrap();
    higher["notes"][0]["protectionStateRevision"] = json!(8);
    let higher = app_data::sanitize_app_data_json(&higher.to_string(), now_millis()).unwrap();
    let mut a = test_client_for_account_scope(&root, "crypto-owner-a", original);
    let mut b = test_client_for_account_scope(&root, "crypto-owner-b", higher);
    for client in [&mut a, &mut b] {
        client.save_state().unwrap();
        let note = client.data.notes[0].clone();
        client.load_note_draft_without_flush(&note);
        client.note_crypto_password_draft = "scope-initial-password".into();
        client.enable_note_encryption();
        assert!(
            client.data.notes[0].encryption.is_some(),
            "{}",
            client.status
        );
        assert!(!client.note_crypto_session_token.is_empty());
        client.lock_note_encryption();
        client.note_crypto_password_draft = "scope-initial-password".into();
        client.unlock_note_encryption();
        assert!(!client.selected_note_is_locked(), "{}", client.status);
    }

    let old_a = a.note_crypto_session_token.clone();
    let token_b = b.note_crypto_session_token.clone();
    let plain_b = serde_json::to_string(b.note_unlocked_record.as_ref().unwrap()).unwrap();
    a.note_crypto_new_password_draft = "scope-owner-a-next-password".into();
    a.change_note_encryption_password();
    assert!(a.note_crypto_session_token.is_empty(), "{}", a.status);
    assert!(a.selected_note_is_locked());
    assert!(!gridtimer_native::close_desktop_note_session(&old_a));
    assert!(gridtimer_native::seal_desktop_note_json(&plain_b, &token_b).is_some());

    a.note_crypto_password_draft = "scope-owner-a-next-password".into();
    a.unlock_note_encryption();
    assert!(!a.selected_note_is_locked(), "{}", a.status);
    let token_a = a.note_crypto_session_token.clone();
    let plain_a = serde_json::to_string(a.note_unlocked_record.as_ref().unwrap()).unwrap();
    b.note_crypto_new_password_draft = "scope-owner-b-next-password".into();
    b.change_note_encryption_password();
    assert!(b.note_crypto_session_token.is_empty(), "{}", b.status);
    assert!(b.selected_note_is_locked());
    assert!(!gridtimer_native::close_desktop_note_session(&token_b));
    assert!(gridtimer_native::seal_desktop_note_json(&plain_a, &token_a).is_some());
    a.close_note_crypto_session();
    b.close_note_crypto_session();
    drop(a);
    drop(b);
    fs::remove_dir_all(root).unwrap();
}
