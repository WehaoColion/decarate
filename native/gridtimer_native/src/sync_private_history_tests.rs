// v0.0.6 - Verify compact history sync, reference persistence and cumulative data above 32 MiB.
// v0.0.5 - Bind metadata-only cleanup state to durable reference commits.
// v0.0.4 - Profile bounded historical lookups and preserve filtered identity guards.
// v0.0.3 - Verify byte transfer from recovery-only encrypted records.
// v0.0.2 - Keep legacy records unresolved without blocking independent attachment transfers.
// v0.0.1 - Reproduce missing historical declarations and conflict-only byte transfer.
#[test]
fn private_history_transport_indexes_exact_ciphertexts_and_rejects_ambiguous_owners() {
    let (old, _) = private_media_fixture("aW5kZXgtb2xk");
    let (current, _) = private_media_fixture("aW5kZXgtY3VycmVudA==");
    let old: Value = serde_json::from_str(&old).unwrap();
    let mut current: Value = serde_json::from_str(&current).unwrap();
    let conflict = json!({"id":"index-conflict", "entityType":"note", "entityId":"protected-note",
        "losingRevisionEpochMillis":100,"capturedAtEpochMillis":200,"payload":old["notes"][0]});
    current["syncConflictHistory"] = json!([conflict.clone(), conflict]);
    let policy = crate::desktop_state_store::DesktopPrivacyPolicy::default();
    let queries = policy
        .private_media_queries(&current.to_string(), false)
        .unwrap();
    assert_eq!(queries.len(), 2);
    assert_ne!(queries[0].envelope_sha256, queries[1].envelope_sha256);
    current["syncConflictHistory"][0]["entityId"] = json!("wrong-owner");
    assert!(policy
        .private_media_queries(&current.to_string(), false)
        .is_err());
    current["syncConflictHistory"] = json!([]);
    let note = current["notes"][0].clone();
    current["notes"].as_array_mut().unwrap().push(note);
    assert!(policy
        .private_media_queries(&current.to_string(), false)
        .is_err());
}

#[test]
fn private_history_transport_accepts_first_declaration_only_for_retained_ciphertext() {
    let token = "private-history-owner";
    let (old, query) = private_media_fixture("b2xkLWhpc3Rvcnk=");
    let (new, _) = private_media_fixture("bmV3LWhlYWQ=");
    let account = account_test_store(token, old);
    account
        .store
        .compare_and_swap_account("user-1", 0, &new, 200)
        .unwrap();
    let token_id = match account
        .store
        .authenticate_token(token, now_millis())
        .unwrap()
    {
        TokenAuthentication::Active(value) => value.token_id,
        other => panic!("{other:?}"),
    };
    let accepted = account
        .store
        .exchange_private_media_references(
            "user-1",
            token_id,
            &private_media_request(query.clone()),
            now_millis(),
        )
        .unwrap();
    assert!(
        accepted.entries[0].accepted,
        "retained history was never declared: {accepted:?}"
    );
    assert!(!accepted.entries[0].current_head_matches);
    assert_eq!(accepted.entries[0].declaration, query.declaration);
    let (_, absent) = private_media_fixture("bmV2ZXItY29tbWl0dGVk");
    let rejected = account
        .store
        .exchange_private_media_references(
            "user-1",
            token_id,
            &private_media_request(absent),
            now_millis(),
        )
        .unwrap();
    assert!(!rejected.entries[0].accepted);
    assert!(rejected.entries[0].declaration.is_none());
}

#[test]
fn private_history_transport_conflict_receipt_downloads_real_bytes_and_remains_bound() {
    use crate::desktop_note_media::DesktopNoteMediaStore;
    use crate::desktop_note_media_sync::{
        sync_desktop_note_media_with_private_references as sync,
        DesktopNoteMediaSyncDirection as Direction,
    };
    let (old, policy, bytes, sha) = encrypted_byte_fixture();
    let old: Value = serde_json::from_str(&old).unwrap();
    let mut current: Value = serde_json::from_str(&app_data::upsert_note_app_data_json(
        &app_data::default_app_data_json(300),
        &json!({"id":"private-byte-note","title":"Current", "content":"Current body", "updatedAtEpochMillis":300}).to_string(),300).unwrap()).unwrap();
    current["syncConflictHistory"] = json!([{"id":"retained-private-conflict", "entityType":"note",
        "entityId":"private-byte-note", "losingRevisionEpochMillis":100,"capturedAtEpochMillis":300,"payload":old["notes"][0]}]);
    let current = current.to_string();
    let queries = policy.private_media_queries(&current, true).unwrap();
    assert_eq!(
        queries.len(),
        1,
        "the only private note is in conflict history"
    );
    let token = "private-conflict-owner";
    let account = account_test_store(token, current.clone());
    let identity = account.store.server_account_identity("user-1").unwrap();
    let server = EncryptedByteServer::start(&account.store);
    let sender = DesktopNoteMediaStore::new(account.directory.join("sender")).unwrap();
    sender
        .write_download_blob(
            "cipher-file",
            &sha,
            bytes.len() as i64,
            "image/bmp",
            100,
            &bytes,
        )
        .unwrap();
    let sent = sync(
        &sender,
        &current,
        &server.url,
        token,
        "user-1",
        &identity.server_instance_id,
        &identity.account_namespace,
        0,
        "",
        Direction::UploadOnly,
        Some(&policy),
    );
    assert_eq!(sent.uploaded, 1, "{sent:?}");
    assert_eq!(
        sent.failures + sent.conflicts + sent.private_media_pending,
        0,
        "{sent:?}"
    );
    let receiver = DesktopNoteMediaStore::new(account.directory.join("receiver")).unwrap();
    let received = sync(
        &receiver,
        &current,
        &server.url,
        token,
        "user-1",
        &identity.server_instance_id,
        &identity.account_namespace,
        0,
        "",
        Direction::DownloadOnly,
        Some(&Default::default()),
    );
    assert_eq!(received.downloaded, 1, "{received:?}");
    assert_eq!(
        receiver
            .read_blob("cipher-file", &sha, bytes.len() as i64)
            .unwrap(),
        bytes
    );
    let declarations =
        crate::desktop_state_store::DesktopSealedMediaDeclaration::from_verified_replies(
            &received.verified_private_media,
            &current,
            "user-1",
            token,
            &identity.server_instance_id,
            &identity.account_namespace,
            0,
        )
        .unwrap();
    assert_eq!(declarations.len(), 1);
    let removed = app_data::default_app_data_json(400);
    assert!(
        crate::desktop_state_store::DesktopSealedMediaDeclaration::from_verified_replies(
            &received.verified_private_media,
            &removed,
            "user-1",
            token,
            &identity.server_instance_id,
            &identity.account_namespace,
            0
        )
        .unwrap()
        .is_empty()
    );
}

#[test]
fn private_history_transport_rejects_corrupt_retained_source_before_accepting_a_declaration() {
    let token = "private-history-corrupt";
    let (old, query) = private_media_fixture("aW50YWN0LWhpc3Rvcnk=");
    let (new, _) = private_media_fixture("aW50YWN0LWhlYWQ=");
    let account = account_test_store(token, old);
    account
        .store
        .compare_and_swap_account("user-1", 0, &new, 200)
        .unwrap();
    let token_id = match account
        .store
        .authenticate_token(token, now_millis())
        .unwrap()
    {
        TokenAuthentication::Active(value) => value.token_id,
        other => panic!("{other:?}"),
    };
    let connection = rusqlite::Connection::open(account.store.database_path()).unwrap();
    assert_eq!(connection.execute("UPDATE snapshot_contents SET content=zeroblob(compressed_size_bytes) WHERE sha256 IN (SELECT content_sha256 FROM account_snapshot_history WHERE user_id='user-1' AND revision=0)", []).unwrap(), 1);
    assert!(account
        .store
        .exchange_private_media_references(
            "user-1",
            token_id,
            &private_media_request(query),
            now_millis()
        )
        .is_err());
}

#[test]
fn private_history_transport_never_uses_another_accounts_retained_snapshot() {
    let (old, query) = private_media_fixture("YWNjb3VudC10d28tb2xk");
    let (new, _) = private_media_fixture("YWNjb3VudC10d28tbmV3");
    let users = (1..=2)
        .map(|number| ServerUser {
            id: format!("user-{number}"),
            email: format!("history-{number}@example.com"),
            password_salt: "legacy-salt".into(),
            password_hash: password_hash("legacy-salt", "secret-password"),
            created_at_epoch_millis: 1,
            updated_at_epoch_millis: 1,
            app_data_json: if number == 2 {
                old.clone()
            } else {
                app_data::default_app_data_json(100)
            },
            tokens: vec![ServerToken {
                token: format!("history-account-{number}"),
                device_name: "fixture".into(),
                created_at_epoch_millis: 1,
                last_seen_at_epoch_millis: 1,
            }],
        })
        .collect();
    let account = sqlite_test_store(ServerStore { users });
    account
        .store
        .compare_and_swap_account("user-2", 0, &new, 200)
        .unwrap();
    for number in 1..=2 {
        let token_id = match account
            .store
            .authenticate_token(&format!("history-account-{number}"), now_millis())
            .unwrap()
        {
            TokenAuthentication::Active(value) => value.token_id,
            other => panic!("{other:?}"),
        };
        let result = account
            .store
            .exchange_private_media_references(
                &format!("user-{number}"),
                token_id,
                &private_media_request(query.clone()),
                now_millis(),
            )
            .unwrap();
        assert_eq!(result.entries[0].accepted, number == 2);
        assert_eq!(result.entries[0].declaration.is_some(), number == 2);
    }
}

fn private_legacy_unindexed_note() -> Value {
    let raw = app_data::upsert_note_app_data_json(&app_data::default_app_data_json(100),
        &json!({"id":"legacy-unindexed-note","title":"Legacy","content":"Retained old body","updatedAtEpochMillis":100}).to_string(),100).unwrap();
    let note = serde_json::from_str::<Value>(&raw).unwrap()["notes"][0].clone();
    let (sealed, token) =
        crate::note_crypto::encrypt_note(&note.to_string(), "legacy-compatibility-password")
            .unwrap();
    crate::note_crypto::close_session(&token);
    let mut legacy: Value = serde_json::from_str(&sealed).unwrap();
    for field in ["formatVersion", "cipherSuite", "kdf"] {
        legacy["encryption"].as_object_mut().unwrap().remove(field);
    }
    let (unlocked, token) =
        crate::note_crypto::unlock_note(&legacy.to_string(), "legacy-compatibility-password")
            .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&unlocked).unwrap()["content"],
        "Retained old body"
    );
    crate::note_crypto::close_session(&token);
    legacy
}

#[test]
fn private_legacy_transport_keeps_known_bytes_moving_and_unknown_deletions_blocked() {
    use crate::desktop_note_media::DesktopNoteMediaStore;
    use crate::desktop_note_media_sync::{
        sync_desktop_note_media_with_private_references as sync,
        DesktopNoteMediaSyncDirection as Direction,
    };
    let (snapshot, policy, bytes, sha) = encrypted_byte_fixture();
    let legacy = private_legacy_unindexed_note();
    for conflict_only in [false, true] {
        let mut mixed: Value = serde_json::from_str(&snapshot).unwrap();
        if conflict_only {
            mixed["syncConflictHistory"] = json!([{"id":"legacy-conflict", "entityType":"note", "entityId":"legacy-unindexed-note", "payload":legacy}]);
        } else {
            mixed["notes"].as_array_mut().unwrap().push(legacy.clone());
        }
        mixed["tombstones"].as_array_mut().unwrap().push(json!({"entityType":"noteMedia","entityId":"unproven-removal","deletedAtEpochMillis":200}));
        let mixed = mixed.to_string();
        let queries = policy
            .private_media_queries(&mixed, true)
            .expect("an unrelated legacy envelope must not block supported queries");
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].note_id, "private-byte-note");
        let scope = crate::desktop_media_references::DesktopMediaReferenceScope::from_snapshot(
            &mixed, &policy,
        );
        assert!(!scope.is_complete());
        assert!(scope.unknown_sealed_notes() > 0);
        let token = "private-legacy-byte-owner";
        let account = account_test_store(token, mixed.clone());
        let identity = account.store.server_account_identity("user-1").unwrap();
        let server = EncryptedByteServer::start(&account.store);
        let media = DesktopNoteMediaStore::new(account.directory.join("mixed-media")).unwrap();
        media
            .write_download_blob(
                "cipher-file",
                &sha,
                bytes.len() as i64,
                "image/bmp",
                100,
                &bytes,
            )
            .unwrap();
        let result = sync(
            &media,
            &mixed,
            &server.url,
            token,
            "user-1",
            &identity.server_instance_id,
            &identity.account_namespace,
            0,
            "",
            Direction::UploadOnly,
            Some(&policy),
        );
        assert_eq!(result.uploaded, 1, "{result:?}");
        assert!(
            result.failures > 0,
            "unknown references must not appear completely synchronized: {result:?}"
        );
        assert!(result.deferred_reference_deletions > 0, "{result:?}");
        assert_eq!(
            media
                .read_blob("cipher-file", &sha, bytes.len() as i64)
                .unwrap(),
            bytes
        );
    }
}

#[test]
fn private_legacy_transport_skips_unindexable_history_without_authorizing_it() {
    let token = "legacy-history-owner";
    let (old, query) = private_media_fixture("cmV0YWluZWQta25vd24=");
    let (new, _) = private_media_fixture("Y3VycmVudC1rbm93bg==");
    let account = account_test_store(token, old);
    let mut intermediate: Value = serde_json::from_str(&new).unwrap();
    intermediate["notes"]
        .as_array_mut()
        .unwrap()
        .push(private_legacy_unindexed_note());
    account
        .store
        .compare_and_swap_account("user-1", 0, &intermediate.to_string(), 200)
        .unwrap();
    account
        .store
        .compare_and_swap_account("user-1", 1, &new, 300)
        .unwrap();
    let token_id = match account
        .store
        .authenticate_token(token, now_millis())
        .unwrap()
    {
        TokenAuthentication::Active(value) => value.token_id,
        other => panic!("{other:?}"),
    };
    let result = account
        .store
        .exchange_private_media_references(
            "user-1",
            token_id,
            &private_media_request(query),
            now_millis(),
        )
        .unwrap();
    assert!(result.entries[0].accepted);
    assert!(result.entries[0].retained_note_matches);

    // A future field may hold more references than this build understands.
    // Even a previously supported envelope cannot grant a declaration then.
    let (unknown, unsupported_query) = private_media_fixture("ZnV0dXJlLXNjaGVtYQ==");
    let mut unknown: Value = serde_json::from_str(&unknown).unwrap();
    unknown["notes"][0]["futureAttachmentContainer"] = json!(["unseen-file"]);
    let known = crate::desktop_state_store::DesktopPrivacyPolicy::default()
        .including_sealed_media(
            &unknown["notes"][0],
            unsupported_query.parsed_declaration().unwrap().unwrap(),
        )
        .unwrap();
    assert!(known
        .private_media_queries(&unknown.to_string(), true)
        .unwrap()
        .is_empty());
    let scope = crate::desktop_media_references::DesktopMediaReferenceScope::from_snapshot(
        &unknown.to_string(),
        &known,
    );
    assert!(
        !scope.is_complete(),
        "an older proof cannot make a future note shape complete"
    );
}

#[test]
fn private_recovery_transport_exchanges_bytes_from_retained_journal_without_restoring_note() {
    use crate::desktop_note_media::DesktopNoteMediaStore;
    use crate::desktop_note_media_sync::{
        sync_desktop_note_media_with_reference_index as sync,
        DesktopNoteMediaSyncDirection as Direction,
    };
    use crate::desktop_state_store::DesktopStateStore;
    let (old, policy, bytes, sha) = encrypted_byte_fixture();
    let current = app_data::default_app_data_json(300);
    let account = account_test_store("recovery-owner", old.clone());
    account
        .store
        .compare_and_swap_account("user-1", 0, &current, 300)
        .unwrap();
    let identity = account.store.server_account_identity("user-1").unwrap();
    let owner = crate::desktop_state_store::desktop_state_owner(
        &identity.server_instance_id,
        &identity.account_namespace,
        "user-1",
    )
    .unwrap();
    let journal = DesktopStateStore::open(account.directory.join("recovery-journal.db")).unwrap();
    journal.record(&owner, &old, 100, "local_save").unwrap();
    journal.record(&owner, &current, 300, "local_save").unwrap();
    let evidence = journal.journal_evidence().unwrap();
    let reference_snapshot = journal.private_media_references(&owner, &current).unwrap();
    assert_eq!(
        reference_snapshot.queries(&policy, true).unwrap().len(),
        1,
        "the worker view must include a known proof from retained local history"
    );
    assert_eq!(journal.journal_evidence().unwrap(), evidence);
    assert_eq!(
        journal
            .latest_valid(&owner, 0)
            .unwrap()
            .unwrap()
            .app_data_json,
        current
    );
    assert!(journal
        .private_media_references("other-owner", &current)
        .unwrap()
        .queries(&policy, true)
        .unwrap()
        .is_empty());
    let server = EncryptedByteServer::start(&account.store);
    let sender = DesktopNoteMediaStore::new(account.directory.join("sender")).unwrap();
    sender
        .write_download_blob(
            "cipher-file",
            &sha,
            bytes.len() as i64,
            "image/bmp",
            100,
            &bytes,
        )
        .unwrap();
    let result = sync(
        &sender,
        &current,
        &server.url,
        "recovery-owner",
        "user-1",
        &identity.server_instance_id,
        &identity.account_namespace,
        0,
        "",
        Direction::UploadOnly,
        Some(&policy),
        Some(&reference_snapshot),
    );
    assert_eq!(result.uploaded, 1, "{result:?}");
    assert_eq!(
        result.failures + result.private_media_pending + result.conflicts,
        0,
        "{result:?}"
    );
    let receiver = DesktopNoteMediaStore::new(account.directory.join("receiver")).unwrap();
    let result = sync(
        &receiver,
        &current,
        &server.url,
        "recovery-owner",
        "user-1",
        &identity.server_instance_id,
        &identity.account_namespace,
        0,
        "",
        Direction::DownloadOnly,
        Some(&Default::default()),
        Some(&reference_snapshot),
    );
    assert_eq!(result.downloaded, 1, "{result:?}");
    assert_eq!(
        receiver
            .read_blob("cipher-file", &sha, bytes.len() as i64)
            .unwrap(),
        bytes
    );
    let declarations =
        crate::desktop_state_store::DesktopSealedMediaDeclaration::from_verified_retained_replies(
            &result.verified_private_media,
            &reference_snapshot,
            "user-1",
            "recovery-owner",
            &identity.server_instance_id,
            &identity.account_namespace,
            0,
        )
        .unwrap();
    assert_eq!(declarations.len(), 1);
    let policy_before = journal.privacy_policy(&owner).unwrap();
    let evidence_before = journal.journal_evidence().unwrap();
    let latest_before = journal.latest_valid(&owner, 0).unwrap().unwrap();
    let connection = rusqlite::Connection::open(journal.database_path()).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_recovery_receipt BEFORE UPDATE OF commit_sequence ON desktop_state_journal_metadata BEGIN SELECT RAISE(ABORT,'injected recovery receipt failure'); END;").unwrap();
    assert!(journal
        .record_with_sync_state_and_media_declarations(
            &owner,
            &current,
            b"",
            400,
            "private_media_sync",
            &declarations
        )
        .is_err());
    assert_eq!(journal.privacy_policy(&owner).unwrap(), policy_before);
    assert_eq!(journal.journal_evidence().unwrap(), evidence_before);
    connection
        .execute_batch("DROP TRIGGER reject_recovery_receipt;")
        .unwrap();
    let saved = journal
        .record_with_sync_state_and_media_declarations(
            &owner,
            &current,
            b"",
            400,
            "private_media_sync",
            &declarations,
        )
        .unwrap();
    assert_eq!(
        saved.id, latest_before.id,
        "metadata must not create a synthetic history snapshot"
    );
    let evidence_after = journal.journal_evidence().unwrap();
    assert_eq!(
        evidence_after.commit_sequence,
        evidence_before.commit_sequence + 1
    );
    journal
        .record_with_sync_state_and_media_declarations(
            &owner,
            &current,
            b"",
            500,
            "private_media_sync",
            &declarations,
        )
        .unwrap();
    assert_eq!(journal.journal_evidence().unwrap(), evidence_after);
    assert!(journal
        .record_with_sync_state_and_media_declarations(
            "other-owner",
            &current,
            b"",
            500,
            "private_media_sync",
            &declarations
        )
        .is_err());
    assert_eq!(journal.journal_evidence().unwrap(), evidence_after);
    let queries = reference_snapshot
        .queries(&journal.privacy_policy(&owner).unwrap().0, true)
        .unwrap();
    assert!(queries[0].declaration.is_some());
    assert_eq!(
        journal
            .latest_valid(&owner, 0)
            .unwrap()
            .unwrap()
            .app_data_json,
        current
    );
}

#[test]
fn private_recovery_transport_rejects_corrupted_local_history() {
    use crate::desktop_state_store::DesktopStateStore;
    let (old, _) = private_media_fixture("cmVjb3ZlcnktaW50ZWdyaXR5");
    let account = account_test_store("recovery-integrity", old.clone());
    let journal = DesktopStateStore::open(account.directory.join("corrupt-journal.db")).unwrap();
    let retained = journal.record("owner", &old, 100, "local_save").unwrap();
    let current = app_data::default_app_data_json(300);
    journal
        .record("owner", &current, 300, "local_save")
        .unwrap();
    let evidence = journal.journal_evidence().unwrap();
    let connection = rusqlite::Connection::open(journal.database_path()).unwrap();
    connection
        .execute(
            "UPDATE desktop_state_snapshots SET app_data_json='{}' WHERE id=?1",
            [retained.id],
        )
        .unwrap();
    assert!(
        journal.private_media_references("owner", &current).is_err(),
        "corrupt history must not grant reference authority"
    );
    assert_eq!(journal.journal_evidence().unwrap(), evidence);
}

#[test]
fn private_recovery_transport_deduplicates_ciphertexts_and_respects_permanent_deletion() {
    use crate::desktop_state_store::DesktopStateStore;
    let (old, _) = private_media_fixture("cmVjb3ZlcnktZGVkdXBsaWNhdGU=");
    let account = account_test_store("recovery-deduplicate", old.clone());
    let journal =
        DesktopStateStore::open(account.directory.join("deduplicate-journal.db")).unwrap();
    journal.record("owner", &old, 100, "local_save").unwrap();
    let mut next: Value = serde_json::from_str(&old).unwrap();
    for revision in 101..=108 {
        next["notes"][0]["updatedAtEpochMillis"] = json!(revision);
        journal
            .record("owner", &next.to_string(), revision, "local_save")
            .unwrap();
    }
    let current = app_data::default_app_data_json(300);
    journal
        .record("owner", &current, 300, "local_save")
        .unwrap();
    let before = journal.journal_evidence().unwrap();
    let view = journal.private_media_references("owner", &current).unwrap();
    assert_eq!(view.queries(&Default::default(), false).unwrap().len(), 1);
    assert_eq!(journal.journal_evidence().unwrap(), before);
    let deleted =
        app_data::delete_note_permanently_app_data_json(&next.to_string(), "protected-note", 500)
            .unwrap();
    journal
        .record("owner", &deleted, 500, "local_save")
        .unwrap();
    let view = journal.private_media_references("owner", &deleted).unwrap();
    assert!(view
        .queries(&journal.privacy_policy("owner").unwrap().0, true)
        .unwrap()
        .is_empty());
    assert!(view
        .scope(&journal.privacy_policy("owner").unwrap().0)
        .is_complete());
}

// This opt-in profile exercises the real store and account transaction; it has
// no wall-clock assertion and is excluded from the normal regression suite.
#[test]
#[ignore = "large retained-history lookup profile"]
fn private_lookup_profile_large_unrelated_ciphertexts() {
    let (initial, target) = private_media_fixture("bG9va3VwLXJldGFpbmVkLXRhcmdldA==");
    let mut data: Value = serde_json::from_str(&initial).unwrap();
    let template = data["notes"][0].clone();
    for index in 0..12 {
        let mut unrelated = template.clone();
        unrelated["id"] = json!(format!("unrelated-{index}"));
        unrelated["encryption"]["ciphertextBase64"] = json!("QUJD".repeat(128 * 1024));
        data["notes"].as_array_mut().unwrap().push(unrelated);
    }
    let snapshot_bytes = data.to_string().len();
    let account = account_test_store("lookup-profile", data.to_string());
    data["notes"].as_array_mut().unwrap().remove(0);
    for revision in 0..4 {
        data["notes"][0]["updatedAtEpochMillis"] = json!(200 + revision);
        account
            .store
            .compare_and_swap_account("user-1", revision, &data.to_string(), 200 + revision)
            .unwrap();
    }
    let token_id = match account
        .store
        .authenticate_token("lookup-profile", now_millis())
        .unwrap()
    {
        TokenAuthentication::Active(value) => value.token_id,
        other => panic!("{other:?}"),
    };
    let connection = rusqlite::Connection::open(account.store.database_path()).unwrap();
    let history_count: i64 = connection
        .query_row(
            "SELECT count(*) FROM account_snapshot_history WHERE user_id='user-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let (_, missing) = private_media_fixture("bG9va3VwLW5ldmVyLXJldGFpbmVk");
    let mut request = private_media_request(target.clone());
    request.queries.push(missing.clone());
    let start = Instant::now();
    let first = account
        .store
        .exchange_private_media_references("user-1", token_id, &request, now_millis())
        .unwrap();
    let first_ms = start.elapsed().as_secs_f64() * 1000.0;
    let first_phases = SqliteServerStore::take_private_media_phase_timings();
    assert!(first.entries[0].accepted && first.entries[0].retained_note_matches);
    assert!(!first.entries[0].current_head_matches);
    assert!(!first.entries[1].accepted && first.entries[1].declaration.is_none());
    let start = Instant::now();
    let absent = account
        .store
        .exchange_private_media_references(
            "user-1",
            token_id,
            &private_media_request(missing),
            now_millis(),
        )
        .unwrap();
    let absent_ms = start.elapsed().as_secs_f64() * 1000.0;
    let absent_phases = SqliteServerStore::take_private_media_phase_timings();
    assert!(!absent.entries[0].accepted && absent.entries[0].declaration.is_none());
    let start = Instant::now();
    let repeated = account
        .store
        .exchange_private_media_references(
            "user-1",
            token_id,
            &private_media_request(target),
            now_millis(),
        )
        .unwrap();
    let repeated_ms = start.elapsed().as_secs_f64() * 1000.0;
    let repeated_phases = SqliteServerStore::take_private_media_phase_timings();
    assert!(repeated.entries[0].declaration.is_some());
    println!(
        "LOOKUP_PROFILE {}",
        json!({"snapshotBytes":snapshot_bytes,"historySnapshots":history_count,
        "firstAndMissingMillis":first_ms,"missingMillis":absent_ms,"retainedMillis":repeated_ms,
        "phases":{"first":first_phases,"absent":absent_phases,"repeated":repeated_phases}})
    );
}

#[test]
fn private_lookup_scoped_index_preserves_ciphertext_and_global_owner_guards() {
    use crate::private_media_retained::RetainedSealedNotes;
    let (raw, query) = private_media_fixture("c2NvcGVkLWxvb2t1cC1vbGQ=");
    let mut snapshot: Value = serde_json::from_str(&raw).unwrap();
    let old = snapshot["notes"][0].clone();
    snapshot["notes"][0]["id"] = json!("unrequested-note");
    snapshot["syncConflictHistory"] = json!([{"id":"target-conflict","entityType":"note",
        "entityId":"protected-note","payload":old}]);
    let wanted = std::collections::BTreeSet::from(["protected-note"]);
    let index = RetainedSealedNotes::for_note_ids(&snapshot, &wanted).unwrap();
    assert_eq!(index.records().count(), 1);
    assert!(index
        .get("protected-note", &query.envelope_sha256)
        .is_some());
    assert!(!index.is_current("protected-note", &query.envelope_sha256));
    assert!(index
        .get("unrequested-note", &query.envelope_sha256)
        .is_none());
    let absent = std::collections::BTreeSet::from(["absent-note"]);
    assert_eq!(
        RetainedSealedNotes::for_note_ids(&snapshot, &absent)
            .unwrap()
            .records()
            .count(),
        0
    );

    // Unrequested identities still cannot excuse a damaged shared document.
    let unrequested = snapshot["notes"][0].clone();
    snapshot["notes"]
        .as_array_mut()
        .unwrap()
        .push(unrequested.clone());
    assert!(RetainedSealedNotes::for_note_ids(&snapshot, &wanted).is_err());
    snapshot["notes"].as_array_mut().unwrap().pop();
    snapshot["syncConflictHistory"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"unrequested-conflict",
        "entityType":"note","entityId":"wrong-owner","payload":unrequested}));
    assert!(RetainedSealedNotes::for_note_ids(&snapshot, &wanted).is_err());
}

#[test]
#[ignore = "paired retained-note index profile"]
fn private_lookup_profile_paired_index() {
    use crate::private_media_retained::RetainedSealedNotes;
    let (raw, query) = private_media_fixture("cGFpcmVkLWxvb2t1cC10YXJnZXQ=");
    let mut snapshot: Value = serde_json::from_str(&raw).unwrap();
    let template = snapshot["notes"][0].clone();
    for number in 0..12 {
        let mut unrelated = template.clone();
        unrelated["id"] = json!(format!("unrelated-{number}"));
        unrelated["encryption"]["ciphertextBase64"] = json!("QUJD".repeat(128 * 1024));
        snapshot["notes"].as_array_mut().unwrap().push(unrelated);
    }
    let wanted = std::collections::BTreeSet::from(["protected-note", "absent-note"]);
    let mut full_millis = Vec::new();
    let mut scoped_millis = Vec::new();
    for round in 0..8 {
        for scoped in if round % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            let start = Instant::now();
            let index = if scoped {
                RetainedSealedNotes::for_note_ids(std::hint::black_box(&snapshot), &wanted).unwrap()
            } else {
                RetainedSealedNotes::from_snapshot(std::hint::black_box(&snapshot)).unwrap()
            };
            let matched = std::hint::black_box(index.get("protected-note", &query.envelope_sha256));
            assert!(matched.is_some());
            assert!(index.is_current("protected-note", &query.envelope_sha256));
            assert!(index.get("absent-note", &query.envelope_sha256).is_none());
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            if scoped {
                scoped_millis.push(elapsed)
            } else {
                full_millis.push(elapsed)
            }
        }
    }
    println!(
        "LOOKUP_INDEX_PROFILE {}",
        json!({"snapshotBytes":snapshot.to_string().len(),
        "fullIndexMillis":full_millis,"scopedIndexMillis":scoped_millis,"alternatingOrder":true})
    );
}

fn private_commit_pending(connection: &rusqlite::Connection) -> i64 {
    connection
        .query_row(
            "SELECT cleanup_pending FROM account_note_privacy WHERE user_id='user-1'",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn private_commit_metadata_keeps_clean_database_clean_and_existing_work_pending() {
    for existing_pending in [false, true] {
        let (snapshot, query) = private_media_fixture(
            "bWV0YWRhdGEtY29tbWl0LWN sZWFudXA="
                .replace(' ', "")
                .as_str(),
        );
        let account = account_test_store("metadata-cleanup", snapshot.clone());
        account.store.finish_note_privacy_cleanup().unwrap();
        let connection = rusqlite::Connection::open(account.store.database_path()).unwrap();
        assert_eq!(private_commit_pending(&connection), 0);
        if existing_pending {
            connection
                .execute(
                    "UPDATE account_note_privacy SET cleanup_pending=1 WHERE user_id='user-1'",
                    [],
                )
                .unwrap();
        }
        let token_id = match account
            .store
            .authenticate_token("metadata-cleanup", now_millis())
            .unwrap()
        {
            TokenAuthentication::Active(value) => value.token_id,
            other => panic!("{other:?}"),
        };
        let before: String = connection
            .query_row(
                "SELECT app_data_json FROM account_snapshots WHERE user_id='user-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let result = account
            .store
            .exchange_private_media_references(
                "user-1",
                token_id,
                &private_media_request(query.clone()),
                now_millis(),
            )
            .unwrap();
        assert!(result.entries[0].accepted);
        assert_eq!(
            private_commit_pending(&connection),
            i64::from(existing_pending),
            "a metadata-only commit must preserve physical cleanup state"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT app_data_json FROM account_snapshots WHERE user_id='user-1'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            before
        );
        let reopened = SqliteServerStore::open(account.store.database_path(), None).unwrap();
        let mut read = private_media_request(query);
        read.queries[0].declaration = None;
        let result = reopened
            .exchange_private_media_references("user-1", token_id, &read, now_millis())
            .unwrap();
        assert!(result.entries[0].declaration.is_some());
        assert_eq!(
            private_commit_pending(&connection),
            i64::from(existing_pending)
        );
    }
}

#[test]
fn private_commit_metadata_still_completes_proven_media_deletion() {
    let (raw, query) = private_media_fixture("bWV0YWRhdGEtY29tbWl0LXB1cmdl");
    let account = account_test_store("metadata-purge", raw.clone());
    let bytes = b"obsolete independent attachment";
    account
        .store
        .upsert_media(
            "user-1",
            "obsolete-file",
            &hex_bytes(&Sha256::digest(bytes)),
            "application/octet-stream",
            bytes.len() as i64,
            bytes,
            100,
        )
        .unwrap();
    let mut changed: Value = serde_json::from_str(&raw).unwrap();
    changed["tombstones"] =
        json!([{"entityType":"noteMedia","entityId":"obsolete-file","deletedAtEpochMillis":300}]);
    account
        .store
        .compare_and_swap_account("user-1", 0, &changed.to_string(), 300)
        .unwrap();
    account.store.finish_note_privacy_cleanup().unwrap();
    let connection = rusqlite::Connection::open(account.store.database_path()).unwrap();
    assert!(
        account
            .store
            .read_media("user-1", "obsolete-file")
            .unwrap()
            .is_some(),
        "unknown references must retain the old file"
    );
    assert_eq!(private_commit_pending(&connection), 0);
    let token_id = match account
        .store
        .authenticate_token("metadata-purge", now_millis())
        .unwrap()
    {
        TokenAuthentication::Active(value) => value.token_id,
        other => panic!("{other:?}"),
    };
    let result = account
        .store
        .exchange_private_media_references(
            "user-1",
            token_id,
            &private_media_request(query),
            now_millis(),
        )
        .unwrap();
    assert!(result.entries[0].accepted);
    assert!(account
        .store
        .read_media("user-1", "obsolete-file")
        .unwrap()
        .is_none());
    let policy: String = connection
        .query_row(
            "SELECT policy_json FROM account_note_privacy WHERE user_id='user-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&policy).unwrap()["media_deletions"]["obsolete-file"],
        json!(300)
    );
    assert_eq!(
        private_commit_pending(&connection),
        1,
        "removing real bytes still requires physical cleanup"
    );
}

#[test]
#[ignore = "large local recovery index profile"]
fn private_compact_profile_history_above_32_mib() {
    use crate::desktop_state_store::DesktopStateStore;
    let current = app_data::default_app_data_json(500);
    let account = account_test_store("compact-profile", current.clone());
    let journal = DesktopStateStore::open(account.directory.join("compact-journal.db")).unwrap();
    let (template, _) = private_media_fixture("Y29tcGFjdC1wcm9maWxl");
    let mut total = 0usize;
    let mut known = crate::desktop_state_store::DesktopPrivacyPolicy::default();
    for ordinal in 0..3 {
        let mut snapshot: Value = serde_json::from_str(&template).unwrap();
        snapshot["notes"][0]["id"] = json!(format!("large-retained-{ordinal}"));
        snapshot["notes"][0]["encryption"]["ciphertextBase64"] =
            json!("QUJD".repeat(11 * 1024 * 1024 / 4));
        let note = &snapshot["notes"][0];
        let declaration = crate::sealed_media_references::SealedMediaReferences::from_payload(
            note["id"].as_str().unwrap(),
            &note["encryption"],
            &json!([]),
            &json!({}),
            &json!([]),
            &json!([]),
        )
        .unwrap();
        known = known.including_sealed_media(note, declaration).unwrap();
        let raw = snapshot.to_string();
        total += raw.len();
        journal
            .record("owner", &raw, 100 + ordinal, "local_save")
            .unwrap();
    }
    journal
        .record("owner", &current, 500, "local_save")
        .unwrap();
    assert!(total > 32 * 1024 * 1024);
    let evidence = journal.journal_evidence().unwrap();
    let started = Instant::now();
    let result = journal.private_media_references("owner", &current);
    println!(
        "COMPACT_PROFILE bytes={total} scanMillis={} success={}",
        started.elapsed().as_secs_f64() * 1000.0,
        result.is_ok()
    );
    let view = result.expect("valid retained history above 32 MiB must remain synchronizable");
    let queries = view.queries(&known, true).unwrap();
    println!(
        "COMPACT_INDEX queryBytes={} retainedDescriptionBytes={}",
        serde_json::to_vec(&queries).unwrap().len(),
        format!("{view:?}").len()
    );
    assert!(queries.iter().all(|query| query.declaration.is_some()));
    assert!(
        view.scope(&known).is_complete(),
        "known retained references must be fully reconcilable"
    );
    assert_eq!(queries.len(), 3);
    assert_eq!(journal.journal_evidence().unwrap(), evidence);
    assert_eq!(
        journal
            .latest_valid("owner", 0)
            .unwrap()
            .unwrap()
            .app_data_json,
        current
    );
}

#[test]
fn private_compact_index_preserves_unknown_records_and_exact_bindings() {
    use crate::desktop_private_media_index::DesktopPrivateMediaReferences;
    use crate::desktop_state_store::DesktopPrivacyPolicy;
    let (snapshot, query) = private_media_fixture("Y29tcGFjdC1iaW5kaW5n");
    let declaration = query.parsed_declaration().unwrap().unwrap();
    let index = DesktopPrivateMediaReferences::from_snapshot(&snapshot).unwrap();
    let policy = DesktopPrivacyPolicy::default();
    assert!(!index.scope(&policy).is_complete());
    assert!(policy
        .including_indexed_sealed_media(&index, "another-note", declaration.clone())
        .is_err());
    let empty =
        DesktopPrivateMediaReferences::from_snapshot(&app_data::default_app_data_json(0)).unwrap();
    assert!(policy
        .including_indexed_sealed_media(&empty, &query.note_id, declaration.clone())
        .is_err());
    let policy = policy
        .including_indexed_sealed_media(&index, &query.note_id, declaration)
        .unwrap();
    let expected = crate::desktop_media_references::DesktopMediaReferenceScope::from_snapshot(
        &snapshot, &policy,
    );
    assert_eq!(index.scope(&policy).ids(), expected.ids());
    assert_eq!(index.scope(&policy).is_complete(), expected.is_complete());
    let mut future: Value = serde_json::from_str(&snapshot).unwrap();
    future["notes"][0]["futureAttachmentContainer"] = json!(["unknown-file"]);
    let future = DesktopPrivateMediaReferences::from_snapshot(&future.to_string()).unwrap();
    assert!(future.queries(&policy, true).unwrap().is_empty());
    assert!(!future.scope(&policy).is_complete());
    let mut current: Value = serde_json::from_str(&snapshot).unwrap();
    current["syncConflictHistory"] = json!([{"id":"wrong-conflict","entityType":"note","entityId":"another-note","payload":current["notes"][0]}]);
    assert!(DesktopPrivateMediaReferences::from_snapshot(&current.to_string()).is_err());
}

#[test]
fn private_compact_index_rejects_changed_current_before_media_transfer() {
    use crate::desktop_note_media::DesktopNoteMediaStore;
    use crate::desktop_note_media_sync::{
        sync_desktop_note_media_with_reference_index, DesktopNoteMediaSyncDirection,
    };
    use crate::desktop_private_media_index::DesktopPrivateMediaReferences;
    let (old, policy, _, _) = encrypted_byte_fixture();
    let current = app_data::default_app_data_json(300);
    let account = account_test_store("compact-current", old.clone());
    let identity = account.store.server_account_identity("user-1").unwrap();
    let references = DesktopPrivateMediaReferences::from_snapshot(&old).unwrap();
    let server = EncryptedByteServer::start(&account.store);
    let media =
        DesktopNoteMediaStore::new(account.directory.join("compact-current-media")).unwrap();
    let result = sync_desktop_note_media_with_reference_index(
        &media,
        &current,
        &server.url,
        "compact-current",
        "user-1",
        &identity.server_instance_id,
        &identity.account_namespace,
        0,
        "",
        DesktopNoteMediaSyncDirection::Bidirectional,
        Some(&policy),
        Some(&references),
    );
    assert_eq!(result.failures, 1);
    assert_eq!(
        result.uploaded + result.downloaded + result.verified_private_media.len(),
        0,
        "an index from another current snapshot must not authorize a network exchange"
    );
}
