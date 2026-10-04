// v0.0.3 - Include independent local workspaces belonging to the same account.
// v0.0.2 - Reproduce password changes leaking across independent account sessions.
// v0.0.1 - Exercise durable references through actual desktop encryption actions.
fn private_session_scope(
    client: &TimerWindowsClient,
) -> gridtimer_native::desktop_media_references::DesktopMediaReferenceScope {
    let root = client.sync_path.parent().unwrap();
    let store = DesktopStateStore::open(desktop_state_store_path(root)).unwrap();
    let policy = store.privacy_policy(&audit_owner(client)).unwrap().0;
    gridtimer_native::desktop_media_references::DesktopMediaReferenceScope::from_snapshot(
        &client.state_json,
        &policy,
    )
}

#[test]
fn private_media_session_seal_autosave_password_and_restart_keep_references() {
    let root = temp_test_dir("private-session-lifecycle");
    let mut client = test_client_for_account_scope(
        &root,
        "owner",
        app_state_with_note("private", "Private", "SESSION_BODY_791", None),
    );
    client.tab = AppTab::Notes;
    client.save_state().unwrap();
    client.select_note_by_id("private");
    client.note_crypto_password_draft = "session-lifecycle-password".into();
    client.enable_note_encryption();
    assert!(
        client.data.notes[0].encryption.is_some(),
        "{}",
        client.status
    );
    assert!(
        private_session_scope(&client).is_complete(),
        "sealed note committed without its authenticated references"
    );
    let old_state = client.state_json.clone();
    client.set_note_canvas_text("SESSION_EDIT_792");
    client.submit_draft_snapshot(&egui::Context::default(), true);
    drain_draft_writer(&mut client);
    assert!(!client.note_dirty, "{}", client.status);
    assert_ne!(old_state, client.state_json);
    assert!(
        private_session_scope(&client).is_complete(),
        "autosave lost sealed references"
    );
    client.note_crypto_new_password_draft = "session-replacement-password".into();
    client.change_note_encryption_password();
    assert!(
        client.note_crypto_session_token.is_empty(),
        "{}",
        client.status
    );
    assert!(
        private_session_scope(&client).is_complete(),
        "password change closed the session before references were durable"
    );
    let stored = client.state_json.clone();
    let state_path = client.state_path.clone();
    let owner = audit_owner(&client);
    drop(client);
    let loaded = load_state_json_with_store(&root, &state_path, &owner, now_millis()).unwrap();
    assert_eq!(loaded.value, stored);
    let reopened = test_client_for_account_scope(&root, "owner", loaded.value);
    assert!(private_session_scope(&reopened).is_complete());
    assert!(audit_plaintext_files(&root, "SESSION_EDIT_792").is_empty());
    drop(reopened);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn private_media_session_unlock_persists_exact_attachment_ids_for_one_account() {
    let root = temp_test_dir("private-session-unlock");
    let image = test_one_pixel_bmp();
    let (plain_state, attachment) = test_note_snapshot_with_media("private-image", &image);
    let plain = decode_data(&plain_state).notes[0].clone();
    let (sealed, setup_token) = gridtimer_native::encrypt_desktop_note_json(
        &serde_json::to_string(&plain).unwrap(),
        "unlock-reference-password",
    )
    .unwrap();
    gridtimer_native::close_desktop_note_session(&setup_token);
    let sealed_state =
        app_data::upsert_note_app_data_json(&plain_state, &sealed, now_millis()).unwrap();
    let mut client = test_client_for_account_scope(&root, "owner-a", sealed_state.clone());
    client.tab = AppTab::Notes;
    client.save_state().unwrap();
    let media = client.note_media_store().unwrap();
    write_test_scope_media_attachment(&media, &attachment, &image);
    assert!(!private_session_scope(&client).is_complete());
    client.select_note_by_id(&plain.id);
    client.note_crypto_password_draft = "unlock-reference-password".into();
    client.unlock_note_encryption();
    assert!(
        !client.note_crypto_session_token.is_empty(),
        "{}",
        client.status
    );
    client.lock_note_encryption();
    assert!(client.note_crypto_session_token.is_empty());
    let scope = private_session_scope(&client);
    assert!(
        scope.is_complete(),
        "unlock lost authenticated references when the session closed"
    );
    assert_eq!(scope.ids(), &HashSet::from([attachment.id.clone()]));
    client.cleanup_unreferenced_committed_note_media();
    assert_eq!(
        media
            .read_blob(&attachment.id, &attachment.sha256, attachment.size_bytes)
            .unwrap(),
        image
    );
    let other = test_client_for_account_scope(&root, "owner-b", sealed_state);
    other.save_state().unwrap();
    assert!(
        !private_session_scope(&other).is_complete(),
        "an account must not borrow another account's session declaration"
    );
    drop(other);
    drop(client);
    drop(media);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn private_media_session_password_change_isolates_other_account_with_same_note_id() {
    for separate_workspace in [false, true] {
        let root = temp_test_dir("private-session-account-isolation");
        let other_root = if separate_workspace {
            root.join("other-workspace")
        } else {
            root.clone()
        };
        fs::create_dir_all(&other_root).unwrap();
        let state = app_state_with_note("shared-note-id", "Private", "ACCOUNT_A_BODY", None);
        let mut other_state: Value = serde_json::from_str(&state).unwrap();
        other_state["notes"][0]["content"] = json!("ACCOUNT_B_BODY");
        other_state["notes"][0]["protectionStateRevision"] = json!(8);
        let mut a = test_client_for_account_scope(&root, "owner-a", state);
        let mut b = test_client_for_account_scope(
            &other_root,
            if separate_workspace {
                "owner-a"
            } else {
                "owner-b"
            },
            other_state.to_string(),
        );
        for client in [&mut a, &mut b] {
            client.tab = AppTab::Notes;
            client.save_state().unwrap();
            client.select_note_by_id("shared-note-id");
            client.note_crypto_password_draft = "account-local-password".into();
            client.enable_note_encryption();
            assert!(
                !client.note_crypto_session_token.is_empty(),
                "{}",
                client.status
            );
        }
        let old_a_token = a.note_crypto_session_token.clone();
        a.note_crypto_new_password_draft = "account-a-new-password".into();
        a.change_note_encryption_password();
        assert!(a.note_crypto_session_token.is_empty(), "{}", a.status);
        assert!(!gridtimer_native::close_desktop_note_session(&old_a_token));
        assert!(private_session_scope(&a).is_complete());
        let b_note = serde_json::to_string(&b.selected_note().unwrap()).unwrap();
        assert!(
            gridtimer_native::seal_desktop_note_json(&b_note, &b.note_crypto_session_token)
                .is_some(),
            "another account's same-ID note session was revoked"
        );
        b.set_note_canvas_text("ACCOUNT_B_UPDATED");
        b.submit_draft_snapshot(&egui::Context::default(), true);
        drain_draft_writer(&mut b);
        assert!(!b.note_dirty, "{}", b.status);
        assert!(private_session_scope(&b).is_complete());
        b.close_note_crypto_session();
        drop(a);
        drop(b);
        fs::remove_dir_all(root).unwrap();
    }
}
