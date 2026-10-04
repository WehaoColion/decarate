fn explicit_restore_old_head_fixture(
    root: &Path,
    floor: i64,
) -> (TimerWindowsClient, DesktopNoteAttachment, String, String) {
    let (plain, attachment) =
        test_note_snapshot_with_media("explicit-restore-file", &test_one_pixel_bmp());
    let captured =
        app_data::capture_note_revision_app_data_json(&plain, "note-with-media", 150).unwrap();
    let revision_id = decode_data(&captured).notes[0].revisions[0].id.clone();
    let mut client = test_client_for_account_scope(root, "restore-owner", captured.clone());
    client.save_state().unwrap();
    let deleted = app_data::delete_note_attachment_app_data_json(
        &captured,
        "note-with-media",
        &attachment.id,
        floor,
    )
    .unwrap();
    let owner = audit_owner(&client);
    save_state_snapshot_with_store(
        root,
        &client.state_path,
        &owner,
        &deleted,
        &client.sync,
        floor,
        "local_save",
    )
    .unwrap();
    let store = open_desktop_state_store(root).unwrap();
    let policy = store.privacy_policy(&owner).unwrap().0;
    let projected = policy.project_current_json(&captured).unwrap().unwrap();
    assert!(decode_data(&projected).notes[0].attachments.is_empty());
    assert!(decode_data(&projected).notes[0].updated_at_epoch_millis < floor);
    let protected = store
        .latest_valid(&owner, 0)
        .unwrap()
        .unwrap()
        .protected_sync_state;
    store
        .record_with_sync_state(
            &owner,
            &projected,
            &protected,
            now_millis(),
            "journal_recovery",
        )
        .unwrap();
    verify_and_refresh_desktop_state_evidence(root, &store).unwrap();
    client.state_json = projected;
    client.data = decode_data(&client.state_json);
    client.tab = AppTab::Notes;
    client.select_note_by_id("note-with-media");
    client.save_state().unwrap();
    (client, attachment, revision_id, captured)
}

#[test]
fn explicit_restore_old_projected_head_crosses_future_floor_in_plain_and_scoped_encrypted_note() {
    for encrypted in [false, true] {
        let root = temp_test_dir("explicit-restore-future-floor");
        let floor = now_millis().checked_add(10_000_000_000).unwrap();
        let (mut client, attachment, revision, _) = explicit_restore_old_head_fixture(&root, floor);
        if encrypted {
            // Imported sealed history is supported even though first-time UI
            // encryption correctly refuses notes whose images are still external.
            let plain = serde_json::to_string(&client.selected_note().unwrap()).unwrap();
            let scope = client.note_crypto_session_scope().unwrap();
            let (sealed, setup_token) = gridtimer_native::encrypt_desktop_note_json_in_scope(
                &plain,
                "explicit-restore-scoped-password",
                &scope,
            )
            .unwrap();
            assert!(gridtimer_native::close_desktop_note_session(&setup_token));
            assert!(
                client.replace_state(
                    app_data::upsert_note_app_data_json(&client.state_json, &sealed, now_millis()),
                    "Imported sealed restore fixture",
                ),
                "{}",
                client.status
            );
            client.select_note_by_id("note-with-media");
            assert!(client.selected_note_is_locked());
            client.note_crypto_password_draft = "explicit-restore-scoped-password".into();
            client.unlock_note_encryption();
            assert!(
                !client.note_crypto_session_token.is_empty(),
                "{}",
                client.status
            );
        }
        client.restore_note_revision("note-with-media", &revision);
        let restored = client.selected_note().unwrap();
        let actual = restored
            .attachments
            .iter()
            .find(|item| item.id == attachment.id)
            .unwrap_or_else(|| panic!("explicit restore lost its attachment: {}", client.status));
        assert!(
            actual.updated_at_epoch_millis > floor,
            "explicit restore used the stale system/head clock"
        );
        assert_eq!(actual.sha256, attachment.sha256);
        assert_eq!(actual.size_bytes, attachment.size_bytes);
        if encrypted {
            assert!(client.data.notes[0].encryption.is_some());
        }
        drop(client);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn explicit_restore_exhausted_floor_preserves_state_policy_and_unsaved_draft() {
    let root = temp_test_dir("explicit-restore-overflow");
    let (mut client, _, revision, _) = explicit_restore_old_head_fixture(&root, i64::MAX);
    let store = open_desktop_state_store(&root).unwrap();
    let policy = store.privacy_policy(&audit_owner(&client)).unwrap();
    let evidence = store.journal_evidence().unwrap();
    let state = client.state_json.clone();
    client.note_title_draft = "DRAFT_MUST_STAY_UNSAVED".into();
    client.note_dirty = true;
    client.restore_note_revision("note-with-media", &revision);
    assert_eq!(client.state_json, state);
    assert_eq!(client.note_title_draft, "DRAFT_MUST_STAY_UNSAVED");
    assert!(client.note_dirty);
    assert_eq!(store.privacy_policy(&audit_owner(&client)).unwrap(), policy);
    assert_eq!(store.journal_evidence().unwrap(), evidence);
    drop(store);
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ordinary_high_note_revision_does_not_restore_a_detached_attachment() {
    let root = temp_test_dir("ordinary-edit-is-not-explicit-restore");
    let floor = now_millis().checked_add(10_000_000_000).unwrap();
    let (mut client, attachment, _, captured) = explicit_restore_old_head_fixture(&root, floor);
    let mut incoming: Value = serde_json::from_str(&captured).unwrap();
    incoming["notes"][0]["title"] = json!("A high ordinary title edit");
    incoming["notes"][0]["updatedAtEpochMillis"] = json!(floor.checked_add(100).unwrap());
    assert!(client.replace_state(Some(incoming.to_string()), "ordinary edit"));
    assert!(!client.data.notes[0]
        .attachments
        .iter()
        .any(|item| item.id == attachment.id));
    drop(client);
    fs::remove_dir_all(root).unwrap();
}
