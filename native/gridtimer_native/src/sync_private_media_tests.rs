// v0.0.4 - Restore blocking accepted sockets before the synchronous HTTP test handler.
// v0.0.3 - Validate current and retained proof origins separately.
// v0.0.2 - Verify incomplete bytes and atomic capacity/restore rollback.
// v0.0.1 - Exercise ciphertext, owner, restore and request boundaries through real transport.
#[test]
fn private_media_transport_worker_honors_direction_and_older_server_capability() {
    use crate::desktop_note_media::DesktopNoteMediaStore;
    use crate::desktop_note_media_sync::{
        sync_desktop_note_media_with_private_references, DesktopNoteMediaSyncDirection as Direction,
    };
    use crate::desktop_state_store::DesktopPrivacyPolicy;
    let (snapshot, query) = private_media_fixture("d29ya2Vy");
    let note = serde_json::from_str::<Value>(&snapshot).unwrap()["notes"][0].clone();
    let policy = DesktopPrivacyPolicy::default()
        .including_sealed_media(
            &note,
            serde_json::from_value(query.declaration.clone().unwrap()).unwrap(),
        )
        .unwrap();
    for direction in [
        Direction::DownloadOnly,
        Direction::UploadOnly,
        Direction::Bidirectional,
    ] {
        let token = "private-worker-token";
        let account = account_test_store(token, snapshot.clone());
        let identity = account.store.server_account_identity("user-1").unwrap();
        let media = DesktopNoteMediaStore::new(account.directory.join("local-media")).unwrap();
        let (url, worker) = private_media_http_server(&account.store, 2);
        let result = sync_desktop_note_media_with_private_references(
            &media,
            &snapshot,
            &url,
            token,
            "user-1",
            &identity.server_instance_id,
            &identity.account_namespace,
            0,
            "",
            direction,
            Some(&policy),
        );
        worker.join().unwrap();
        // This metadata-only fixture deliberately has no content hash or blob.
        assert_eq!(result.failures, 1, "{:?}", result.failure_messages);
        assert_eq!(result.conflicts, 0);
        assert_eq!(
            result.private_media_pending,
            usize::from(direction == Direction::DownloadOnly)
        );
        if direction == Direction::UploadOnly {
            assert!(result.verified_private_media.is_empty());
        } else {
            assert_eq!(result.verified_private_media.len(), 1);
        }
        let token_id = match account
            .store
            .authenticate_token(token, now_millis())
            .unwrap()
        {
            TokenAuthentication::Active(value) => value.token_id,
            other => panic!("{other:?}"),
        };
        let mut read = private_media_request(query.clone());
        read.queries[0].declaration = None;
        let stored = account
            .store
            .exchange_private_media_references("user-1", token_id, &read, now_millis())
            .unwrap();
        assert_eq!(
            stored.entries[0].declaration.is_some(),
            direction != Direction::DownloadOnly
        );
    }
    let account = account_test_store("legacy-worker-token", snapshot.clone());
    let media = DesktopNoteMediaStore::new(account.directory.join("legacy-media")).unwrap();
    let (url, captured, worker) = one_shot_media_client_server(
        200,
        SyncClientResult {
            ok: true,
            user_id: "user-1".into(),
            token_id: token_identifier("legacy-worker-token"),
            server_instance_id: DESKTOP_MEDIA_SERVER_INSTANCE_ID.into(),
            account_namespace: DESKTOP_MEDIA_ACCOUNT_NAMESPACE.into(),
            mode: "media_manifest".into(),
            ..Default::default()
        },
    );
    let result = sync_desktop_note_media_with_private_references(
        &media,
        &snapshot,
        &url,
        "legacy-worker-token",
        "user-1",
        DESKTOP_MEDIA_SERVER_INSTANCE_ID,
        DESKTOP_MEDIA_ACCOUNT_NAMESPACE,
        0,
        "",
        Direction::Bidirectional,
        Some(&policy),
    );
    assert_eq!(
        captured.recv_timeout(Duration::from_secs(1)).unwrap().path,
        "/v1/media/manifest"
    );
    worker.join().unwrap();
    assert_eq!(result.failures, 1);
    assert_eq!(result.private_media_pending, 1);
    assert!(result.verified_private_media.is_empty());
}

#[test]
fn private_media_transport_defers_reference_delete_without_losing_intent_or_bytes() {
    use crate::desktop_note_media::DesktopNoteMediaStore;
    use crate::desktop_note_media_sync::{
        sync_desktop_note_media_with_direction, DesktopNoteMediaSyncDirection,
    };
    for unknown in [false, true] {
        let token = "private-deletion-token";
        let content = b"retained reference content";
        let mut server: Value =
            serde_json::from_str(&note_app_data("reference-note", 100)).unwrap();
        server["notes"][0]["attachments"] = json!([{"id":"referenced-file",
            "mimeType":"application/octet-stream", "sha256":hex_bytes(&Sha256::digest(content)),
            "sizeBytes":content.len(), "updatedAtEpochMillis":100}]);
        if unknown {
            server = serde_json::from_str(&private_media_fixture("c2VhbGVk").0).unwrap();
        }
        let account = account_test_store(token, server.to_string());
        let upload = HttpRequest {
            headers: vec![("Authorization".into(), format!("Bearer {token}"))],
            body: json!({"requestId":"private-deletion-upload","acknowledgedGeneration":0,
                "attachmentId":"referenced-file","sha256":hex_bytes(&Sha256::digest(content)),
                "mimeType":"application/octet-stream","sizeBytes":content.len(),
                "updatedAtEpochMillis":100,"contentBase64":BASE64_STANDARD.encode(content)})
            .to_string(),
            ..Default::default()
        };
        assert_eq!(handle_media_upload(&upload, &account.store).0, 200);
        let mut local: Value = serde_json::from_str(&app_data::default_app_data_json(200)).unwrap();
        local["tombstones"] = json!([{"entityType":"noteMedia","entityId":"referenced-file","deletedAtEpochMillis":200}]);
        let before = local.to_string();
        let identity = account.store.server_account_identity("user-1").unwrap();
        let media = DesktopNoteMediaStore::new(account.directory.join("local-media")).unwrap();
        let (url, worker) = private_media_http_server(&account.store, 2);
        let summary = sync_desktop_note_media_with_direction(
            &media,
            &before,
            &url,
            token,
            "user-1",
            &identity.server_instance_id,
            &identity.account_namespace,
            0,
            "",
            DesktopNoteMediaSyncDirection::UploadOnly,
        );
        worker.join().unwrap();
        assert_eq!(summary.failures, 0, "{:?}", summary.failure_messages);
        assert_eq!(
            summary.conflicts, 0,
            "a retained reference must not block uploading the new AppData"
        );
        assert_eq!(summary.deferred_reference_deletions, 1);
        assert!(summary.remote_deleted_at_by_attachment_id.is_empty());
        assert_eq!(before, local.to_string());
        let manifest = account.store.list_media_metadata("user-1", true).unwrap();
        assert_eq!(manifest[0].deleted_at_epoch_millis, None);
        let db = rusqlite::Connection::open(account.store.database_path()).unwrap();
        let remaining: Vec<u8> = db.query_row("SELECT content FROM note_media WHERE user_id='user-1' AND attachment_id='referenced-file'", [], |row| row.get(0)).unwrap();
        assert_eq!(remaining, content);
    }
}

fn private_media_fixture(
    cipher: &str,
) -> (String, crate::private_media_protocol::PrivateMediaQuery) {
    use crate::sealed_media_references::SealedMediaReferences;
    let mut note = encrypted_note_for_sync(cipher, 100, 100);
    note["encryption"]["cipherSuite"] = json!("AES-256-GCM");
    note["encryption"]["kdf"] = json!("Argon2id");
    let declaration = SealedMediaReferences::from_payload(
        "protected-note",
        &note["encryption"],
        &json!([{"id":"private-file"}]),
        &json!({}),
        &json!([]),
        &json!([]),
    )
    .unwrap();
    let mut query = crate::private_media_protocol::PrivateMediaQuery::for_note(&note).unwrap();
    query.declaration = Some(serde_json::to_value(declaration).unwrap());
    let mut data: Value = serde_json::from_str(&app_data::default_app_data_json(100)).unwrap();
    data["notes"] = json!([note]);
    (data.to_string(), query)
}

// A history restored by a transport test must first have a complete exact-
// envelope declaration and independently retained same-account bytes. Keep the
// generic IDs-only fixture above for tests of unresolved media behavior.
fn private_media_restorable_fixture(
    token: &str,
    cipher: &str,
) -> (
    TestSqliteStore,
    String,
    crate::private_media_protocol::PrivateMediaQuery,
) {
    use crate::sealed_media_references::SealedMediaReferences;
    let (data, mut query) = private_media_fixture(cipher);
    let note = serde_json::from_str::<Value>(&data).unwrap()["notes"][0].clone();
    let content = b"synthetic private transport restore bytes";
    let sha256 = hex_bytes(&Sha256::digest(content));
    let declaration = SealedMediaReferences::from_payload(
        "protected-note",
        &note["encryption"],
        &json!([{"id":"private-file", "sha256":sha256, "sizeBytes":content.len(),
            "mimeType":"application/octet-stream", "updatedAtEpochMillis":100}]),
        &json!({}),
        &json!([]),
        &json!([]),
    )
    .unwrap();
    assert!(declaration.has_complete_content_hashes());
    assert!(declaration.valid_for("protected-note", &note["encryption"]));
    query.declaration = Some(serde_json::to_value(declaration).unwrap());
    assert_eq!(query.declaration.as_ref().unwrap()["formatVersion"], 2);
    let account = account_test_store(token, data.clone());
    account
        .store
        .upsert_media(
            "user-1",
            "private-file",
            &sha256,
            "application/octet-stream",
            content.len() as i64,
            content,
            100,
        )
        .unwrap();
    let token_id = match account
        .store
        .authenticate_token(token, now_millis())
        .unwrap()
    {
        TokenAuthentication::Active(value) => value.token_id,
        other => panic!("{other:?}"),
    };
    let reply = account
        .store
        .exchange_private_media_references(
            "user-1",
            token_id,
            &private_media_request(query.clone()),
            now_millis(),
        )
        .unwrap();
    assert!(reply.entries[0].accepted && reply.entries[0].current_head_matches);
    assert_eq!(reply.entries[0].declaration, query.declaration);
    assert_eq!(
        account
            .store
            .read_media("user-1", "private-file")
            .unwrap()
            .unwrap()
            .content,
        content.as_slice()
    );
    (account, data, query)
}

fn private_media_request(
    query: crate::private_media_protocol::PrivateMediaQuery,
) -> crate::private_media_protocol::PrivateMediaRequest {
    crate::private_media_protocol::PrivateMediaRequest {
        format_version: 1,
        request_id: "private_request".into(),
        acknowledged_generation: 0,
        restore_receipt: String::new(),
        queries: vec![query],
    }
}

fn private_media_http_server(
    store: &SqliteServerStore,
    count: usize,
) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let store = Arc::new(store.clone());
    let worker = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut handled = 0;
        while handled < count {
            match listener.accept() {
                Ok((stream, _)) => {
                    // Winsock inherits the nonblocking listener mode. The real
                    // server uses blocking streams with bounded read deadlines.
                    stream.set_nonblocking(false).unwrap();
                    handle_connection(
                        stream,
                        store.clone(),
                        Arc::new(Mutex::new(ServerRuntimeInfo::default())),
                        Arc::new(Mutex::new(LoginRateLimiter::default())),
                    )
                    .unwrap();
                    handled += 1;
                }
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(5))
                }
                Err(error) => {
                    panic!("private media fixture stopped after {handled} requests: {error}")
                }
            }
        }
    });
    (url, worker)
}

#[test]
fn private_media_transport_roundtrip_rejects_wrong_head_and_keeps_old_declaration() {
    let token = "private-roundtrip-token";
    let (data, query) = private_media_fixture("Y2lwaGVyLTE=");
    let store = account_test_store(token, data.clone());
    let identity = store.store.server_account_identity("user-1").unwrap();
    let (_, wrong_head) = private_media_fixture("Y2lwaGVyLTI=");
    let (url, worker) = private_media_http_server(&store.store, 3);
    let invoke = |query| {
        desktop_private_media_exchange(
            &url,
            "user-1",
            token,
            &identity.server_instance_id,
            &identity.account_namespace,
            0,
            "",
            vec![query],
        )
    };
    let stale = invoke(wrong_head.clone());
    assert!(stale.result.ok, "{}", stale.result.message);
    let entry = &stale.verified.unwrap().reply.entries[0];
    assert!(!entry.accepted && !entry.current_head_matches && entry.declaration.is_none());
    let submitted = invoke(query.clone());
    assert!(submitted.result.ok, "{}", submitted.result.message);
    assert!(submitted.verified.unwrap().entries()[0].accepted);
    let mut read = query.clone();
    read.declaration = None;
    let fetched = invoke(read);
    assert_eq!(
        fetched.verified.unwrap().entries()[0].declaration,
        query.declaration
    );
    worker.join().unwrap();
    // A head move keeps the previous verified declaration queryable; a first
    // declaration for an uncommitted head was not persisted by the failed match.
    let token_id = match store.store.authenticate_token(token, now_millis()).unwrap() {
        TokenAuthentication::Active(value) => value.token_id,
        other => panic!("{other:?}"),
    };
    let (next, _) = private_media_fixture("Y2lwaGVyLTI=");
    let mut next: Value = serde_json::from_str(&next).unwrap();
    next["notes"][0]["updatedAtEpochMillis"] = json!(200);
    store
        .store
        .compare_and_swap_account("user-1", 0, &next.to_string(), 200)
        .unwrap();
    let mut read = private_media_request(query.clone());
    read.queries[0].declaration = None;
    let old = store
        .store
        .exchange_private_media_references("user-1", token_id, &read, now_millis())
        .unwrap();
    assert!(!old.entries[0].current_head_matches);
    assert_eq!(old.entries[0].declaration, query.declaration);
    read.queries[0] = wrong_head;
    read.queries[0].declaration = None;
    assert!(store
        .store
        .exchange_private_media_references("user-1", token_id, &read, now_millis())
        .unwrap()
        .entries[0]
        .declaration
        .is_none());
}

#[test]
fn private_media_transport_restore_and_token_checks_are_transactional() {
    let token = "private-restore-token";
    let (store, data, query) = private_media_restorable_fixture(token, "cmVzdG9yZS1maXh0dXJl");
    store
        .store
        .compare_and_swap_account("user-1", 0, &data, 200)
        .unwrap();
    store
        .store
        .compare_and_swap_account("user-1", 1, &data, 300)
        .unwrap();
    let token_id = match store.store.authenticate_token(token, now_millis()).unwrap() {
        TokenAuthentication::Active(value) => value.token_id,
        other => panic!("{other:?}"),
    };
    store
        .store
        .restore_account_snapshot("user-1", 1, 2, 500)
        .unwrap();
    let mut request = private_media_request(query);
    assert!(matches!(
        store
            .store
            .exchange_private_media_references("user-1", token_id, &request, now_millis()),
        Err(StoreError::RestoreGenerationConflict { .. })
    ));
    request.acknowledged_generation = 1;
    assert!(matches!(
        store
            .store
            .exchange_private_media_references("user-1", token_id, &request, now_millis()),
        Err(StoreError::RestoreReceiptRequired { .. })
    ));
    let receipt = store
        .store
        .ensure_restore_receipt("user-1", token_id)
        .unwrap();
    request.restore_receipt = receipt.receipt;
    assert!(store
        .store
        .exchange_private_media_references("other-user", token_id, &request, now_millis())
        .is_err());
    assert!(
        store
            .store
            .exchange_private_media_references("user-1", token_id, &request, now_millis())
            .unwrap()
            .entries[0]
            .accepted
    );
    request.acknowledged_generation = 2;
    assert!(matches!(
        store
            .store
            .exchange_private_media_references("user-1", token_id, &request, now_millis()),
        Err(StoreError::ServerGenerationRollback { .. })
    ));
    request.acknowledged_generation = 1;
    store.store.revoke_token(token, now_millis()).unwrap();
    assert!(store
        .store
        .exchange_private_media_references("user-1", token_id, &request, now_millis())
        .is_err());
}

#[test]
fn private_media_transport_response_cannot_change_identity_generation_or_request() {
    use crate::private_media_protocol::{PrivateMediaReply, PrivateMediaReplyEntry};
    let (_, query) = private_media_fixture("cmVzcG9uc2U=");
    let request = private_media_request(query.clone());
    let response = SyncClientResult {
        ok: true,
        user_id: "private-user".into(),
        token_id: token_identifier("private-token"),
        server_instance_id: DESKTOP_MEDIA_SERVER_INSTANCE_ID.into(),
        account_namespace: DESKTOP_MEDIA_ACCOUNT_NAMESPACE.into(),
        mode: "private_media_exchange".into(),
        private_media_exchange: Some(PrivateMediaReply {
            format_version: 1,
            request_id: request.request_id.clone(),
            generation: 0,
            entries: vec![PrivateMediaReplyEntry {
                note_id: query.note_id.clone(),
                envelope_sha256: query.envelope_sha256.clone(),
                current_head_matches: true,
                retained_note_matches: false,
                accepted: true,
                declaration: query.declaration.clone(),
            }],
        }),
        ..Default::default()
    };
    let verify = |value| {
        private_media::validate_client_response(
            value,
            &request,
            "private-user",
            "private-token",
            DESKTOP_MEDIA_SERVER_INSTANCE_ID,
            DESKTOP_MEDIA_ACCOUNT_NAMESPACE,
        )
    };
    assert!(verify(response.clone()).verified.is_some());
    let mut retained_response = response.clone();
    let retained = &mut retained_response
        .private_media_exchange
        .as_mut()
        .unwrap()
        .entries[0];
    retained.current_head_matches = false;
    retained.retained_note_matches = true;
    assert!(verify(retained_response).verified.is_some());
    for case in 0..14 {
        let mut bad = response.clone();
        match case {
            0 => bad.user_id = "other-user".into(),
            1 => bad.token_id = token_identifier("other-token"),
            2 => bad.server_instance_id = OTHER_DESKTOP_MEDIA_SERVER_INSTANCE_ID.into(),
            3 => bad.account_namespace = OTHER_DESKTOP_MEDIA_ACCOUNT_NAMESPACE.into(),
            4 => bad.private_media_exchange.as_mut().unwrap().request_id = "other-request".into(),
            5 => bad.private_media_exchange.as_mut().unwrap().generation = 1,
            6 => bad.private_media_exchange.as_mut().unwrap().entries.clear(),
            7 => bad.private_media_exchange.as_mut().unwrap().entries[0].declaration = None,
            8 => {
                bad.private_media_exchange.as_mut().unwrap().entries[0].note_id =
                    "other-note".into()
            }
            9 => {
                bad.private_media_exchange.as_mut().unwrap().entries[0]
                    .declaration
                    .as_mut()
                    .unwrap()["attachmentIds"] = json!([])
            }
            10 => bad.restore_required = true,
            11 => bad.current_generation = 1,
            12 => {
                bad.private_media_exchange.as_mut().unwrap().entries[0].current_head_matches = false
            }
            _ => {
                bad.private_media_exchange.as_mut().unwrap().entries[0].retained_note_matches = true
            }
        }
        let result = verify(bad);
        assert!(
            !result.result.ok && result.verified.is_none(),
            "case {case}"
        );
    }
    let mut duplicate = request.clone();
    duplicate.queries.push(query);
    assert!(duplicate.validate().is_err());
    let mut invalid = request.clone();
    invalid.acknowledged_generation = -1;
    assert!(invalid.validate().is_err());
}

#[test]
fn private_media_transport_capacity_rejection_rolls_back_metadata_and_restore_ack() {
    use crate::private_media_protocol::{PrivateMediaQuery, PRIVATE_MEDIA_MAX_REQUEST_BYTES};
    use crate::sealed_media_references::SealedMediaReferences;
    let (data, _) = private_media_fixture("Y2FwYWNpdHk=");
    let mut data: Value = serde_json::from_str(&data).unwrap();
    let template = data["notes"][0].clone();
    let attachments = Value::Array(
        (0..10_000)
            .map(|id| json!({"id":id.to_string(),"sha256":"a".repeat(64)}))
            .collect(),
    );
    let mut notes = Vec::new();
    let mut queries = Vec::new();
    for index in 0..5 {
        let mut note = template.clone();
        let id = format!("capacity-note-{index}");
        note["id"] = json!(id);
        let declaration = SealedMediaReferences::from_payload(
            &id,
            &note["encryption"],
            &attachments,
            &json!({}),
            &json!([]),
            &json!([]),
        )
        .unwrap();
        let mut query = PrivateMediaQuery::for_note(&note).unwrap();
        query.declaration = Some(serde_json::to_value(declaration).unwrap());
        notes.push(note);
        queries.push(query);
    }
    data["notes"] = json!(notes);
    let token = "private-capacity-token";
    // Produce the real restore barrier from a small, byte-complete history.
    // The oversized declarations under test must not be prerequisites for
    // that restore: 50,000 declared references exceed the policy capacity.
    let (account, restorable, _) =
        private_media_restorable_fixture(token, "Y2FwYWNpdHktcmVzdG9yZQ==");
    account
        .store
        .compare_and_swap_account("user-1", 0, &restorable, 200)
        .unwrap();
    account
        .store
        .compare_and_swap_account("user-1", 1, &restorable, 300)
        .unwrap();
    account
        .store
        .restore_account_snapshot("user-1", 1, 2, 500)
        .unwrap();
    // Move the current head to the capacity scenario while retaining the
    // restored note. A server-side CAS does not acknowledge the token barrier.
    data["notes"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::from_str::<Value>(&restorable).unwrap()["notes"][0].clone());
    account
        .store
        .compare_and_swap_account("user-1", 3, &data.to_string(), 600)
        .unwrap();
    let token_id = match account
        .store
        .authenticate_token(token, now_millis())
        .unwrap()
    {
        TokenAuthentication::Active(value) => value.token_id,
        other => panic!("{other:?}"),
    };
    let receipt = account
        .store
        .ensure_restore_receipt("user-1", token_id)
        .unwrap();
    let mut request = private_media_request(queries[0].clone());
    request.queries = queries;
    request.acknowledged_generation = 1;
    request.restore_receipt = receipt.receipt;
    assert!(serde_json::to_vec(&request).unwrap().len() <= PRIVATE_MEDIA_MAX_REQUEST_BYTES);
    request.validate().unwrap();
    let connection = rusqlite::Connection::open(account.store.database_path()).unwrap();
    let read_policy = || {
        connection
            .query_row(
                "SELECT policy_json FROM account_note_privacy WHERE user_id='user-1'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
    };
    let before = read_policy();
    let result =
        account
            .store
            .exchange_private_media_references("user-1", token_id, &request, now_millis());
    assert!(
        matches!(result, Err(StoreError::Integrity(_))),
        "{result:?}"
    );
    assert_eq!(read_policy(), before);
    let barrier = account
        .store
        .restore_barrier_state("user-1", token_id)
        .unwrap();
    assert_eq!(barrier.current_generation, 1);
    assert!(!barrier.token_restore_acknowledged);
    request.queries.truncate(1);
    let committed = account
        .store
        .exchange_private_media_references("user-1", token_id, &request, now_millis())
        .unwrap();
    assert!(committed.entries[0].accepted);
    assert!(
        account
            .store
            .restore_barrier_state("user-1", token_id)
            .unwrap()
            .token_restore_acknowledged
    );
    assert_ne!(read_policy(), before);
}
