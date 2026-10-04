// v0.0.1 - Verify metadata rollback, exact-envelope binding and metadata-only durability.
#[test]
fn private_media_session_declaration_rejects_cross_owner_same_ciphertext_without_mutation() {
    let (directory, store) = temp_store("private_declaration_owner");
    let server = "a".repeat(64);
    let account = |user| {
        desktop_state_owner(
            &server,
            &sync_core::account_namespace_identifier(&server, user),
            user,
        )
        .unwrap()
    };
    let owner_a = account("source-account");
    let owner_b = account("other-account");
    let (_, sealed, declaration) = private_media_core_fixture(&store, &owner_a);
    store
        .record_with_sync_state_and_media_declaration(
            &owner_a,
            &sealed,
            b"sync-a",
            200,
            "local_save",
            Some(&declaration),
        )
        .unwrap();
    for owner in [owner_b.as_str(), "guest-v1"] {
        // A copied, valid ciphertext in the target journal must not establish
        // authority to copy the source account's private attachment knowledge.
        store
            .record_with_sync_state(owner, &sealed, b"sync-before", 200, "local_save")
            .unwrap();
        let policy = store.privacy_policy(owner).unwrap();
        let evidence = store.journal_evidence().unwrap();
        let before = store.latest_valid(owner, 0).unwrap().unwrap();
        assert!(
            store
                .record_with_sync_state_and_media_declaration(
                    owner,
                    &sealed,
                    b"sync-must-not-commit",
                    300,
                    "local_save",
                    Some(&declaration),
                )
                .is_err(),
            "same ciphertext allowed a declaration from another owner"
        );
        assert_eq!(store.privacy_policy(owner).unwrap(), policy);
        assert_eq!(store.journal_evidence().unwrap(), evidence);
        let after = store.latest_valid(owner, 0).unwrap().unwrap();
        assert_eq!(after.id, before.id);
        assert_eq!(after.protected_sync_state, before.protected_sync_state);
    }
    drop(store);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn private_media_session_declaration_rejects_same_owner_in_other_workspace() {
    let (directory_a, store_a) = temp_store("private_declaration_workspace_a");
    let (directory_b, store_b) = temp_store("private_declaration_workspace_b");
    let server = "b".repeat(64);
    let account = desktop_state_owner(
        &server,
        &sync_core::account_namespace_identifier(&server, "same-account"),
        "same-account",
    )
    .unwrap();
    for owner in [account.as_str(), "guest-v1"] {
        let root = store_a.database_path().parent().unwrap();
        assert_eq!(
            desktop_note_session_scope(root, owner).unwrap(),
            desktop_note_session_scope(&root.join("."), owner).unwrap()
        );
        let (_, sealed, declaration) = private_media_core_fixture(&store_a, owner);
        store_a
            .record_with_sync_state_and_media_declaration(
                owner,
                &sealed,
                b"sync-a",
                200,
                "local_save",
                Some(&declaration),
            )
            .unwrap();
        store_b
            .record_with_sync_state(owner, &sealed, b"sync-b", 200, "local_save")
            .unwrap();
        let before = store_b.latest_valid(owner, 0).unwrap().unwrap();
        let evidence = store_b.journal_evidence().unwrap();
        let policy = store_b.privacy_policy(owner).unwrap();
        assert!(
            store_b
                .record_with_sync_state_and_media_declaration(
                    owner,
                    &sealed,
                    b"must-not-commit",
                    300,
                    "local_save",
                    Some(&declaration),
                )
                .is_err(),
            "same owner borrowed a different workspace's live session"
        );
        assert_eq!(store_b.privacy_policy(owner).unwrap(), policy);
        assert_eq!(store_b.journal_evidence().unwrap(), evidence);
        assert_eq!(
            store_b.latest_valid(owner, 0).unwrap().unwrap().id,
            before.id
        );
    }
    drop(store_a);
    drop(store_b);
    fs::remove_dir_all(directory_a).unwrap();
    fs::remove_dir_all(directory_b).unwrap();
}

#[test]
fn private_media_unscoped_session_cannot_mint_durable_declaration() {
    let plain: Value = serde_json::from_str(&media_retention_test_state()).unwrap();
    let (sealed, token) = crate::encrypt_desktop_note_json(
        &plain["notes"][0].to_string(),
        "unscoped-compatibility-password",
    )
    .unwrap();
    assert!(DesktopSealedMediaDeclaration::from_session(&sealed, &token).is_none());
    assert!(crate::close_desktop_note_session(&token));
}

fn private_media_core_fixture(
    store: &DesktopStateStore,
    owner: &str,
) -> (String, String, DesktopSealedMediaDeclaration) {
    let plain = media_retention_test_state();
    let value: Value = serde_json::from_str(&plain).unwrap();
    let scope = desktop_note_session_scope(store.database_path().parent().unwrap(), owner).unwrap();
    let (sealed_note, token) = crate::encrypt_desktop_note_json_in_scope(
        &value["notes"][0].to_string(),
        "atomic-reference-test",
        &scope,
    )
    .unwrap();
    let declaration = DesktopSealedMediaDeclaration::from_session(&sealed_note, &token).unwrap();
    let sealed = app_data::upsert_note_app_data_json(&plain, &sealed_note, 200).unwrap();
    assert!(crate::close_desktop_note_session(&token));
    assert!(DesktopSealedMediaDeclaration::from_session(&sealed_note, &token).is_none());
    (plain, sealed, declaration)
}

#[test]
fn private_media_session_snapshot_failure_rolls_back_declaration_and_privacy() {
    let (directory, store) = temp_store("private_media_atomic");
    let (plain, sealed, declaration) = private_media_core_fixture(&store, "owner");
    store.record("owner", &plain, 100, "local_save").unwrap();
    let before_policy = store.privacy_policy("owner").unwrap();
    let before_evidence = store.journal_evidence().unwrap();
    let connection = Connection::open(store.database_path()).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_media_snapshot BEFORE INSERT ON desktop_state_snapshots BEGIN SELECT RAISE(ABORT,'injected snapshot failure'); END;").unwrap();
    assert!(store
        .record_with_sync_state_and_media_declaration(
            "owner",
            &sealed,
            b"sealed-sync",
            200,
            "local_save",
            Some(&declaration)
        )
        .is_err());
    assert_eq!(store.privacy_policy("owner").unwrap(), before_policy);
    assert_eq!(store.journal_evidence().unwrap(), before_evidence);
    assert_eq!(
        store
            .latest_valid("owner", 0)
            .unwrap()
            .unwrap()
            .app_data_json,
        plain
    );
    connection
        .execute_batch("DROP TRIGGER reject_media_snapshot;")
        .unwrap();
    let committed = store
        .record_with_sync_state_and_media_declaration(
            "owner",
            &sealed,
            b"sealed-sync",
            200,
            "local_save",
            Some(&declaration),
        )
        .unwrap();
    assert_eq!(committed.protected_sync_state, b"sealed-sync");
    let policy = store.privacy_policy("owner").unwrap().0;
    let refs = crate::desktop_media_references::DesktopMediaReferenceScope::from_snapshot(
        &committed.app_data_json,
        &policy,
    );
    assert!(refs.is_complete());
    assert!(refs.ids().contains("retained-file"));
    assert!(!committed.app_data_json.contains("bindingSha256"));
    let mut wrong: Value = serde_json::from_str(&sealed).unwrap();
    wrong["notes"][0]["id"] = Value::String("different-note".into());
    let evidence = store.journal_evidence().unwrap();
    assert!(store
        .record_with_sync_state_and_media_declaration(
            "owner",
            &wrong.to_string(),
            b"new-sync",
            300,
            "local_save",
            Some(&declaration)
        )
        .is_err());
    assert_eq!(store.journal_evidence().unwrap(), evidence);
    drop(connection);
    drop(store);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn private_media_session_metadata_only_commit_survives_reopen_without_cleanup_work() {
    let (directory, store) = temp_store("private_media_metadata_commit");
    let (_, sealed, declaration) = private_media_core_fixture(&store, "owner");
    let first = store
        .record_with_sync_state("owner", &sealed, b"sync", 200, "local_save")
        .unwrap();
    store.finish_privacy_cleanup("owner").unwrap();
    assert!(!store.privacy_policy("owner").unwrap().1);
    let before = store.journal_evidence().unwrap();
    let before_policy = store.privacy_policy("owner").unwrap();
    let connection = Connection::open(store.database_path()).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_media_metadata BEFORE UPDATE OF commit_sequence ON desktop_state_journal_metadata BEGIN SELECT RAISE(ABORT,'injected metadata evidence failure'); END;").unwrap();
    assert!(store
        .record_with_sync_state_and_media_declaration(
            "owner",
            &sealed,
            b"sync",
            250,
            "local_save",
            Some(&declaration)
        )
        .is_err());
    assert_eq!(store.journal_evidence().unwrap(), before);
    assert_eq!(store.privacy_policy("owner").unwrap(), before_policy);
    connection
        .execute_batch("DROP TRIGGER reject_media_metadata;")
        .unwrap();
    drop(connection);
    let second = store
        .record_with_sync_state_and_media_declaration(
            "owner",
            &sealed,
            b"sync",
            300,
            "local_save",
            Some(&declaration),
        )
        .unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(
        store.journal_evidence().unwrap().commit_sequence,
        before.commit_sequence + 1
    );
    assert!(
        !store.privacy_policy("owner").unwrap().1,
        "reference metadata must not schedule a plaintext vacuum"
    );
    let path = store.database_path().to_path_buf();
    drop(store);
    let reopened = DesktopStateStore::open(path).unwrap();
    let policy = reopened.privacy_policy("owner").unwrap().0;
    let scope = crate::desktop_media_references::DesktopMediaReferenceScope::from_snapshot(
        &sealed, &policy,
    );
    assert!(scope.is_complete());
    assert!(scope.ids().contains("retained-file"));
    assert!(
        !crate::desktop_media_references::DesktopMediaReferenceScope::from_snapshot(
            &sealed,
            &reopened.privacy_policy("other-owner").unwrap().0
        )
        .is_complete()
    );
    let evidence = reopened.journal_evidence().unwrap();
    reopened
        .record_with_sync_state_and_media_declaration(
            "owner",
            &sealed,
            b"sync",
            400,
            "local_save",
            Some(&declaration),
        )
        .unwrap();
    assert_eq!(
        reopened.journal_evidence().unwrap(),
        evidence,
        "repeating a declaration must not advance the journal"
    );
    let changed =
        app_data::update_slot_title_app_data_json(&sealed, 1, "after metadata", 500).unwrap();
    let appended = reopened
        .record("owner", &changed, 500, "local_save")
        .unwrap();
    assert_eq!(appended.id, evidence.commit_sequence + 1);
    assert_eq!(
        reopened.journal_evidence().unwrap().commit_sequence,
        appended.id
    );
    drop(reopened);
    fs::remove_dir_all(directory).unwrap();
}
