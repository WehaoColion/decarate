// v0.0.5 - Exercise compact history references through the native sync and receipt paths.
// v0.0.4 - Verify publication requirements separately from recovery attachment work.
// v0.0.3 - Verify journal-only receipt persistence without restoring old note content.
// v0.0.2 - Keep retained receipt origins explicit in the native transport fixture.
// v0.0.1 - Verify network receipts through the real desktop journal and completion path.
fn private_transport_test_receipt(
    client: &TimerWindowsClient,
    declaration: Value,
) -> gridtimer_native::private_media_protocol::VerifiedPrivateMediaReply {
    use gridtimer_native::private_media_protocol::{
        PrivateMediaQuery, PrivateMediaReply, PrivateMediaReplyEntry,
    };
    use std::io::{BufRead, Read, Write};
    let value: Value = serde_json::from_str(&client.state_json).unwrap();
    let fingerprint = declaration["envelopeSha256"].as_str().unwrap();
    let reference_snapshot =
        open_desktop_state_store(&client.ai_workspace_identity().namespace_root)
            .unwrap()
            .private_media_references(&audit_owner(client), &client.state_json)
            .unwrap();
    let query = reference_snapshot
        .queries(
            &gridtimer_native::desktop_state_store::DesktopPrivacyPolicy::default(),
            false,
        )
        .unwrap()
        .into_iter()
        .find(|query| query.envelope_sha256 == fingerprint)
        .unwrap();
    let current_head_matches = value["notes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(PrivateMediaQuery::for_note)
        .any(|head| head.envelope_sha256 == fingerprint);
    let user = client.sync.user_id.clone();
    let server_id = client.sync.server_instance_id.clone();
    let namespace = client.sync.account_namespace.clone();
    let token_id = sync_core::token_identifier(&client.sync.token);
    let generation = client.sync.acknowledged_generation;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut reader = std::io::BufReader::new(&mut stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(line.starts_with("POST /v1/media/private-references "));
        let mut length = 0;
        loop {
            line.clear();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse::<usize>().unwrap();
            }
        }
        assert!(length > 0 && length < 65536);
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        drop(reader);
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert!(body["queries"][0].get("declaration").is_none());
        let query: PrivateMediaQuery = serde_json::from_value(body["queries"][0].clone()).unwrap();
        let reply = sync_core::SyncClientResult {
            ok: true,
            user_id: user,
            server_instance_id: server_id,
            account_namespace: namespace,
            token_id,
            current_generation: generation,
            mode: "private_media_exchange".into(),
            private_media_exchange: Some(PrivateMediaReply {
                format_version: 1,
                request_id: body["requestId"].as_str().unwrap().into(),
                generation,
                entries: vec![PrivateMediaReplyEntry {
                    note_id: query.note_id,
                    envelope_sha256: query.envelope_sha256,
                    accepted: false,
                    current_head_matches,
                    retained_note_matches: !current_head_matches,
                    declaration: Some(declaration),
                }],
            }),
            ..Default::default()
        };
        let body = serde_json::to_vec(&reply).unwrap();
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
        stream.write_all(&body).unwrap();
    });
    let outcome = sync_core::desktop_private_media_exchange(
        &url,
        &client.sync.user_id,
        &client.sync.token,
        &client.sync.server_instance_id,
        &client.sync.account_namespace,
        generation,
        "",
        vec![query],
    );
    worker.join().unwrap();
    assert!(outcome.result.ok, "{}", outcome.result.message);
    outcome.verified.unwrap()
}

#[test]
fn private_media_transport_declaration_cannot_cross_owner_with_same_ciphertext() {
    use gridtimer_native::desktop_state_store::DesktopSealedMediaDeclaration;
    let root = temp_test_dir("private-transport-store-owner");
    let mut source = test_client_for_account_scope(
        &root,
        "receipt-source",
        app_state_with_note("same-ciphertext", "Secret", "RECEIPT_BODY", None),
    );
    source.save_state().unwrap();
    source.tab = AppTab::Notes;
    source.select_note_by_id("same-ciphertext");
    source.note_crypto_password_draft = "receipt-owner-password".into();
    source.enable_note_encryption();
    source.close_note_crypto_session();
    let store = open_desktop_state_store(&root).unwrap();
    let owner_a = audit_owner(&source);
    let metadata = store
        .privacy_policy(&owner_a)
        .unwrap()
        .0
        .private_media_queries(&source.state_json, true)
        .unwrap()
        .remove(0)
        .declaration
        .unwrap();
    let receipt = private_transport_test_receipt(&source, metadata);
    let references = store
        .private_media_references(&owner_a, &source.state_json)
        .unwrap();
    let current_declarations = DesktopSealedMediaDeclaration::from_verified_replies(
        &[receipt.clone()],
        &source.state_json,
        &source.sync.user_id,
        &source.sync.token,
        &source.sync.server_instance_id,
        &source.sync.account_namespace,
        source.sync.acknowledged_generation,
    )
    .unwrap();
    let retained_declarations = DesktopSealedMediaDeclaration::from_verified_retained_replies(
        &[receipt],
        &references,
        &source.sync.user_id,
        &source.sync.token,
        &source.sync.server_instance_id,
        &source.sync.account_namespace,
        source.sync.acknowledged_generation,
    )
    .unwrap();
    let owner_b = desktop_state_owner(
        &source.sync.server_instance_id,
        &sync_core::account_namespace_identifier(&source.sync.server_instance_id, "receipt-other"),
        "receipt-other",
    )
    .unwrap();
    for declarations in [current_declarations, retained_declarations] {
        assert_eq!(declarations.len(), 1);
        store
            .record_with_sync_state_and_media_declarations(
                &owner_a,
                &source.state_json,
                b"source-sync",
                200,
                "private_media_sync",
                &declarations,
            )
            .unwrap();
        for owner in [owner_b.as_str(), "guest-v1"] {
            store
                .record_with_sync_state(
                    owner,
                    &source.state_json,
                    b"target-sync",
                    200,
                    "local_save",
                )
                .unwrap();
            let policy = store.privacy_policy(owner).unwrap();
            let evidence = store.journal_evidence().unwrap();
            let before = store.latest_valid(owner, 0).unwrap().unwrap();
            assert!(
                store
                    .record_with_sync_state_and_media_declarations(
                        owner,
                        &source.state_json,
                        b"must-not-commit",
                        300,
                        "private_media_sync",
                        &declarations,
                    )
                    .is_err(),
                "a verified source-account receipt authorized another owner"
            );
            assert_eq!(store.privacy_policy(owner).unwrap(), policy);
            assert_eq!(store.journal_evidence().unwrap(), evidence);
            let after = store.latest_valid(owner, 0).unwrap().unwrap();
            assert_eq!(after.id, before.id);
            assert_eq!(after.protected_sync_state, before.protected_sync_state);
        }
    }
    drop(store);
    drop(source);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn private_media_transport_desktop_receipt_is_durable_idempotent_and_account_bound() {
    let root = temp_test_dir("private-transport-native");
    let receiver_root = root.join("receiver");
    fs::create_dir_all(&receiver_root).unwrap();
    let mut source = test_client_for_account_scope(
        &root,
        "private-owner",
        app_state_with_note("private-transfer", "Secret", "PRIVATE_CONTENT", None),
    );
    source.save_state().unwrap();
    source.tab = AppTab::Notes;
    source.select_note_by_id("private-transfer");
    source.note_crypto_password_draft = "transport-password".into();
    source.enable_note_encryption();
    source.close_note_crypto_session();
    let source_store = open_desktop_state_store(&root).unwrap();
    let policy = source_store
        .privacy_policy(&audit_owner(&source))
        .unwrap()
        .0;
    let query = policy
        .private_media_queries(&source.state_json, true)
        .unwrap()
        .remove(0);
    let declaration = query.declaration.unwrap();
    let mut receiver =
        test_client_for_account_scope(&receiver_root, "private-owner", source.state_json.clone());
    receiver.save_state().unwrap();
    assert!(!private_session_scope(&receiver).is_complete());
    let receipt = private_transport_test_receipt(&receiver, declaration);
    let workspace = receiver.ai_workspace_identity();
    let before = receiver.state_json.clone();
    let mut other_sync = receiver.sync.clone();
    other_sync.user_id = "other-owner".into();
    assert!(
        persist_private_media_replies(&workspace, &before, &other_sync, &[receipt.clone()])
            .is_err()
    );
    let mut stale_sync = receiver.sync.clone();
    stale_sync.acknowledged_generation += 1;
    assert!(
        persist_private_media_replies(&workspace, &before, &stale_sync, &[receipt.clone()])
            .is_err()
    );
    assert!(!private_session_scope(&receiver).is_complete());
    receiver.finish_note_media_sync_task(NoteMediaSyncTaskResult {
        task: SyncTaskState {
            kind: SyncTaskKind::Sync,
            phase: SyncTaskPhase::NoteMedia,
            started_at_epoch_millis: now_millis(),
        },
        background_job_id: None,
        origin_workspace: workspace.clone(),
        base_status: "sync".into(),
        result: Ok(
            gridtimer_native::desktop_note_media_sync::DesktopNoteMediaSyncSummary {
                verified_private_media: vec![receipt.clone()],
                ..Default::default()
            },
        ),
    });
    assert_eq!(receiver.state_json, before);
    assert!(
        private_session_scope(&receiver).is_complete(),
        "{}",
        receiver.status
    );
    let store = open_desktop_state_store(&receiver_root).unwrap();
    let evidence = store.journal_evidence().unwrap();
    persist_private_media_replies(&workspace, &before, &receiver.sync, &[receipt]).unwrap();
    assert_eq!(
        store.journal_evidence().unwrap(),
        evidence,
        "repeated replies must not rewrite the snapshot or evidence"
    );
    drop(store);
    drop(source_store);
    drop(source);
    drop(receiver);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn private_history_transport_native_conflict_receipt_survives_reopen_without_changing_notes() {
    let root = temp_test_dir("private-history-native");
    let source_root = root.join("source");
    let receiver_root = root.join("receiver");
    fs::create_dir_all(&source_root).unwrap();
    fs::create_dir_all(&receiver_root).unwrap();
    let mut source = test_client_for_account_scope(
        &source_root,
        "history-owner",
        app_state_with_note(
            "historical-note",
            "Secret",
            "RETAINED_PRIVATE_CONTENT",
            None,
        ),
    );
    source.save_state().unwrap();
    source.tab = AppTab::Notes;
    source.select_note_by_id("historical-note");
    source.note_crypto_password_draft = "history-password".into();
    source.enable_note_encryption();
    source.close_note_crypto_session();
    let store = open_desktop_state_store(&source_root).unwrap();
    let declaration = store
        .privacy_policy(&audit_owner(&source))
        .unwrap()
        .0
        .private_media_queries(&source.state_json, true)
        .unwrap()
        .remove(0)
        .declaration
        .unwrap();
    let mut state: Value = serde_json::from_str(&source.state_json).unwrap();
    let old = state["notes"][0].clone();
    state["notes"] = json!([]);
    state["syncConflictHistory"] = json!([{"id":"native-retained-conflict","entityType":"note",
        "entityId":"historical-note","losingRevisionEpochMillis":100,"capturedAtEpochMillis":300,"payload":old}]);
    let mut receiver =
        test_client_for_account_scope(&receiver_root, "history-owner", state.to_string());
    receiver.save_state().unwrap();
    assert!(!private_session_scope(&receiver).is_complete());
    let receipt = private_transport_test_receipt(&receiver, declaration);
    let before = receiver.state_json.clone();
    let workspace = receiver.ai_workspace_identity();
    persist_private_media_replies(&workspace, &before, &receiver.sync, &[receipt.clone()]).unwrap();
    assert_eq!(receiver.state_json, before);
    assert!(private_session_scope(&receiver).is_complete());
    let receiver_store = open_desktop_state_store(&receiver_root).unwrap();
    let evidence = receiver_store.journal_evidence().unwrap();
    drop(receiver_store);
    persist_private_media_replies(&workspace, &before, &receiver.sync, &[receipt]).unwrap();
    let reopened = open_desktop_state_store(&receiver_root).unwrap();
    assert_eq!(reopened.journal_evidence().unwrap(), evidence);
    assert!(
        gridtimer_native::desktop_media_references::DesktopMediaReferenceScope::from_snapshot(
            &before,
            &reopened.privacy_policy(&audit_owner(&receiver)).unwrap().0
        )
        .is_complete()
    );
    drop(reopened);
    drop(store);
    drop(source);
    drop(receiver);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn private_recovery_transport_native_receipt_survives_reopen_without_changing_current_snapshot() {
    let root = temp_test_dir("private-recovery-native");
    let source_root = root.join("source");
    let receiver_root = root.join("receiver");
    fs::create_dir_all(&source_root).unwrap();
    fs::create_dir_all(&receiver_root).unwrap();
    let mut source = test_client_for_account_scope(
        &source_root,
        "recovery-owner",
        app_state_with_note("retained-note", "Secret", "RECOVERY_ONLY_CONTENT", None),
    );
    source.save_state().unwrap();
    source.tab = AppTab::Notes;
    source.select_note_by_id("retained-note");
    source.note_crypto_password_draft = "recovery-password".into();
    source.enable_note_encryption();
    source.close_note_crypto_session();
    let source_store = open_desktop_state_store(&source_root).unwrap();
    let declaration = source_store
        .privacy_policy(&audit_owner(&source))
        .unwrap()
        .0
        .private_media_queries(&source.state_json, true)
        .unwrap()
        .remove(0)
        .declaration
        .unwrap();
    let mut receiver =
        test_client_for_account_scope(&receiver_root, "recovery-owner", source.state_json.clone());
    receiver.save_state().unwrap();
    receiver.state_json = app_data::default_app_data_json(300);
    receiver.save_state().unwrap();
    let before = receiver.state_json.clone();
    let receipt = private_transport_test_receipt(&receiver, declaration);
    let workspace = receiver.ai_workspace_identity();
    persist_private_media_replies(&workspace, &before, &receiver.sync, &[receipt.clone()]).unwrap();
    assert_eq!(receiver.state_json, before);
    let reopened = open_desktop_state_store(&receiver_root).unwrap();
    let owner = audit_owner(&receiver);
    assert_eq!(
        reopened
            .latest_valid(&owner, 0)
            .unwrap()
            .unwrap()
            .app_data_json,
        before
    );
    let view = reopened.private_media_references(&owner, &before).unwrap();
    let queries = view
        .queries(&reopened.privacy_policy(&owner).unwrap().0, true)
        .unwrap();
    assert_eq!(queries.len(), 1);
    assert!(queries[0].declaration.is_some());
    let evidence = reopened.journal_evidence().unwrap();
    let mut stale = receiver.sync.clone();
    stale.acknowledged_generation += 1;
    assert!(
        persist_private_media_replies(&workspace, &before, &stale, &[receipt.clone()]).is_err()
    );
    persist_private_media_replies(&workspace, &before, &receiver.sync, &[receipt]).unwrap();
    assert_eq!(reopened.journal_evidence().unwrap(), evidence);
    drop(reopened);
    drop(source_store);
    drop(source);
    drop(receiver);
    fs::remove_dir_all(root).unwrap();
}

fn private_preflight_fixture(root: &Path) -> TimerWindowsClient {
    use gridtimer_native::desktop_state_store::DesktopSealedMediaDeclaration;
    let (plain, _) =
        test_note_snapshot_with_media("historical-preflight-file", &test_one_pixel_bmp());
    let value: Value = serde_json::from_str(&plain).unwrap();
    let mut client = test_client_for_account_scope(root, "preflight-owner", plain.clone());
    let scope = client.note_crypto_session_scope().unwrap();
    let (sealed, token) = gridtimer_native::encrypt_desktop_note_json_in_scope(
        &value["notes"][0].to_string(),
        "preflight-test-password",
        &scope,
    )
    .unwrap();
    let declaration = DesktopSealedMediaDeclaration::from_session(&sealed, &token).unwrap();
    let snapshot = app_data::upsert_note_app_data_json(&plain, &sealed, 200).unwrap();
    client.state_json = snapshot;
    client.data = decode_data(&client.state_json);
    let outcome = save_state_snapshot_with_store_and_media_declaration(
        root,
        &client.state_path,
        &audit_owner(&client),
        &client.state_json,
        &client.sync,
        200,
        "local_save",
        Some(&declaration),
    )
    .unwrap();
    assert!(outcome.warning().is_none());
    gridtimer_native::close_desktop_note_session(&token);
    assert!(private_session_scope(&client).is_complete());
    client
}

fn private_preflight_sync_summary(
    client: &TimerWindowsClient,
    phase: SyncTaskPhase,
) -> gridtimer_native::desktop_note_media_sync::DesktopNoteMediaSyncSummary {
    use gridtimer_native::desktop_note_media_sync::{
        sync_desktop_note_media_with_reference_index, DesktopNoteMediaSyncDirection,
    };
    use std::io::{BufRead, Read, Write};
    let workspace = client.ai_workspace_identity();
    let store = open_desktop_state_store(&workspace.namespace_root).unwrap();
    let owner = audit_owner(client);
    let view = note_media_references_for_phase(&store, &owner, &client.state_json, phase).unwrap();
    let policy = store.privacy_policy(&owner).unwrap().0;
    let result = sync_core::SyncClientResult {
        ok: true,
        mode: "media_manifest".into(),
        user_id: client.sync.user_id.clone(),
        server_instance_id: client.sync.server_instance_id.clone(),
        account_namespace: client.sync.account_namespace.clone(),
        token_id: sync_core::token_identifier(&client.sync.token),
        current_generation: client.sync.acknowledged_generation,
        ..Default::default()
    };
    // An older but authenticated server is sufficient to reconcile known local
    // declarations. No new private protocol or remote write is assumed here.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut reader = std::io::BufReader::new(&mut stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(line.starts_with("POST /v1/media/manifest "));
        let mut length = 0;
        loop {
            line.clear();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse::<usize>().unwrap();
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        drop(reader);
        let body = serde_json::to_vec(&result).unwrap();
        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).unwrap();
        stream.write_all(&body).unwrap();
    });
    let media = note_media_store_for_workspace(&workspace).unwrap();
    let summary = sync_desktop_note_media_with_reference_index(
        &media,
        &client.state_json,
        &url,
        &client.sync.token,
        &client.sync.user_id,
        &client.sync.server_instance_id,
        &client.sync.account_namespace,
        client.sync.acknowledged_generation,
        "",
        DesktopNoteMediaSyncDirection::UploadOnly,
        Some(&policy),
        Some(&view),
    );
    worker.join().unwrap();
    summary
}

#[test]
fn private_preflight_publication_ignores_unavailable_recovery_only_attachments() {
    for deleted in [false, true] {
        let root = temp_test_dir("private-preflight-historical");
        let mut client = private_preflight_fixture(&root);
        let mut current: Value = serde_json::from_str(&app_state_with_note(
            "current-note",
            "Current",
            "CURRENT_DATA_TO_PUBLISH",
            None,
        ))
        .unwrap();
        if deleted {
            current["tombstones"].as_array_mut().unwrap().push(json!({"entityType":"noteAttachment","entityId":"historical-preflight-file","deletedAtEpochMillis":300}));
        }
        client.state_json = current.to_string();
        client.save_state().unwrap();
        let before = client.state_json.clone();
        let summary = private_preflight_sync_summary(&client, SyncTaskPhase::NoteMediaPreflight);
        assert!(
            note_media_preflight_allows_app_data(&summary),
            "historical-only attachment blocked current publication: {summary:?}"
        );
        let recovery = private_preflight_sync_summary(&client, SyncTaskPhase::NoteMedia);
        assert!(
            recovery.failures + recovery.conflicts > 0,
            "unresolved recovery work must remain visible: {recovery:?}"
        );
        assert_eq!(client.state_json, before);
        drop(client);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn private_preflight_publication_still_blocks_missing_or_deleted_current_attachments() {
    for deleted in [false, true] {
        let root = temp_test_dir("private-preflight-current");
        let mut client = private_preflight_fixture(&root);
        if deleted {
            let mut current: Value = serde_json::from_str(&client.state_json).unwrap();
            current["tombstones"].as_array_mut().unwrap().push(json!({"entityType":"noteAttachment","entityId":"historical-preflight-file","deletedAtEpochMillis":300}));
            client.state_json = current.to_string();
            client.save_state().unwrap();
        }
        let summary = private_preflight_sync_summary(&client, SyncTaskPhase::NoteMediaPreflight);
        assert!(
            !note_media_preflight_allows_app_data(&summary),
            "a current missing attachment was allowed to publish: {summary:?}"
        );
        assert!(summary.failures + summary.conflicts > 0);
        drop(client);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn private_preflight_publication_keeps_current_conflict_references_required() {
    let root = temp_test_dir("private-preflight-current-conflict");
    let mut client = private_preflight_fixture(&root);
    let mut current: Value = serde_json::from_str(&client.state_json).unwrap();
    let note = current["notes"][0].clone();
    current["notes"] = json!([]);
    current["syncConflictHistory"] = json!([{"id":"preflight-current-conflict","entityType":"note",
        "entityId":note["id"],"losingRevisionEpochMillis":100,"capturedAtEpochMillis":300,"payload":note}]);
    client.state_json = current.to_string();
    client.save_state().unwrap();
    let summary = private_preflight_sync_summary(&client, SyncTaskPhase::NoteMediaPreflight);
    assert!(
        !note_media_preflight_allows_app_data(&summary),
        "a conflict published in current AppData still needs its attachment: {summary:?}"
    );
    assert!(summary.failures > 0);
    drop(client);
    fs::remove_dir_all(root).unwrap();
}
