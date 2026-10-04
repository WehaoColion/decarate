// v0.0.3 - Retained history must not excuse orphan plaintext during encryption.
// v0.0.2 - Reject media mutations before opening storage in an unverified workspace.
// v0.0.1 - Exercise media retained only by local journal history or managed recovery files.
#[test]
fn private_recovery_proven_mirror_accepts_receipt_after_history_pruning() {
    let root = temp_test_dir("proven-mirror-receipt");
    let receiver_root = root.join("receiver");
    fs::create_dir_all(&receiver_root).unwrap();
    let mut source = test_client_for_account_scope(
        &root,
        "same-owner",
        app_state_with_note("mirror-sealed", "Private", "MIRROR_PRIVATE_CONTENT", None),
    );
    source.save_state().unwrap();
    source.tab = AppTab::Notes;
    source.select_note_by_id("mirror-sealed");
    source.note_crypto_password_draft = "mirror-proof-password".into();
    source.enable_note_encryption();
    source.close_note_crypto_session();
    let source_store = open_desktop_state_store(&root).unwrap();
    let metadata = source_store
        .privacy_policy(&audit_owner(&source))
        .unwrap()
        .0
        .private_media_queries(&source.state_json, true)
        .unwrap()
        .remove(0)
        .declaration
        .unwrap();
    let sealed = source.state_json.clone();
    let mut receiver = test_client_for_account_scope(&receiver_root, "same-owner", sealed.clone());
    receiver.save_state().unwrap();
    receiver.state_json = app_data::default_app_data_json(now_millis());
    receiver.data = decode_data(&receiver.state_json);
    receiver.save_state().unwrap(); // rotates the sealed head into a proven backup
    let store = open_desktop_state_store(&receiver_root).unwrap();
    let connection = rusqlite::Connection::open(store.database_path()).unwrap();
    connection
        .execute(
            "DELETE FROM desktop_state_snapshots WHERE owner=?1 AND raw_sha256=?2",
            rusqlite::params![
                audit_owner(&receiver),
                format!("{:x}", Sha256::digest(sealed.as_bytes()))
            ],
        )
        .unwrap();
    drop(connection);
    assert_eq!(
        fs::read_to_string(backup_path(&receiver.state_path)).unwrap(),
        sealed
    );
    let receipt = private_transport_test_receipt(&receiver, metadata);
    let before = receiver.state_json.clone();
    persist_private_media_replies(
        &receiver.ai_workspace_identity(),
        &before,
        &receiver.sync,
        &[receipt],
    )
    .unwrap();
    assert_eq!(receiver.state_json, before);
    let refs = store
        .private_media_references(&audit_owner(&receiver), &before)
        .unwrap();
    let policy = store.privacy_policy(&audit_owner(&receiver)).unwrap().0;
    assert!(
        refs.queries(&policy, true)
            .unwrap()
            .iter()
            .any(|query| query.declaration.is_some()),
        "a mirror-only ciphertext could not persist a verified receipt after pruning"
    );
    let evidence = store.journal_evidence().unwrap();
    receiver.save_state().unwrap();
    receiver.save_state().unwrap();
    assert_eq!(
        store.journal_evidence().unwrap(),
        evidence,
        "unchanged proven mirrors consumed unbounded metadata revisions"
    );
    drop(store);
    drop(source_store);
    drop(source);
    drop(receiver);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn private_recovery_proof_commit_failure_never_replaces_primary() {
    let root = temp_test_dir("proven-mirror-failed-commit");
    let mut client = test_client_for_account_scope(
        &root,
        "owner",
        app_state_with_note("private", "Before", "OLD_MIRROR", None),
    );
    client.save_state().unwrap();
    let original = fs::read_to_string(&client.state_path).unwrap();
    let store = open_desktop_state_store(&root).unwrap();
    let connection = rusqlite::Connection::open(store.database_path()).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_mirror_proof BEFORE INSERT ON desktop_state_mirror_provenance BEGIN SELECT RAISE(ABORT,'injected mirror proof failure'); END;").unwrap();
    client.state_json = app_state_with_note("private", "After", "NEW_MIRROR", None);
    client.data = decode_data(&client.state_json);
    let outcome = save_state_snapshot_with_store(
        &root,
        &client.state_path,
        &audit_owner(&client),
        &client.state_json,
        &client.sync,
        now_millis(),
        "local_save",
    )
    .unwrap();
    assert!(outcome.warning().is_some());
    assert_eq!(fs::read_to_string(&client.state_path).unwrap(), original);
    assert!(store
        .recovery_mirror_is_verified(&audit_owner(&client), &client.state_path, &original)
        .unwrap());
    drop(connection);
    drop(store);
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn private_recovery_failed_rename_retains_previous_and_prepared_bytes() {
    use std::os::windows::fs::OpenOptionsExt;
    let root = temp_test_dir("proven-mirror-failed-rename");
    let client = test_client_for_account_scope(
        &root,
        "owner",
        app_state_with_note("private", "Before", "OLD_RENAME", None),
    );
    client.save_state().unwrap();
    let old = fs::read_to_string(&client.state_path).unwrap();
    let new = app_state_with_note("private", "After", "NEW_RENAME", None);
    let owner = audit_owner(&client);
    let store = open_desktop_state_store(&root).unwrap();
    store
        .record(&owner, &new, now_millis(), "local_save")
        .unwrap();
    verify_and_refresh_desktop_state_evidence(&root, &store).unwrap();
    let deny_rename = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&client.state_path)
        .unwrap();
    let writer = store.lock_recovery_mirror_writer().unwrap();
    assert!(write_proven_desktop_mirror(
        &root,
        &store,
        &writer,
        &owner,
        &client.state_path,
        &client.state_path,
        &new,
        None,
        true
    )
    .is_err());
    assert_eq!(fs::read_to_string(&client.state_path).unwrap(), old);
    assert!(store
        .recovery_mirror_is_verified(&owner, &client.state_path, &old)
        .unwrap());
    let mut prepared_found = false;
    visit_desktop_recovery_mirrors(&client.state_path, |path, raw| {
        if raw == new && path != client.state_path {
            prepared_found = store
                .recovery_mirror_is_verified(&owner, path, raw)
                .unwrap();
        }
        Ok(())
    })
    .unwrap();
    assert!(
        prepared_found,
        "failed rename lost its fully written recovery candidate"
    );
    drop(writer);
    drop(deny_rename);
    drop(store);
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn private_recovery_mirrors_enumerate_all_managed_sources_without_path_authority() {
    use gridtimer_native::desktop_private_media_index::DesktopPrivateMediaReferences;
    let root = temp_test_dir("private-mirror-enumeration");
    let (plain, _) = test_note_snapshot_with_media("mirror-proof-file", &test_one_pixel_bmp());
    let sealed = local_reference_seal_snapshot(&plain);
    let client = test_client_for_account_scope(&root, "mirror-owner", sealed.clone());
    client.save_state().unwrap();
    let owner = audit_owner(&client);
    let store = open_desktop_state_store(&root).unwrap();
    let current = app_data::default_app_data_json(300);
    let name = client.state_path.file_name().unwrap().to_str().unwrap();
    let paths = ["", ".bak", ".invalid-reference-test", ".tmp-reference-test"]
        .map(|suffix| client.state_path.with_file_name(format!("{name}{suffix}")));
    for path in &paths {
        fs::write(path, &sealed).unwrap();
    }
    let unrelated = client.state_path.with_file_name("other-account.json.bak");
    fs::write(&unrelated, "{").unwrap();
    let mut proven = DesktopPrivateMediaReferences::from_snapshot(&current).unwrap();
    include_desktop_private_recovery_mirrors(&store, &owner, &client.state_path, &mut proven)
        .unwrap();
    assert_eq!(proven.queries(&Default::default(), false).unwrap().len(), 1);
    assert_eq!(proven.unverified_recovery_sources(), 0);

    // The same four managed names, with no exact byte proof, grant no queries.
    for path in &paths {
        fs::write(path, format!("{sealed} ")).unwrap();
    }
    let mut unknown = DesktopPrivateMediaReferences::from_snapshot(&current).unwrap();
    include_desktop_private_recovery_mirrors(&store, &owner, &client.state_path, &mut unknown)
        .unwrap();
    assert_eq!(unknown.unverified_recovery_sources(), 4);
    assert!(unknown
        .queries(&Default::default(), false)
        .unwrap()
        .is_empty());
    assert!(!unknown.scope(&Default::default()).is_complete());
    for path in &paths {
        assert!(path.exists());
    }

    // A broken source does not stop the remaining verified mirrors being read.
    fs::write(&paths[0], "{").unwrap();
    for path in &paths[1..] {
        fs::write(path, &sealed).unwrap();
    }
    let mut mixed = DesktopPrivateMediaReferences::from_snapshot(&current).unwrap();
    include_desktop_private_recovery_mirrors(&store, &owner, &client.state_path, &mut mixed)
        .unwrap();
    assert_eq!(mixed.unverified_recovery_sources(), 1);
    assert_eq!(mixed.queries(&Default::default(), false).unwrap().len(), 1);
    drop(store);
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn local_recovery_media_old_plaintext_files_still_block_encryption() {
    let root = temp_test_dir("recovery-media-old-plaintext");
    let image = test_one_pixel_bmp();
    let (snapshot, attachment) = test_note_snapshot_with_media("old-plaintext-image", &image);
    let mut client = test_client_for_account_scope(&root, "history-owner", snapshot);
    client.save_state().unwrap();
    let media = client.note_media_store().unwrap();
    write_test_scope_media_attachment(&media, &attachment, &image);
    let pure = app_state_with_note("note-with-media", "Now plain text", "text", None);
    assert!(client.replace_state(Some(pure), "test"));
    client.select_note_by_id("note-with-media");
    assert!(client.selected_note().unwrap().attachments.is_empty());
    client.note_crypto_password_draft = "history-plaintext-test-password".into();
    client.enable_note_encryption();
    assert!(
        client
            .data
            .notes
            .iter()
            .find(|note| note.id == "note-with-media")
            .unwrap()
            .encryption
            .is_none(),
        "history-only plaintext must block encryption until explicitly resolved"
    );
    assert!(media.blob_exists(&attachment.id).unwrap());
    drop(client);
    drop(media);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn local_recovery_media_readonly_workspace_does_not_initialize_or_clean_storage() {
    for encrypt in [false, true] {
        let root = temp_test_dir("recovery-media-readonly");
        let snapshot = app_state_with_note("readonly-note", "Note", "content", None);
        let mut client = test_client_for_account_scope(&root, "readonly-owner", snapshot);
        client.tab = AppTab::Notes;
        client.select_note_by_id("readonly-note");
        client.workspace_persistence_ready = false;
        let before = client.state_json.clone();
        let media = client.note_media_store().unwrap();
        let image = test_one_pixel_bmp();
        let (_, attachment) = test_note_snapshot_with_media("readonly-orphan", &image);
        write_test_scope_media_attachment(&media, &attachment, &image);
        let source = root.join("incoming.bmp");
        fs::write(&source, &image).unwrap();
        assert!(!desktop_state_store_path(&root).exists());
        if encrypt {
            client.note_crypto_password_draft = "readonly-test-password".into();
            client.enable_note_encryption();
        } else {
            client.import_note_image_path(&source);
        }
        assert!(
            !desktop_state_store_path(&root).exists(),
            "a read-only workspace must not initialize the journal through media actions"
        );
        assert!(media.blob_exists(&attachment.id).unwrap());
        assert_eq!(client.state_json, before);
        drop(client);
        drop(media);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn local_recovery_media_journal_keeps_bytes_for_real_fallback() {
    let root = temp_test_dir("recovery-media-journal");
    let image = test_one_pixel_bmp();
    let (snapshot, attachment) = test_note_snapshot_with_media("journal-only-image", &image);
    let mut client = test_client_for_account_scope(&root, "history-owner", snapshot.clone());
    client.save_state().unwrap();
    let media = client.note_media_store().unwrap();
    write_test_scope_media_attachment(&media, &attachment, &image);
    let empty = app_data::default_app_data_json(300);
    assert!(client.replace_state(Some(empty.clone()), "test"));
    atomic_replace_text_no_backup(&backup_path(&client.state_path), &empty).unwrap();
    let owner = audit_owner(&client);
    let connection = rusqlite::Connection::open(desktop_state_store_path(&root)).unwrap();
    let retained: i64 = connection
        .query_row(
            "SELECT count(*) FROM desktop_state_snapshots WHERE owner=?1 AND app_data_json=?2",
            rusqlite::params![owner, snapshot],
            |row| row.get(0),
        )
        .unwrap();
    assert!(retained > 0);
    client.cleanup_unreferenced_committed_note_media();
    assert!(
        media.blob_exists(&attachment.id).unwrap(),
        "cleanup erased media needed only by a retained journal snapshot"
    );

    // Corrupt the newest state and use the actual recovery entry point.
    connection.execute(
        "UPDATE desktop_state_snapshots SET raw_sha256=?1 WHERE id=(SELECT max(id) FROM desktop_state_snapshots WHERE owner=?2)",
        rusqlite::params!["0".repeat(64), owner],
    ).unwrap();
    drop(connection);
    let recovered =
        load_state_json_with_store(&root, &client.state_path, &owner, now_millis()).unwrap();
    assert!(recovered.value.contains(&attachment.id));
    assert_eq!(
        media
            .read_blob(&attachment.id, &attachment.sha256, attachment.size_bytes)
            .unwrap(),
        image
    );
    drop(client);
    drop(media);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn local_recovery_media_managed_mirrors_keep_bytes_until_removed() {
    let image = test_one_pixel_bmp();
    let (snapshot, attachment) = test_note_snapshot_with_media("mirror-only-image", &image);
    for suffix in [".bak", ".invalid-reference-test", ".tmp-reference-test"] {
        let root = temp_test_dir("recovery-media-mirror");
        let mut client = test_client_for_account_scope(
            &root,
            "mirror-owner",
            app_data::default_app_data_json(100),
        );
        client.save_state().unwrap();
        let media = client.note_media_store().unwrap();
        write_test_scope_media_attachment(&media, &attachment, &image);
        let name = client.state_path.file_name().unwrap().to_str().unwrap();
        let mirror = client.state_path.with_file_name(format!("{name}{suffix}"));
        fs::write(&mirror, &snapshot).unwrap();
        client.recover_workspace_note_media();
        assert!(
            media.blob_exists(&attachment.id).unwrap(),
            "{suffix}: cleanup erased a retained recovery attachment"
        );
        fs::remove_file(&mirror).unwrap();
        client.cleanup_unreferenced_committed_note_media();
        assert!(
            !media.blob_exists(&attachment.id).unwrap(),
            "unreferenced bytes should be cleaned after the last recovery copy is gone"
        );
        drop(client);
        drop(media);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn local_recovery_media_permanent_delete_releases_only_its_own_account() {
    let root = temp_test_dir("recovery-media-permanent-delete");
    let image = test_one_pixel_bmp();
    let (snapshot, attachment) = test_note_snapshot_with_media("shared-id-image", &image);
    let mut first = test_client_for_account_scope(&root, "history-owner-a", snapshot.clone());
    first.save_state().unwrap();
    let first_media = first.note_media_store().unwrap();
    write_test_scope_media_attachment(&first_media, &attachment, &image);
    let second = test_client_for_account_scope(&root, "history-owner-b", snapshot);
    second.save_state().unwrap();
    let second_media = second.note_media_store().unwrap();
    write_test_scope_media_attachment(&second_media, &attachment, &image);
    let deleted = app_data::delete_note_permanently_app_data_json(
        &first.state_json,
        "note-with-media",
        now_millis(),
    )
    .unwrap();
    assert!(first.replace_state(Some(deleted), "test"));
    first.recover_workspace_note_media();
    assert!(!first_media.blob_exists(&attachment.id).unwrap());
    assert_eq!(
        second_media
            .read_blob(&attachment.id, &attachment.sha256, attachment.size_bytes)
            .unwrap(),
        image
    );
    drop(first);
    drop(second);
    drop(first_media);
    drop(second_media);
    fs::remove_dir_all(root).unwrap();
}

// v0.0.1 - Verify unknown recovery files and release reservations before normal mutations.
#[test]
fn local_recovery_media_invalid_mirror_preserves_files_until_resolved() {
    let root = temp_test_dir("recovery-media-unknown-mirror");
    let image = test_one_pixel_bmp();
    let (_, attachment) = test_note_snapshot_with_media("unknown-mirror-file", &image);
    let mut client =
        test_client_for_account_scope(&root, "mirror-owner", app_data::default_app_data_json(100));
    client.save_state().unwrap();
    let media = client.note_media_store().unwrap();
    write_test_scope_media_attachment(&media, &attachment, &image);
    let name = client.state_path.file_name().unwrap().to_str().unwrap();
    let mirror = client
        .state_path
        .with_file_name(format!("{name}.invalid-incomplete"));
    fs::write(&mirror, "{").unwrap();
    client.recover_workspace_note_media();
    assert!(media.blob_exists(&attachment.id).unwrap());
    fs::remove_file(&mirror).unwrap();
    client.recover_workspace_note_media();
    assert!(!media.blob_exists(&attachment.id).unwrap());
    drop(client);
    drop(media);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn local_recovery_media_reservation_is_released_before_import_and_encryption_commit() {
    let root = temp_test_dir("recovery-media-import-save");
    let snapshot = app_state_with_note("note-image", "Picture", "", None);
    let snapshot = app_data::upsert_note_app_data_json(&snapshot,
        &serde_json::json!({"id":"note-secret","title":"Private","content":"private test text","updatedAtEpochMillis":100}).to_string(),100).unwrap();
    let mut client = test_client_for_account_scope(&root, "import-owner", snapshot);
    client.save_state().unwrap();
    client.tab = AppTab::Notes;
    client.select_note_by_id("note-image");
    let source = root.join("image.bmp");
    fs::write(&source, test_one_pixel_bmp()).unwrap();
    client.import_note_image_path(&source);
    let note = client
        .data
        .notes
        .iter()
        .find(|note| note.id == "note-image")
        .unwrap();
    assert_eq!(
        note.attachments.len(),
        1,
        "image import failed to commit its reference: {}",
        client.status
    );
    assert!(client
        .note_media_store()
        .unwrap()
        .blob_exists(&note.attachments[0].id)
        .unwrap());
    client.select_note_by_id("note-secret");
    client.note_crypto_password_draft = "retention-guard-test-password".into();
    client.enable_note_encryption();
    assert!(
        client
            .data
            .notes
            .iter()
            .find(|note| note.id == "note-secret")
            .unwrap()
            .encryption
            .is_some(),
        "encryption failed to commit: {}",
        client.status
    );
    drop(client);
    fs::remove_dir_all(root).unwrap();
}
