// v0.0.0.1 - Keep rejected saves atomic and preserve owner-scoped detach floors.
fn attachment_intent_workspace() -> String {
    let mut raw = app_data::default_app_data_json(100);
    for id in ["page-a", "page-b"] {
        raw = app_data::upsert_note_app_data_json(&raw, &serde_json::json!({
            "id":id,"title":id,"content":"Attachment intent fixture",
            "updatedAtEpochMillis":100,
            "attachments":[{"id":"shared-image","mimeType":"image/png",
                "sha256":"a".repeat(64),"sizeBytes":4,"updatedAtEpochMillis":100}],
            "document":{"blocks":[{"id":format!("{id}-image"),"type":"IMAGE","attachmentId":"shared-image"}]}
        }).to_string(),100).unwrap();
    }
    raw
}

#[test]
fn desktop_rejected_item_drop_does_not_commit_a_new_privacy_fence() {
    let (directory, store) = temp_store("rejected_privacy_intent");
    let populated = app_data::upsert_note_app_data_json(&state_with_sessions(30,100),
        r#"{"id":"private","title":"Private","content":"Atomic privacy fixture","updatedAtEpochMillis":100}"#,100).unwrap();
    let head = store
        .record("owner", &populated, 100, "local_save")
        .unwrap();
    let before = store.journal_evidence().unwrap();
    let policy = store.privacy_policy("owner").unwrap();
    let mut invalid: Value = serde_json::from_str(&populated).unwrap();
    let scope = desktop_note_session_scope(&directory, "owner").unwrap();
    let (sealed, token) = crate::encrypt_desktop_note_json_in_scope(
        &invalid["notes"][0].to_string(),
        "atomic-drop-fixture",
        &scope,
    )
    .unwrap();
    invalid["notes"][0] = serde_json::from_str(&sealed).unwrap();
    invalid["sessions"] = serde_json::json!([]);
    assert!(crate::close_desktop_note_session(&token));
    let result = store.record("owner", &invalid.to_string(), 300, "local_save");
    assert!(
        matches!(
            result,
            Err(DesktopStateStoreError::SuspiciousItemDrop { .. })
        ),
        "{result:?}"
    );
    assert_eq!(policy, store.privacy_policy("owner").unwrap());
    assert_eq!(before, store.journal_evidence().unwrap());
    assert_eq!(head, store.latest_valid("owner", 0).unwrap().unwrap());
    drop(store);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn desktop_attachment_detach_floor_survives_history_prune_and_does_not_cross_owner() {
    let (directory, store) = temp_store("durable_attachment_detach");
    let shared = attachment_intent_workspace();
    store.record("owner-a", &shared, 100, "local_save").unwrap();
    store.record("owner-b", &shared, 100, "local_save").unwrap();
    let detached =
        app_data::delete_note_attachment_app_data_json(&shared, "page-a", "shared-image", 300)
            .unwrap();
    store
        .record("owner-a", &detached, 300, "local_save")
        .unwrap();
    let mut pruned: Value = serde_json::from_str(&detached).unwrap();
    for note in pruned["notes"].as_array_mut().unwrap() {
        note["revisions"] = serde_json::json!([]);
        note["versions"] = serde_json::json!([]);
    }
    let head = store
        .record("owner-a", &pruned.to_string(), 400, "verified_restore")
        .unwrap();
    let connection = store.open_connection(false).unwrap();
    connection
        .execute(
            "DELETE FROM desktop_state_snapshots WHERE owner=?1 AND id<>?2",
            params!["owner-a", head.id],
        )
        .unwrap();
    drop(connection);
    drop(store);
    let store = DesktopStateStore::open(directory.join("state_journal.sqlite3")).unwrap();
    let policy = store.privacy_policy("owner-a").unwrap().0;
    assert!(!policy.note_attachment_detachments().is_empty());
    let mut replay: Value = serde_json::from_str(&shared).unwrap();
    replay["notes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|note| note["id"] == "page-a")
        .unwrap()["updatedAtEpochMillis"] = serde_json::json!(900);
    let projected: Value = serde_json::from_str(
        &policy
            .project_current_json(&replay.to_string())
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let detached_page = projected["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|note| note["id"] == "page-a")
        .unwrap();
    let shared_page = projected["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|note| note["id"] == "page-b")
        .unwrap();
    assert!(detached_page["attachments"].as_array().unwrap().is_empty());
    assert_eq!(shared_page["attachments"].as_array().unwrap().len(), 1);
    assert!(store
        .privacy_policy("owner-b")
        .unwrap()
        .0
        .project_current_json(&shared)
        .unwrap()
        .is_none());
    assert!(store
        .record("owner-a", &replay.to_string(), 900, "local_save")
        .is_err());
    replay["notes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|note| note["id"] == "page-a")
        .unwrap()["attachments"][0]["updatedAtEpochMillis"] = serde_json::json!(901);
    assert!(policy
        .project_current_json(&replay.to_string())
        .unwrap()
        .is_none());
    assert_eq!(
        policy,
        policy
            .merged_policy(&DesktopPrivacyPolicy::default())
            .unwrap()
    );
    assert!(
        policy.redact_json(&shared).unwrap().is_none(),
        "recoverable historical bytes lost their references"
    );
    drop(store);
    fs::remove_dir_all(directory).unwrap();
}
