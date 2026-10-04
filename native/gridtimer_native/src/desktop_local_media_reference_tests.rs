// v0.0.1 - Exercise real desktop recovery, cleanup replay and account media staging.
fn local_reference_seal_snapshot(snapshot: &str) -> String {
    let mut value: Value = serde_json::from_str(snapshot).unwrap();
    let (sealed, token) = gridtimer_native::encrypt_desktop_note_json(
        &value["notes"][0].to_string(),
        "local-media-test-password",
    )
    .unwrap();
    assert!(gridtimer_native::close_desktop_note_session(&token));
    value["notes"][0] = serde_json::from_str(&sealed).unwrap();
    value.to_string()
}

fn local_reference_conflict_snapshot(snapshot: &str) -> String {
    let mut value: Value = serde_json::from_str(snapshot).unwrap();
    let note = value["notes"][0].take();
    value["notes"] = serde_json::json!([]);
    value["syncConflictHistory"] = serde_json::json!([{
        "id": "local-reference-conflict",
        "entityType": "note",
        "entityId": note["id"],
        "losingRevisionEpochMillis": 100,
        "capturedAtEpochMillis": 200,
        "payload": note
    }]);
    value.to_string()
}

#[test]
fn local_media_reference_restart_preserves_sealed_and_conflict_files() {
    let image = test_one_pixel_bmp();
    let (plain, attachment) = test_note_snapshot_with_media("local-shared-image", &image);
    let sealed = local_reference_seal_snapshot(&plain);
    for (label, snapshot) in [
        ("sealed", sealed.clone()),
        ("plain-conflict", local_reference_conflict_snapshot(&plain)),
        (
            "sealed-conflict",
            local_reference_conflict_snapshot(&sealed),
        ),
    ] {
        let root = temp_test_dir(&format!("local-reference-{label}"));
        let mut client = test_client_for_account_scope(&root, "local-owner", snapshot);
        let store = client.note_media_store().unwrap();
        write_test_scope_media_attachment(&store, &attachment, &image);
        client.recover_workspace_note_media();
        assert!(
            store.blob_exists(&attachment.id).unwrap(),
            "{label}: restart erased retained media"
        );
        client.cleanup_unreferenced_committed_note_media();
        assert_eq!(
            store
                .read_blob(&attachment.id, &attachment.sha256, attachment.size_bytes)
                .unwrap(),
            image
        );
        drop(client);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn local_media_reference_replayed_cleanup_rechecks_current_references() {
    let root = temp_test_dir("local-reference-cleanup-replay");
    let image = test_one_pixel_bmp();
    let (snapshot, attachment) = test_note_snapshot_with_media("local-referenced-again", &image);
    let mut client = test_client_for_account_scope(&root, "local-owner", snapshot);
    let store = client.note_media_store().unwrap();
    write_test_scope_media_attachment(&store, &attachment, &image);
    let marker = store
        .root()
        .join(".pending_imports_v1")
        .join(format!(".cleanup_block_{}.json", attachment.id));
    fs::write(
        &marker,
        serde_json::json!({
            "stateVersion": 1, "attachmentId": attachment.id,
            "reason": "unreferenced-committed", "createdAtEpochMillis": 100
        })
        .to_string(),
    )
    .unwrap();
    client.recover_workspace_note_media();
    assert!(
        store.blob_exists(&attachment.id).unwrap(),
        "old cleanup marker erased a current reference"
    );
    assert!(
        marker.exists(),
        "retain retry evidence until references permit cleanup"
    );

    // An explicit permanent deletion removes retained recovery references and permits retry.
    let deleted = app_data::delete_note_permanently_app_data_json(
        &client.state_json,
        "note-with-media",
        now_millis(),
    )
    .unwrap();
    assert!(client.replace_state(Some(deleted), "test"));
    client.recover_workspace_note_media();
    assert!(!store.blob_exists(&attachment.id).unwrap());
    assert!(!marker.exists());
    drop(client);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn local_media_reference_opaque_pending_import_survives_restart() {
    let root = temp_test_dir("local-reference-pending");
    let image = test_one_pixel_bmp();
    let mut client =
        test_client_for_account_scope(&root, "local-owner", app_data::default_app_data_json(100));
    let store = client.note_media_store().unwrap();
    let source = root.join("image.bmp");
    fs::write(&source, &image).unwrap();
    let pending = store.begin_import(&source).unwrap();
    let (snapshot, _) = test_note_snapshot_with_media(pending.attachment_id(), &image);
    let snapshot = local_reference_seal_snapshot(&snapshot);
    assert!(client.replace_state(Some(snapshot), "test"));
    client.recover_workspace_note_media();
    store
        .commit_import(&pending)
        .expect("opaque references must not erase an unfinished import");
    assert!(store.blob_exists(pending.attachment_id()).unwrap());
    drop(client);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn local_media_reference_scope_migration_refuses_unresolved_ciphertext_before_copy() {
    let root = temp_test_dir("local-reference-scope");
    let image = test_one_pixel_bmp();
    let (snapshot, attachment) = test_note_snapshot_with_media("local-scope-image", &image);
    let snapshot = local_reference_seal_snapshot(&snapshot);
    let (source, target, owner) = test_scope_media_workspaces(&root);
    let source_store = note_media_store_for_workspace(&source).unwrap();
    write_test_scope_media_attachment(&source_store, &attachment, &image);
    let result = stage_scope_media_transaction(&root, &owner, &snapshot, &source, &target);
    assert!(
        result.is_err(),
        "unknown sealed references must not commit an empty media migration"
    );
    assert!(source_store.blob_exists(&attachment.id).unwrap());
    assert!(probe_existing_scope_note_media_store(&target)
        .unwrap()
        .is_none());
    assert!(!scope_media_transaction_base(&root).exists());
    drop(source_store);
    fs::remove_dir_all(root).unwrap();
}

// v0.0.1 - Verify complete conflict references copy only required files across accounts.
#[test]
fn local_media_reference_conflict_migration_recovers_only_referenced_files() {
    let root = temp_test_dir("local-reference-conflict-migration");
    let image = test_one_pixel_bmp();
    let (snapshot, attachment) = test_note_snapshot_with_media("conflict-media-image", &image);
    let snapshot = local_reference_conflict_snapshot(&snapshot);
    let (source, target, owner) = test_scope_media_workspaces(&root);
    let source_store = note_media_store_for_workspace(&source).unwrap();
    write_test_scope_media_attachment(&source_store, &attachment, &image);
    let unrelated = DesktopNoteAttachment {
        id: "other-account-private-file".into(),
        ..attachment.clone()
    };
    write_test_scope_media_attachment(&source_store, &unrelated, &image);
    let transaction = stage_scope_media_transaction(&root, &owner, &snapshot, &source, &target)
        .unwrap()
        .expect("conflict-only references require media migration");
    open_desktop_state_store(&root)
        .unwrap()
        .record(
            &owner,
            &snapshot,
            100,
            &scope_media_transaction_source(&transaction.manifest.tx_id),
        )
        .unwrap();
    recover_scope_media_transactions(&root).unwrap();
    let target_store = note_media_store_for_workspace(&target).unwrap();
    assert_eq!(
        target_store
            .read_blob(&attachment.id, &attachment.sha256, attachment.size_bytes)
            .unwrap(),
        image
    );
    assert!(!target_store.blob_exists(&unrelated.id).unwrap());
    assert!(source_store.blob_exists(&unrelated.id).unwrap());
    assert!(!transaction.directory.exists());
    drop(source_store);
    drop(target_store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn local_media_reference_unknown_conflict_shape_prevents_migration() {
    let root = temp_test_dir("local-reference-unknown-conflict");
    let image = test_one_pixel_bmp();
    let (snapshot, attachment) = test_note_snapshot_with_media("unknown-conflict-image", &image);
    let mut snapshot: Value =
        serde_json::from_str(&local_reference_conflict_snapshot(&snapshot)).unwrap();
    snapshot["syncConflictHistory"][0]["payload"]["futureAttachments"] =
        serde_json::json!(["hidden"]);
    let (source, target, owner) = test_scope_media_workspaces(&root);
    let source_store = note_media_store_for_workspace(&source).unwrap();
    write_test_scope_media_attachment(&source_store, &attachment, &image);
    assert!(
        stage_scope_media_transaction(&root, &owner, &snapshot.to_string(), &source, &target)
            .is_err()
    );
    assert!(source_store.blob_exists(&attachment.id).unwrap());
    assert!(probe_existing_scope_note_media_store(&target)
        .unwrap()
        .is_none());
    assert!(!scope_media_transaction_base(&root).exists());
    drop(source_store);
    fs::remove_dir_all(root).unwrap();
}
